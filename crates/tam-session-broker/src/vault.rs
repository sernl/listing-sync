//! The vault: the broker-role database access plus the seal/open envelope.
//! This is the only code in the system that holds ciphertext and the
//! key-encryption key in the same process.

use sqlx::PgPool;
use tam_secrets::{account_digest, open, seal, AadContext, Kek, Secret};
use tam_types::{ConnectionEvent, ConnectionId, Marketplace, OrgId};

pub(crate) const KEY_VERSION: i32 = 1;

/// The index whose violation means another organisation already holds this
/// marketplace account. Matched by name rather than by parsing a message,
/// because the name is the schema's own contract and the message is not.
const EXCLUSIVITY_INDEX: &str = "connection_platform_account_exclusive";

/// Whether a marketplace can name the account a link speaks for.
///
/// The account reference is what makes global exclusivity enforceable, so a
/// marketplace that cannot produce one links without a lock rather than with a
/// guessed lock. The variant is a value rather than an `Option` at each call
/// site so the reason travels with the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdentitySource {
    /// An identity read after linking supplies the account reference, which
    /// `Vault::claim` turns into the digest and the lock.
    Claimed,
    /// No identity read exists for this marketplace yet, so the link completes
    /// immediately and holds no exclusivity lock.
    Unavailable,
}

/// Tpt's seller identity is `author { id }` on the `Store` type, present in
/// every committed cassette of `UploadPageProductQuery` and `MyProductListings`.
///
/// Tes parses no author identity today. Every Tes response the connector reads
/// is a resource or a dashboard listing, none of which names the account that
/// owns it, so there is nothing to claim; the pending capture is the body of
/// `GET /api/tier/gmv/me`, which the longevity probe already reaches and which
/// no cassette records. Until that capture exists this arm is deliberately
/// `Unavailable` rather than a guess, and a Tes link therefore takes no
/// exclusivity lock. Etsy has no connector at all.
pub(crate) const fn identity_source(marketplace: Marketplace) -> IdentitySource {
    match marketplace {
        Marketplace::Tpt => IdentitySource::Claimed,
        Marketplace::Tes | Marketplace::Etsy => IdentitySource::Unavailable,
    }
}

/// The seller's declaration of who authored the listings a connection
/// publishes, supplied with the link rather than minted here.
///
/// `attested_at_ms` is the instant the seller attested. It enters as data and
/// is never read from this process's clock, because a server-minted instant
/// would be this system attesting on the seller's behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Authorship {
    pub(crate) name: String,
    pub(crate) attested_at_ms: i64,
}

/// One link, whole. A struct because the five inputs always travel together
/// and `too-many-arguments-threshold` is five; the grouping is the tenant
/// context plus the two things the seller supplied.
pub(crate) struct LinkRequest<'a> {
    pub(crate) org: OrgId,
    pub(crate) connection: ConnectionId,
    pub(crate) marketplace: Marketplace,
    pub(crate) secret: Secret,
    pub(crate) authorship: Option<&'a Authorship>,
}

pub(crate) struct Vault {
    pool: PgPool,
    kek: Kek,
}

#[derive(Debug)]
pub(crate) enum VaultError {
    Db(sqlx::Error),
    Seal,
    Open(String),
    NoSecret,
    /// Another organisation already holds this marketplace account. Names the
    /// marketplace and nothing else: telling the caller who holds it would
    /// turn the constraint into a directory of every seller on the platform.
    AccountAlreadyLinked(Marketplace),
    /// The connection was not in a state a claim may complete.
    NotClaimable,
}

impl core::fmt::Display for VaultError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Db(error) => write!(f, "vault database error: {error}"),
            Self::Seal => f.write_str("sealing failed"),
            Self::Open(detail) => write!(f, "opening failed: {detail}"),
            Self::NoSecret => f.write_str("no stored credential for this connection"),
            Self::AccountAlreadyLinked(marketplace) => write!(
                f,
                "that {} account is already linked to another organisation",
                marketplace_to_db(*marketplace)
            ),
            Self::NotClaimable => {
                f.write_str("the connection is not awaiting a claim, so none was applied")
            }
        }
    }
}

impl core::error::Error for VaultError {}

impl From<sqlx::Error> for VaultError {
    fn from(error: sqlx::Error) -> Self {
        Self::Db(error)
    }
}

/// Whether this error is the exclusivity index refusing a second holder.
fn is_exclusivity_violation(error: &sqlx::Error) -> bool {
    let sqlx::Error::Database(database) = error else {
        return false;
    };
    database.constraint() == Some(EXCLUSIVITY_INDEX)
}

fn marketplace_to_db(marketplace: Marketplace) -> &'static str {
    match marketplace {
        Marketplace::Tes => "tes",
        Marketplace::Etsy => "etsy",
        Marketplace::Tpt => "tpt",
    }
}

fn uuid(id: tam_types::Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

/// Appends one connection-lifecycle row, in the caller's transaction so the
/// record and the change it describes land together.
///
/// Written here with the broker's own role rather than through
/// `tam-storage`: this crate does not depend on that one, deliberately, so
/// that the vault's queries stay on this side of the privilege boundary. The
/// event vocabulary is shared through `tam-types` instead, and the database's
/// CHECK constraint holds the same closed set from the other side.
///
/// Every row the broker writes is attributed to the broker. A link or a
/// revoke is seller-initiated, but the broker's wire protocol carries only
/// the organisation and the connection, so the person who asked is not
/// recoverable in this process; naming the broker is the honest answer and
/// inventing a user would not be.
async fn audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    connection: ConnectionId,
    event: ConnectionEvent,
    detail: Option<&str>,
) -> Result<(), VaultError> {
    sqlx::query(
        "INSERT INTO connection_audit \
         (org_id, connection_id, event, actor_kind, actor_id, detail, at) \
         VALUES ($1, $2, $3, 'system', 'broker', $4, now())",
    )
    .bind(uuid(org.0))
    .bind(uuid(connection.0))
    .bind(event.as_str())
    .bind(detail)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

impl Vault {
    #[must_use]
    pub(crate) fn new(pool: PgPool, kek: Kek) -> Self {
        Self { pool, kek }
    }

    /// Seals the credential and opens the link, both in one transaction. The
    /// secret is moved in and never leaves.
    ///
    /// The state the link lands in is the identity obligation made visible.
    /// A marketplace whose account this system can name lands in `linking` and
    /// waits for [`Vault::claim`]; one it cannot name lands in `linked`
    /// immediately, because there is no obligation to discharge. A crash
    /// between the two phases leaves a `linking` row with no digest, which
    /// holds no exclusivity lock and which the lease scan — requiring
    /// `linked` — will not drive. Half-linked is inert, not half-usable.
    pub(crate) async fn link(&self, request: LinkRequest<'_>) -> Result<(), VaultError> {
        let LinkRequest {
            org,
            connection,
            marketplace,
            secret,
            authorship,
        } = request;
        let context = AadContext {
            org,
            marketplace,
            connection,
            key_version: KEY_VERSION,
        };
        let sealed = seal(&self.kek, &context, secret).map_err(|_| VaultError::Seal)?;
        let opening_state = match identity_source(marketplace) {
            IdentitySource::Claimed => "linking",
            IdentitySource::Unavailable => "linked",
        };

        let mut tx = self.pool.begin().await?;
        // Which of the two events this is, read before the upsert rather than
        // inferred from it: a plain existence check inside this transaction
        // is exact, where reading it back out of an upsert is not.
        let relinking: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM connection_secret \
             WHERE org_id = $1 AND connection_id = $2)",
        )
        .bind(uuid(org.0))
        .bind(uuid(connection.0))
        .fetch_one(&mut *tx)
        .await?;
        // The connection row must exist before the secret row: the secret's
        // foreign key points at it, so a fresh connection linked in the
        // other order dies on the constraint before the upsert can run.
        sqlx::query(
            "INSERT INTO connection \
             (org_id, id, marketplace, state, created_at, updated_at, \
              session_verified_at, refresh_failures, authorship_name, authorship_attested_at) \
             VALUES ($1, $2, $3, $4, now(), now(), now(), 0, $5, \
                     to_timestamp($6::bigint / 1000.0)) \
             ON CONFLICT (org_id, id) DO UPDATE SET \
                 state = EXCLUDED.state, updated_at = now(), \
                 session_verified_at = now(), refresh_failures = 0, \
                 authorship_name = COALESCE(EXCLUDED.authorship_name, connection.authorship_name), \
                 authorship_attested_at = COALESCE(EXCLUDED.authorship_attested_at, \
                                                   connection.authorship_attested_at)",
        )
        .bind(uuid(org.0))
        .bind(uuid(connection.0))
        .bind(marketplace_to_db(marketplace))
        .bind(opening_state)
        .bind(authorship.map(|declared| declared.name.as_str()))
        .bind(authorship.map(|declared| declared.attested_at_ms))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO connection_secret \
             (org_id, connection_id, key_version, wrapped_dek, nonce, ciphertext, aad, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, now()) \
             ON CONFLICT (org_id, connection_id, key_version) DO UPDATE \
             SET wrapped_dek = EXCLUDED.wrapped_dek, nonce = EXCLUDED.nonce, \
                 ciphertext = EXCLUDED.ciphertext",
        )
        .bind(uuid(org.0))
        .bind(uuid(connection.0))
        .bind(sealed.key_version)
        .bind(&sealed.wrapped_dek)
        .bind(&sealed.nonce)
        .bind(&sealed.ciphertext)
        .bind(context_bytes(&context))
        .execute(&mut *tx)
        .await?;
        audit(
            &mut tx,
            org,
            connection,
            if relinking {
                ConnectionEvent::Relinked
            } else {
                ConnectionEvent::Linked
            },
            Some(marketplace_to_db(marketplace)),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The second phase: names the account this connection speaks for and
    /// completes the link.
    ///
    /// The broker never parses a marketplace response — the caller runs the
    /// identity read through its lease and passes the account reference it
    /// found, which keeps response parsing outside the one process holding the
    /// key-encryption key. Only a server-asserted identity may be claimed: a
    /// seller-typed value would be a denial-of-service primitive against the
    /// real owner of that storefront, who could then never link.
    ///
    /// Writing the digest is what takes the lock, so a second organisation
    /// claiming the same account is refused here by the index rather than by a
    /// read-then-write race this code would have to win.
    pub(crate) async fn claim(
        &self,
        org: OrgId,
        connection: ConnectionId,
        marketplace: Marketplace,
        account_ref: &str,
    ) -> Result<(), VaultError> {
        let digest = account_digest(&self.kek, marketplace, account_ref);
        let mut tx = self.pool.begin().await?;
        let claimed = sqlx::query(
            "UPDATE connection \
             SET platform_account_digest = $3, platform_account_seen_at = now(), \
                 state = 'linked', updated_at = now() \
             WHERE org_id = $1 AND id = $2 \
               AND state IN ('linking', 'linked', 'needs_reauth')",
        )
        .bind(uuid(org.0))
        .bind(uuid(connection.0))
        .bind(digest.as_slice())
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            if is_exclusivity_violation(&error) {
                VaultError::AccountAlreadyLinked(marketplace)
            } else {
                VaultError::Db(error)
            }
        })?;
        if claimed.rows_affected() == 0 {
            return Err(VaultError::NotClaimable);
        }
        // The account reference itself is never recorded: the digest exists
        // precisely so a compromised database yields no storefront
        // identifiers, and an audit row naming one would undo that.
        audit(
            &mut tx,
            org,
            connection,
            ConnectionEvent::Claimed,
            Some(marketplace_to_db(marketplace)),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Opens the stored credential for the gateway to inject. The plaintext
    /// lives only for the lease's lifetime, inside this process.
    pub(crate) async fn open_secret(
        &self,
        org: OrgId,
        connection: ConnectionId,
        marketplace: Marketplace,
    ) -> Result<Secret, VaultError> {
        let row = sqlx::query_as::<_, (i32, Vec<u8>, Vec<u8>, Vec<u8>)>(
            "SELECT key_version, wrapped_dek, nonce, ciphertext FROM connection_secret \
             WHERE org_id = $1 AND connection_id = $2 ORDER BY key_version DESC LIMIT 1",
        )
        .bind(uuid(org.0))
        .bind(uuid(connection.0))
        .fetch_optional(&self.pool)
        .await?
        .ok_or(VaultError::NoSecret)?;
        let sealed = tam_secrets::Sealed {
            key_version: row.0,
            wrapped_dek: row.1,
            nonce: row.2,
            ciphertext: row.3,
        };
        let context = AadContext {
            org,
            marketplace,
            connection,
            key_version: row.0,
        };
        open(&self.kek, &context, &sealed).map_err(|error| VaultError::Open(error.to_string()))
    }

    /// Replaces the stored credential with the renewed one and records that
    /// the session was proved live just now.
    ///
    /// This is the whole of what separates a session that survives from one
    /// that can only age. The upstream renews a session by answering with
    /// `Set-Cookie`; a proxy that forwards the response and drops those
    /// headers hands the renewal to nobody, and the sealed copy expires on the
    /// original cookie's own schedule however often it is used. Resealing here
    /// is the same persistence `curl -b`/`-c` gives the longevity probe, whose
    /// session has stayed authenticated for days against an account whose
    /// sealed copy went stale in one.
    ///
    /// `refresh_failures` returns to zero because a renewal arriving is proof
    /// the session is live, whatever the preceding failures counted.
    pub(crate) async fn reseal(
        &self,
        org: OrgId,
        connection: ConnectionId,
        marketplace: Marketplace,
        secret: Secret,
    ) -> Result<(), VaultError> {
        let context = AadContext {
            org,
            marketplace,
            connection,
            key_version: KEY_VERSION,
        };
        let sealed = seal(&self.kek, &context, secret).map_err(|_| VaultError::Seal)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO connection_secret \
             (org_id, connection_id, key_version, wrapped_dek, nonce, ciphertext, aad, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, now()) \
             ON CONFLICT (org_id, connection_id, key_version) DO UPDATE \
             SET wrapped_dek = EXCLUDED.wrapped_dek, nonce = EXCLUDED.nonce, \
                 ciphertext = EXCLUDED.ciphertext",
        )
        .bind(uuid(org.0))
        .bind(uuid(connection.0))
        .bind(sealed.key_version)
        .bind(&sealed.wrapped_dek)
        .bind(&sealed.nonce)
        .bind(&sealed.ciphertext)
        .bind(context_bytes(&context))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE connection \
             SET session_verified_at = now(), refresh_failures = 0, updated_at = now() \
             WHERE org_id = $1 AND id = $2",
        )
        .bind(uuid(org.0))
        .bind(uuid(connection.0))
        .execute(&mut *tx)
        .await?;
        audit(&mut tx, org, connection, ConnectionEvent::Refreshed, None).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Records that a refresh attempt failed without asserting the session is
    /// dead. Only an auth-class failure may flip `state`, which is a decision
    /// the caller makes and this counter deliberately does not.
    pub(crate) async fn record_refresh_failure(
        &self,
        org: OrgId,
        connection: ConnectionId,
    ) -> Result<(), VaultError> {
        let mut tx = self.pool.begin().await?;
        let streak: Option<i32> = sqlx::query_scalar(
            "UPDATE connection \
             SET refresh_failures = refresh_failures + 1, updated_at = now() \
             WHERE org_id = $1 AND id = $2 \
             RETURNING refresh_failures",
        )
        .bind(uuid(org.0))
        .bind(uuid(connection.0))
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(streak) = streak {
            // The resulting count, not the increment: this is the number the
            // status derivation reads to report `unstable`, so the trail and
            // the page agree about the same value.
            audit(
                &mut tx,
                org,
                connection,
                ConnectionEvent::RefreshFailed,
                Some(&format!("consecutive failures: {streak}")),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Revokes one connection: tombstones its ciphertext and marks it revoked.
    /// No DELETE — the row stays as a tombstone so the revocation is auditable.
    ///
    /// The digest is cleared with the ciphertext. The exclusivity index is
    /// predicated on the live states, so `revoked` alone already releases the
    /// account for another organisation to claim; clearing the digest as well
    /// means a tombstone carries no reference to the storefront it once held.
    pub(crate) async fn revoke(
        &self,
        org: OrgId,
        connection: ConnectionId,
    ) -> Result<u32, VaultError> {
        let mut tx = self.pool.begin().await?;
        let cleared = sqlx::query(
            "UPDATE connection_secret \
             SET wrapped_dek = ''::bytea, ciphertext = ''::bytea \
             WHERE org_id = $1 AND connection_id = $2",
        )
        .bind(uuid(org.0))
        .bind(uuid(connection.0))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE connection \
             SET state = 'revoked', platform_account_digest = NULL, updated_at = now() \
             WHERE org_id = $1 AND id = $2",
        )
        .bind(uuid(org.0))
        .bind(uuid(connection.0))
        .execute(&mut *tx)
        .await?;
        audit(&mut tx, org, connection, ConnectionEvent::Revoked, None).await?;
        tx.commit().await?;
        Ok(u32::try_from(cleared.rows_affected()).unwrap_or(u32::MAX))
    }

    /// The global kill switch: every stored credential cleared, every
    /// connection revoked, fleet-wide.
    pub(crate) async fn revoke_all(&self) -> Result<u32, VaultError> {
        let mut tx = self.pool.begin().await?;
        let cleared = sqlx::query(
            "UPDATE connection_secret SET wrapped_dek = ''::bytea, ciphertext = ''::bytea",
        )
        .execute(&mut *tx)
        .await?;
        // The audit rides the same statement rather than a follow-up loop:
        // the kill switch crosses every tenant, and a second pass could not
        // name the rows this one changed.
        sqlx::query(
            "WITH revoked AS ( \
                 UPDATE connection \
                 SET state = 'revoked', platform_account_digest = NULL, updated_at = now() \
                 RETURNING org_id, id \
             ) \
             INSERT INTO connection_audit \
             (org_id, connection_id, event, actor_kind, actor_id, detail, at) \
             SELECT org_id, id, 'revoked', 'system', 'broker', \
                    'fleet-wide kill switch', now() \
             FROM revoked",
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(u32::try_from(cleared.rows_affected()).unwrap_or(u32::MAX))
    }
}

fn context_bytes(context: &AadContext) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(37);
    bytes.extend_from_slice(&context.org.0 .0);
    bytes.push(match context.marketplace {
        Marketplace::Tes => 0,
        Marketplace::Etsy => 1,
        Marketplace::Tpt => 2,
    });
    bytes.extend_from_slice(&context.connection.0 .0);
    bytes.extend_from_slice(&context.key_version.to_be_bytes());
    bytes
}

#[cfg(test)]
mod identity_tests {
    use super::{identity_source, IdentitySource};
    use tam_types::Marketplace;

    #[test]
    fn only_tpt_can_name_the_account_a_link_speaks_for() {
        assert_eq!(
            identity_source(Marketplace::Tpt),
            IdentitySource::Claimed,
            "Tpt's author.id is in the committed cassettes, so a Tpt link takes a lock"
        );
        assert_eq!(
            identity_source(Marketplace::Tes),
            IdentitySource::Unavailable,
            "Tes parses no author identity yet, so a Tes link must complete without a lock \
             rather than guess at one"
        );
        assert_eq!(
            identity_source(Marketplace::Etsy),
            IdentitySource::Unavailable,
            "Etsy has no connector at all"
        );
    }
}

#[cfg(all(test, feature = "pg-tests"))]
mod tests {
    use super::{Authorship, LinkRequest, Vault, VaultError, KEY_VERSION};
    use sqlx::PgPool;
    use tam_secrets::{Kek, Secret};
    use tam_types::{ConnectionId, Marketplace, OrgId, Uuid};

    /// The account both tenants in the exclusivity tests try to claim; the
    /// store id `UploadPageProductQuery` returns in the committed cassette.
    const SHARED_ACCOUNT: &str = "900000001";

    /// `allow-expect-in-tests` reaches `#[test]` functions, not the free
    /// helpers beside them; a broken fixture should panic.
    #[expect(clippy::expect_used, reason = "a broken test fixture should panic")]
    async fn seed_org(app: &PgPool, org: [u8; 16], name: &str) {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org))
            .bind(name)
            .execute(app)
            .await
            .expect("the org inserts");
    }

    #[expect(clippy::expect_used, reason = "a broken test fixture should panic")]
    async fn broker_pool(app: &PgPool) -> PgPool {
        let database: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(app)
            .await
            .expect("the database name reads");
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&format!(
                "postgres://tam_broker:tam_broker_dev@127.0.0.1:5433/{database}"
            ))
            .await
            .expect("the broker role connects")
    }

    #[expect(clippy::expect_used, reason = "a broken test fixture should panic")]
    async fn link_tpt(vault: &Vault, org: [u8; 16], connection: [u8; 16]) {
        vault
            .link(LinkRequest {
                org: OrgId(Uuid(org)),
                connection: ConnectionId(Uuid(connection)),
                marketplace: Marketplace::Tpt,
                secret: Secret::new("csrfToken=abc".to_owned()),
                authorship: None,
            })
            .await
            .expect("the Tpt link seals");
    }

    #[sqlx::test(migrations = "../tam-storage/migrations")]
    async fn a_fresh_connection_links_without_a_preexisting_row(app: PgPool) {
        let org = [0xA1; 16];
        sqlx::query(
            "INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-fresh', now())",
        )
        .bind(uuid::Uuid::from_bytes(org))
        .execute(&app)
        .await
        .expect("the org inserts");

        let database: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&app)
            .await
            .expect("the database name reads");
        let broker = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&format!(
                "postgres://tam_broker:tam_broker_dev@127.0.0.1:5433/{database}"
            ))
            .await
            .expect("the broker role connects");
        let vault = Vault::new(broker.clone(), Kek::from_bytes(&[0x22; 32]).expect("kek"));
        let connection = [0xB2; 16];
        vault
            .link(LinkRequest {
                org: OrgId(Uuid(org)),
                connection: ConnectionId(Uuid(connection)),
                marketplace: Marketplace::Tes,
                secret: Secret::new("TESSession=fresh".to_owned()),
                authorship: None,
            })
            .await
            .expect("a fresh connection links with no pre-created row");

        let (state,): (String,) =
            sqlx::query_as("SELECT state FROM connection WHERE org_id = $1 AND id = $2")
                .bind(uuid::Uuid::from_bytes(org))
                .bind(uuid::Uuid::from_bytes(connection))
                .fetch_one(&broker)
                .await
                .expect("the connection row exists");
        assert_eq!(state, "linked", "the upsert ran before the secret insert");

        let (versions,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM connection_secret WHERE org_id = $1 AND connection_id = $2 \
             AND key_version = $3",
        )
        .bind(uuid::Uuid::from_bytes(org))
        .bind(uuid::Uuid::from_bytes(connection))
        .bind(KEY_VERSION)
        .fetch_one(&broker)
        .await
        .expect("the secret row is countable");
        assert_eq!(
            versions, 1,
            "the sealed credential landed in the same transaction"
        );
    }

    #[sqlx::test(migrations = "../tam-storage/migrations")]
    async fn two_organisations_cannot_hold_the_same_marketplace_account(app: PgPool) {
        let first = [0xC1; 16];
        let second = [0xC2; 16];
        seed_org(&app, first, "org-first").await;
        seed_org(&app, second, "org-second").await;
        let broker = broker_pool(&app).await;
        let vault = Vault::new(broker.clone(), Kek::from_bytes(&[0x31; 32]).expect("kek"));

        link_tpt(&vault, first, [0xD1; 16]).await;
        vault
            .claim(
                OrgId(Uuid(first)),
                ConnectionId(Uuid([0xD1; 16])),
                Marketplace::Tpt,
                SHARED_ACCOUNT,
            )
            .await
            .expect("the first organisation claims the account");

        link_tpt(&vault, second, [0xD2; 16]).await;
        let collision = vault
            .claim(
                OrgId(Uuid(second)),
                ConnectionId(Uuid([0xD2; 16])),
                Marketplace::Tpt,
                SHARED_ACCOUNT,
            )
            .await;
        assert!(
            matches!(
                collision,
                Err(VaultError::AccountAlreadyLinked(Marketplace::Tpt))
            ),
            "a second tenant claiming a held account must be refused by name-matched \
             constraint rather than by a read-then-write race, and got {collision:?}"
        );
        let Err(refusal) = collision else {
            panic!("the collision must be an error");
        };
        let message = refusal.to_string();
        assert!(
            message.contains("tpt"),
            "the refusal names the marketplace so the seller knows which link failed: {message}"
        );
        assert!(
            !message.contains(&uuid::Uuid::from_bytes(first).to_string()),
            "the refusal must never name the holder, or the constraint becomes a directory \
             of every seller on the platform: {message}"
        );

        let (second_state,): (String,) =
            sqlx::query_as("SELECT state FROM connection WHERE org_id = $1 AND id = $2")
                .bind(uuid::Uuid::from_bytes(second))
                .bind(uuid::Uuid::from_bytes([0xD2; 16]))
                .fetch_one(&broker)
                .await
                .expect("the refused connection still exists");
        assert_eq!(
            second_state, "linking",
            "a refused claim leaves the connection short of linked, so the lease scan will \
             not drive it against an account it does not own"
        );
    }

    #[sqlx::test(migrations = "../tam-storage/migrations")]
    async fn unlinking_releases_the_account_for_another_organisation(app: PgPool) {
        let first = [0xC3; 16];
        let second = [0xC4; 16];
        seed_org(&app, first, "org-third").await;
        seed_org(&app, second, "org-fourth").await;
        let vault = Vault::new(
            broker_pool(&app).await,
            Kek::from_bytes(&[0x32; 32]).expect("kek"),
        );

        link_tpt(&vault, first, [0xD3; 16]).await;
        vault
            .claim(
                OrgId(Uuid(first)),
                ConnectionId(Uuid([0xD3; 16])),
                Marketplace::Tpt,
                SHARED_ACCOUNT,
            )
            .await
            .expect("the first organisation claims the account");
        vault
            .revoke(OrgId(Uuid(first)), ConnectionId(Uuid([0xD3; 16])))
            .await
            .expect("the first organisation unlinks");

        link_tpt(&vault, second, [0xD4; 16]).await;
        vault
            .claim(
                OrgId(Uuid(second)),
                ConnectionId(Uuid([0xD4; 16])),
                Marketplace::Tpt,
                SHARED_ACCOUNT,
            )
            .await
            .expect(
                "the index is predicated on the live states, so an unlinked account is \
                 immediately claimable and a seller is never permanently locked out",
            );
    }

    #[sqlx::test(migrations = "../tam-storage/migrations")]
    async fn a_link_that_never_claimed_holds_no_lock(app: PgPool) {
        let crashed = [0xC5; 16];
        let other = [0xC6; 16];
        seed_org(&app, crashed, "org-crashed").await;
        seed_org(&app, other, "org-other").await;
        let broker = broker_pool(&app).await;
        let vault = Vault::new(broker.clone(), Kek::from_bytes(&[0x33; 32]).expect("kek"));

        // Phase one only: the credential is sealed and the row is `linking`,
        // which is exactly the state a crash between the two phases leaves.
        link_tpt(&vault, crashed, [0xD5; 16]).await;
        let (digest,): (Option<Vec<u8>>,) = sqlx::query_as(
            "SELECT platform_account_digest FROM connection WHERE org_id = $1 AND id = $2",
        )
        .bind(uuid::Uuid::from_bytes(crashed))
        .bind(uuid::Uuid::from_bytes([0xD5; 16]))
        .fetch_one(&broker)
        .await
        .expect("the half-linked row exists");
        assert_eq!(
            digest, None,
            "phase one writes no digest, which is what makes the crash inert"
        );

        link_tpt(&vault, other, [0xD6; 16]).await;
        vault
            .claim(
                OrgId(Uuid(other)),
                ConnectionId(Uuid([0xD6; 16])),
                Marketplace::Tpt,
                SHARED_ACCOUNT,
            )
            .await
            .expect(
                "a half-linked row holds no digest and therefore no lock, so a crash must \
                 not strand the account against every other tenant",
            );
    }

    #[sqlx::test(migrations = "../tam-storage/migrations")]
    async fn a_renewal_reseals_in_place_and_reopens_as_the_renewed_value(app: PgPool) {
        let org = [0xC7; 16];
        seed_org(&app, org, "org-reseal").await;
        let broker = broker_pool(&app).await;
        let vault = Vault::new(broker.clone(), Kek::from_bytes(&[0x34; 32]).expect("kek"));
        let connection = [0xD7; 16];
        vault
            .link(LinkRequest {
                org: OrgId(Uuid(org)),
                connection: ConnectionId(Uuid(connection)),
                marketplace: Marketplace::Tes,
                secret: Secret::new("session=original".to_owned()),
                authorship: None,
            })
            .await
            .expect("the link seals");

        vault
            .reseal(
                OrgId(Uuid(org)),
                ConnectionId(Uuid(connection)),
                Marketplace::Tes,
                Secret::new("session=renewed".to_owned()),
            )
            .await
            .expect("the renewal reseals");

        let reopened = vault
            .open_secret(
                OrgId(Uuid(org)),
                ConnectionId(Uuid(connection)),
                Marketplace::Tes,
            )
            .await
            .expect("the resealed credential opens");
        assert_eq!(
            reopened.expose(),
            "session=renewed",
            "the next lease must open the renewed jar, or the reseal bought nothing"
        );

        let (versions,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM connection_secret WHERE org_id = $1 AND connection_id = $2",
        )
        .bind(uuid::Uuid::from_bytes(org))
        .bind(uuid::Uuid::from_bytes(connection))
        .fetch_one(&broker)
        .await
        .expect("the secret rows count");
        assert_eq!(
            versions, 1,
            "the reseal updates in place; a busy session must not accumulate a row per renewal"
        );

        let (verified, failures): (Option<i64>, i32) = sqlx::query_as(
            "SELECT (extract(epoch from session_verified_at) * 1000)::bigint, refresh_failures \
             FROM connection WHERE org_id = $1 AND id = $2",
        )
        .bind(uuid::Uuid::from_bytes(org))
        .bind(uuid::Uuid::from_bytes(connection))
        .fetch_one(&broker)
        .await
        .expect("the freshness columns read");
        assert!(
            verified.is_some(),
            "a renewal is proof the session was live, and the freshness axis must record it"
        );
        assert_eq!(
            failures, 0,
            "a renewal arriving clears the consecutive-failure count"
        );
    }

    #[sqlx::test(migrations = "../tam-storage/migrations")]
    async fn a_tpt_link_seals_the_authorship_the_seller_attested(app: PgPool) {
        let org = [0xA4; 16];
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a4', now())")
            .bind(uuid::Uuid::from_bytes(org))
            .execute(&app)
            .await
            .expect("the org inserts");
        let database: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&app)
            .await
            .expect("the database name reads");
        let broker = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&format!(
                "postgres://tam_broker:tam_broker_dev@127.0.0.1:5433/{database}"
            ))
            .await
            .expect("the broker role connects");
        let vault = Vault::new(broker.clone(), Kek::from_bytes(&[0x25; 32]).expect("kek"));
        let connection = [0xB5; 16];
        vault
            .link(LinkRequest {
                org: OrgId(Uuid(org)),
                connection: ConnectionId(Uuid(connection)),
                marketplace: Marketplace::Tpt,
                secret: Secret::new("csrfToken=abc".to_owned()),
                authorship: Some(&Authorship {
                    name: "Sample Teaching Studio".to_owned(),
                    attested_at_ms: 1_750_000_000_000,
                }),
            })
            .await
            .expect("the Tpt link seals");

        let (state, name, attested): (String, Option<String>, Option<i64>) = sqlx::query_as(
            "SELECT state, authorship_name, \
             (extract(epoch from authorship_attested_at) * 1000)::bigint \
             FROM connection WHERE org_id = $1 AND id = $2",
        )
        .bind(uuid::Uuid::from_bytes(org))
        .bind(uuid::Uuid::from_bytes(connection))
        .fetch_one(&broker)
        .await
        .expect("the connection row exists");
        assert_eq!(
            state, "linking",
            "Tpt can name its account, so the link waits for the claim rather than completing"
        );
        assert_eq!(
            name.as_deref(),
            Some("Sample Teaching Studio"),
            "the attestation the seller supplied is stored on the row, not in configuration"
        );
        assert_eq!(
            attested,
            Some(1_750_000_000_000),
            "the attested instant survives the round trip rather than being reminted here"
        );
    }
}
