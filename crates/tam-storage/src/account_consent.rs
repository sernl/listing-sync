//! What a person agreed to when their account was made: the Terms of Service
//! and the Privacy Policy, ownership of what they publish, and being 18 or
//! older (migration 0103).
//!
//! Not to be confused with [`crate::consent`], which is a seller's permission
//! for one marketplace. This record belongs to the person, keyed on their
//! identity subject, and is written before any organisation exists.
//!
//! Append-only: an agreement is recorded once per terms version and never
//! rewritten. The sign-up's rows arrive through the identity service, a later
//! version's through the signed-in console; both call [`AccountConsentRepo::record`].

use std::net::IpAddr;

use sqlx::PgPool;
use tam_types::{Timestamp, UserId, Uuid};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db};
use crate::StorageError;

/// One of the three statements a person agrees to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccountConsentKind {
    /// The Terms of Service and the Privacy Policy.
    TermsPrivacy,
    /// They own, or hold the rights to, every resource they publish.
    IpOwnership,
    /// They are 18 or older.
    Age18,
}

impl AccountConsentKind {
    /// The database's spelling, which is also the wire's.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TermsPrivacy => "terms_privacy",
            Self::IpOwnership => "ip_ownership",
            Self::Age18 => "age_18",
        }
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "terms_privacy" => Ok(Self::TermsPrivacy),
            "ip_ownership" => Ok(Self::IpOwnership),
            "age_18" => Ok(Self::Age18),
            other => Err(StorageError::CorruptRow {
                reason: format!("account_consent.kind {other} is not a known statement"),
            }),
        }
    }
}

/// One agreement to all three statements, as it is about to be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAccountConsent<'a> {
    /// The identity subject, `auth."user".id`.
    pub subject: Uuid,
    pub email: Option<&'a str>,
    /// The Terms' effective date, `YYYY-MM-DD`.
    pub document_version: &'a str,
    pub accepted_at: Timestamp,
    pub ip_address: Option<IpAddr>,
    pub user_agent: Option<&'a str>,
}

/// One stored row, as the operator's user detail reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountConsentRecord {
    pub kind: AccountConsentKind,
    pub document_version: String,
    pub accepted_at: Timestamp,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub email: Option<String>,
}

/// The newest Terms acceptance one subject holds, for the Users page column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountConsentSummary {
    pub subject: Uuid,
    pub document_version: String,
    pub accepted_at: Timestamp,
}

/// The longest user agent kept. The column refuses anything longer; a header
/// past this is truncated rather than costing the person their sign-up.
pub const USER_AGENT_MAX: usize = 1024;

fn bounded(agent: &str) -> &str {
    match agent.char_indices().nth(USER_AGENT_MAX) {
        Some((end, _)) => agent.get(..end).unwrap_or(agent),
        None => agent,
    }
}

pub struct AccountConsentRepo {
    pool: PgPool,
}

impl AccountConsentRepo {
    /// The application pool writes and answers the signed-in person's status;
    /// the backoffice pool reads the operator's views. Both are granted what
    /// they need by migration 0103.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Writes one agreement: a row for each of the three statements, in one
    /// statement, so a record is never left holding two of the three.
    pub async fn record(&self, consent: &NewAccountConsent<'_>) -> Result<(), StorageError> {
        sqlx::query!(
            "INSERT INTO account_consent \
                 (subject, email, kind, document_version, accepted_at, ip_address, user_agent) \
             SELECT $1, $2, kind, $3, $4, $5::text::inet, $6 \
             FROM unnest(enum_range(NULL::account_consent_kind)) AS kind",
            uuid_to_db(consent.subject),
            consent.email,
            consent.document_version,
            timestamp_to_db(consent.accepted_at)?,
            consent.ip_address.map(|address| address.to_string()),
            consent.user_agent.map(bounded),
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Whether this subject has agreed to all three statements under
    /// `version`.
    pub async fn accepted(&self, subject: Uuid, version: &str) -> Result<bool, StorageError> {
        let kinds = sqlx::query_scalar!(
            "SELECT count(DISTINCT kind) AS \"kinds!\" FROM account_consent \
             WHERE subject = $1 AND document_version = $2",
            uuid_to_db(subject),
            version,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(kinds == 3)
    }

    /// The identity subject and address of one app user, for the signed-in
    /// path, which knows the user and not the subject. `None` for a user that
    /// does not exist; a subject of `None` for one provisioned before the
    /// identity service, who has no account to agree with.
    pub async fn identity_of(
        &self,
        user: UserId,
    ) -> Result<Option<(Option<Uuid>, String)>, StorageError> {
        let row = sqlx::query!(
            "SELECT auth_subject, email FROM app_user WHERE id = $1",
            uuid_to_db(user.0),
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| (row.auth_subject.map(uuid_from_db), row.email)))
    }

    /// The version of the newest Terms acceptance this subject holds, if any.
    pub async fn latest_version(&self, subject: Uuid) -> Result<Option<String>, StorageError> {
        Ok(sqlx::query_scalar!(
            "SELECT document_version FROM account_consent \
             WHERE subject = $1 AND kind = 'terms_privacy' \
             ORDER BY accepted_at DESC, created_at DESC LIMIT 1",
            uuid_to_db(subject),
        )
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Every row one subject holds, newest first.
    pub async fn for_subject(
        &self,
        subject: Uuid,
    ) -> Result<Vec<AccountConsentRecord>, StorageError> {
        let rows = sqlx::query!(
            "SELECT kind::text AS \"kind!\", document_version, accepted_at, \
                    host(ip_address) AS ip_address, user_agent, email \
             FROM account_consent WHERE subject = $1 \
             ORDER BY accepted_at DESC, kind",
            uuid_to_db(subject),
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(AccountConsentRecord {
                    kind: AccountConsentKind::from_db(&row.kind)?,
                    document_version: row.document_version,
                    accepted_at: timestamp_from_db(row.accepted_at),
                    ip_address: row.ip_address,
                    user_agent: row.user_agent,
                    email: row.email,
                })
            })
            .collect()
    }

    /// The newest Terms acceptance of every subject that holds one.
    pub async fn summaries(&self) -> Result<Vec<AccountConsentSummary>, StorageError> {
        let rows = sqlx::query!(
            "SELECT DISTINCT ON (subject) subject, document_version, accepted_at \
             FROM account_consent WHERE kind = 'terms_privacy' \
             ORDER BY subject, accepted_at DESC, created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| AccountConsentSummary {
                subject: uuid_from_db(row.subject),
                document_version: row.document_version,
                accepted_at: timestamp_from_db(row.accepted_at),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{bounded, USER_AGENT_MAX};

    #[test]
    fn a_long_user_agent_is_cut_on_a_character_boundary() {
        let long = "é".repeat(USER_AGENT_MAX + 10);
        assert_eq!(bounded(&long).chars().count(), USER_AGENT_MAX);
        assert_eq!(bounded("Mozilla/5.0"), "Mozilla/5.0");
    }
}
