//! The session floor: the cookie-session verifier store behind the API's
//! tenant boundary. The table holds the token's BLAKE3 digest, never the
//! token, so a database read yields a verifier and not the capability; the
//! token itself exists in the client's cookie and transiently in the process
//! that minted or resolved it. Entropy and the clock both enter as data from
//! the process boundary, per the lint table.

use sqlx::PgPool;
use tam_types::{OrgId, Timestamp, UserId};

use crate::codec::{timestamp_to_db, uuid_to_db};
use crate::StorageError;

/// The bearer capability: 32 random bytes, hex-carried in the cookie. Held
/// by value and never stored; only its digest reaches the database.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SessionToken(pub [u8; 32]);

impl core::fmt::Debug for SessionToken {
    /// The token is a credential; Debug must not leak it into a log line.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SessionToken(..)")
    }
}

impl SessionToken {
    #[must_use]
    pub fn to_hex(&self) -> String {
        use core::fmt::Write;
        let mut hex = String::with_capacity(64);
        for byte in self.0 {
            // infallible on String; the Result is the trait's, not the writer's
            let _unused: core::fmt::Result = write!(hex, "{byte:02x}");
        }
        hex
    }

    /// Parses the 64-character lowercase-or-uppercase hex form; anything
    /// else is `None`, so a malformed cookie reads as no session.
    #[must_use]
    pub fn from_hex(hex: &str) -> Option<Self> {
        if hex.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (index, slot) in bytes.iter_mut().enumerate() {
            let pair = hex.get(index * 2..index * 2 + 2)?;
            *slot = u8::from_str_radix(pair, 16).ok()?;
        }
        Some(Self(bytes))
    }

    fn digest(&self) -> Vec<u8> {
        tam_pipeline::hash::content_hash(&self.0).0.to_vec()
    }
}

/// Who a resolved session speaks for, and until when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionIdentity {
    pub org: OrgId,
    pub user: UserId,
    pub expires_at: Timestamp,
}

pub struct SessionRepo {
    pool: PgPool,
}

impl SessionRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Ensures the organisation row exists, for the development mint path:
    /// a fresh database has no tenant until something creates one, and
    /// self-serve signup is M5's. Idempotent on the id.
    pub async fn ensure_org(
        &self,
        org: OrgId,
        name: &str,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        sqlx::query!(
            "INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, $3) \
             ON CONFLICT (id) DO NOTHING",
            uuid_to_db(org.0),
            name,
            timestamp_to_db(at)?,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Creates the user row a session hangs off. Minting is an operator
    /// one-shot until M5 lands self-serve signup.
    pub async fn create_user(
        &self,
        org: OrgId,
        user: UserId,
        email: &str,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        sqlx::query!(
            "INSERT INTO app_user (id, org_id, email, created_at) VALUES ($1, $2, $3, $4)",
            uuid_to_db(user.0),
            uuid_to_db(org.0),
            email,
            timestamp_to_db(at)?,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Stores the digest of a freshly generated token. The token itself is
    /// the caller's to print once and forget.
    pub async fn mint(
        &self,
        token: &SessionToken,
        user: UserId,
        expires_at: Timestamp,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        sqlx::query!(
            "INSERT INTO user_session (token_digest, user_id, org_id, expires_at, created_at) \
             SELECT $1, id, org_id, $3, $4 FROM app_user WHERE id = $2",
            token.digest(),
            uuid_to_db(user.0),
            timestamp_to_db(expires_at)?,
            timestamp_to_db(at)?,
        )
        .execute(&self.pool)
        .await
        .map_err(StorageError::Db)
        .and_then(|done| {
            if done.rows_affected() == 1 {
                Ok(())
            } else {
                Err(StorageError::Inconsistent {
                    reason: "a session must hang off an existing user".to_owned(),
                })
            }
        })
    }

    /// The cookie's token to its identity, or `None` for unknown and expired
    /// alike — the caller cannot distinguish them, which is the point.
    pub async fn resolve(
        &self,
        token: &SessionToken,
        now: Timestamp,
    ) -> Result<Option<SessionIdentity>, StorageError> {
        let row = sqlx::query!(
            "SELECT org_id, user_id, expires_at FROM user_session \
             WHERE token_digest = $1 AND expires_at > $2",
            token.digest(),
            timestamp_to_db(now)?,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| SessionIdentity {
            org: OrgId(crate::codec::uuid_from_db(row.org_id)),
            user: UserId(crate::codec::uuid_from_db(row.user_id)),
            expires_at: crate::codec::timestamp_from_db(row.expires_at),
        }))
    }

    /// Deletes the session outright; a bearer token has nothing to tombstone.
    pub async fn expire(&self, token: &SessionToken) -> Result<bool, StorageError> {
        let done = sqlx::query!(
            "DELETE FROM user_session WHERE token_digest = $1",
            token.digest(),
        )
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }
}

impl SessionRepo {
    /// The mint path's find-or-create half: an email already provisioned
    /// yields its existing user rather than a second one.
    pub async fn user_by_email(&self, email: &str) -> Result<Option<UserId>, StorageError> {
        let row = sqlx::query!("SELECT id FROM app_user WHERE email = $1", email)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| UserId(crate::codec::uuid_from_db(row.id))))
    }
}

/// The tenant a subject seen for the first time becomes. The address and the
/// name are the caller's rather than derived here, because what a signup
/// without either should carry is a product decision and not a storage one.
#[derive(Debug, Clone, Copy)]
pub struct NewTenant<'a> {
    pub subject: tam_types::Uuid,
    pub org: OrgId,
    pub user: UserId,
    pub email: &'a str,
    pub org_name: &'a str,
}

/// The identity service's subject to the tenant it speaks for.
///
/// The join key is `app_user.auth_subject` rather than the email, because the
/// identity service lets a user change their address: an email join would
/// rebind an identity to whichever row currently holds it (migration 0035).
impl SessionRepo {
    pub async fn user_by_auth_subject(
        &self,
        subject: tam_types::Uuid,
    ) -> Result<Option<(OrgId, UserId)>, StorageError> {
        let row = sqlx::query!(
            "SELECT id, org_id FROM app_user WHERE auth_subject = $1",
            uuid_to_db(subject),
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| {
            (
                OrgId(crate::codec::uuid_from_db(row.org_id)),
                UserId(crate::codec::uuid_from_db(row.id)),
            )
        }))
    }

    /// A subject seen for the first time becomes a tenant: the organisation
    /// and its first user in one transaction, so a crash between the two
    /// cannot leave a user referencing an organisation that does not exist.
    /// This is self-serve signup; the caller has already established that the
    /// assertion is valid and its address verified.
    ///
    /// Two logins racing for the same new subject both reach here. The unique
    /// constraint on `auth_subject` decides, the loser's transaction rolls
    /// back — organisation included, so no orphan tenant survives — and the
    /// row the winner wrote is returned to both.
    pub async fn provision_for_auth_subject(
        &self,
        tenant: NewTenant<'_>,
        at: Timestamp,
    ) -> Result<(OrgId, UserId), StorageError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            "INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, $3)",
            uuid_to_db(tenant.org.0),
            tenant.org_name,
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        let inserted = sqlx::query!(
            "INSERT INTO app_user (id, org_id, email, created_at, auth_subject) \
             VALUES ($1, $2, $3, $4, $5)",
            uuid_to_db(tenant.user.0),
            uuid_to_db(tenant.org.0),
            tenant.email,
            timestamp_to_db(at)?,
            uuid_to_db(tenant.subject),
        )
        .execute(&mut *tx)
        .await;
        match inserted {
            Ok(_) => {
                tx.commit().await?;
                Ok((tenant.org, tenant.user))
            }
            Err(sqlx::Error::Database(database))
                if database.constraint() == Some("app_user_auth_subject") =>
            {
                drop(tx);
                self.user_by_auth_subject(tenant.subject)
                    .await?
                    .ok_or(StorageError::Inconsistent {
                        reason: "the winning signup's user must exist".to_owned(),
                    })
            }
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SessionToken;

    #[test]
    fn the_hex_form_round_trips() {
        let token = SessionToken([0xA7; 32]);
        let hex = token.to_hex();
        assert_eq!(hex.len(), 64, "32 bytes render as 64 hex characters");
        assert_eq!(
            SessionToken::from_hex(&hex),
            Some(token),
            "the cookie form parses back to the same token"
        );
    }

    #[test]
    fn a_malformed_cookie_reads_as_no_session() {
        assert_eq!(SessionToken::from_hex(""), None, "empty");
        assert_eq!(SessionToken::from_hex("ab"), None, "too short");
        assert_eq!(
            SessionToken::from_hex(&"zz".repeat(32)),
            None,
            "non-hex characters"
        );
    }

    #[test]
    fn debug_never_prints_the_token() {
        let rendered = format!("{:?}", SessionToken([0xA7; 32]));
        assert!(
            !rendered.contains("a7"),
            "a credential must not leak through Debug: {rendered}"
        );
    }
}
