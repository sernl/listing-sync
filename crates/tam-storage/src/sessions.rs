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

/// Who a resolved session speaks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionIdentity {
    pub org: OrgId,
    pub user: UserId,
}

pub struct SessionRepo {
    pool: PgPool,
}

impl SessionRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
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
            "SELECT org_id, user_id FROM user_session \
             WHERE token_digest = $1 AND expires_at > $2",
            token.digest(),
            timestamp_to_db(now)?,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| SessionIdentity {
            org: OrgId(crate::codec::uuid_from_db(row.org_id)),
            user: UserId(crate::codec::uuid_from_db(row.user_id)),
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
