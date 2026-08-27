//! The vault: the broker-role database access plus the seal/open envelope.
//! This is the only code in the system that holds ciphertext and the
//! key-encryption key in the same process.

use sqlx::PgPool;
use tam_secrets::{open, seal, AadContext, Kek, Secret};
use tam_types::{ConnectionId, Marketplace, OrgId};

pub(crate) const KEY_VERSION: i32 = 1;

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
}

impl core::fmt::Display for VaultError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Db(error) => write!(f, "vault database error: {error}"),
            Self::Seal => f.write_str("sealing failed"),
            Self::Open(detail) => write!(f, "opening failed: {detail}"),
            Self::NoSecret => f.write_str("no stored credential for this connection"),
        }
    }
}

impl core::error::Error for VaultError {}

impl From<sqlx::Error> for VaultError {
    fn from(error: sqlx::Error) -> Self {
        Self::Db(error)
    }
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

impl Vault {
    #[must_use]
    pub(crate) fn new(pool: PgPool, kek: Kek) -> Self {
        Self { pool, kek }
    }

    /// Seals the credential and marks the connection linked, both in one
    /// transaction. The secret is moved in and never leaves.
    pub(crate) async fn link(
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
        // The connection row must exist before the secret row: the secret's
        // foreign key points at it, so a fresh connection linked in the
        // other order dies on the constraint before the upsert can run.
        sqlx::query(
            "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
             VALUES ($1, $2, $3, 'linked', now(), now()) \
             ON CONFLICT (org_id, id) DO UPDATE SET state = 'linked', updated_at = now()",
        )
        .bind(uuid(org.0))
        .bind(uuid(connection.0))
        .bind(marketplace_to_db(marketplace))
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

    /// Revokes one connection: tombstones its ciphertext and marks it revoked.
    /// No DELETE — the row stays as a tombstone so the revocation is auditable.
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
            "UPDATE connection SET state = 'revoked', updated_at = now() \
             WHERE org_id = $1 AND id = $2",
        )
        .bind(uuid(org.0))
        .bind(uuid(connection.0))
        .execute(&mut *tx)
        .await?;
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
        sqlx::query("UPDATE connection SET state = 'revoked', updated_at = now()")
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

#[cfg(all(test, feature = "pg-tests"))]
mod tests {
    use super::{Vault, KEY_VERSION};
    use sqlx::PgPool;
    use tam_secrets::{Kek, Secret};
    use tam_types::{ConnectionId, Marketplace, OrgId, Uuid};

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
            .link(
                OrgId(Uuid(org)),
                ConnectionId(Uuid(connection)),
                Marketplace::Tes,
                Secret::new("TESSession=fresh".to_owned()),
            )
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
}
