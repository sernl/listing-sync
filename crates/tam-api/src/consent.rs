//! The seller's explicit permission for a marketplace that publishes no
//! official API, over the wire, and the gate every mint reads it through.
//!
//! The founder's rule: a no-API marketplace may be connected only after the
//! seller has read the notice, ticked the box and pressed "I agree". The
//! console asks at Connect and on the Account page; this module records the
//! answer and refuses, with [`APIErrorCode::ConsentRequired`], every request
//! that would start seller-device work without one standing.
//!
//! Where the gate sits, and why there. Work is minted in five handlers and
//! one worker, and each mints from a marketplace it already knows, so
//! [`require_grant`] is called immediately before each `NewJob` is built.
//! Heartbeats are not gated: a device that already holds a login keeps
//! reporting it, so the marketplace stays visible as linked and the console
//! can say what is missing rather than the row vanishing.
//!
//! A marketplace with an official API needs nothing here and is answered
//! `Ok` without a read: its automation runs server-side under a token the
//! marketplace issued, which is the marketplace's own sanction.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{ConsentRecord, ConsentRepo};
use tam_types::{Marketplace, OrgId, Timestamp, TransportClass, CONSENT_NOTICE_VERSION};

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

/// One grant, standing or withdrawn, as the console reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentView {
    pub marketplace: Marketplace,
    pub notice_version: String,
    pub granted_at: Timestamp,
    pub withdrawn_at: Option<Timestamp>,
    /// Not withdrawn, and on the current notice. What the gate reads.
    pub standing: bool,
}

impl ConsentView {
    fn of(record: ConsentRecord) -> Self {
        let standing =
            record.withdrawn_at.is_none() && record.notice_version == CONSENT_NOTICE_VERSION;
        Self {
            marketplace: record.marketplace,
            notice_version: record.notice_version,
            granted_at: record.granted_at,
            withdrawn_at: record.withdrawn_at,
            standing,
        }
    }
}

/// The whole record, newest grant first, with the version a new grant must
/// carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentsView {
    pub notice_version: String,
    pub consents: Vec<ConsentView>,
}

#[derive(Debug, Deserialize)]
pub struct GrantBody {
    pub notice_version: String,
    pub agreed: bool,
}

/// The name the wire spells a marketplace by, which is its serde name.
fn wire_name(marketplace: Marketplace) -> String {
    serde_json::to_value(marketplace)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{marketplace:?}"))
}

/// The refusal every mint answers with when no grant stands.
#[must_use]
pub fn consent_refusal(marketplace: Marketplace) -> APIError {
    let name = wire_name(marketplace);
    APIError::new(
        StatusCode::FORBIDDEN,
        APIErrorEntry::new(&format!(
            "{name} needs your permission before Teachouse can work with it. \
             Grant it under Account > Permissions."
        ))
        .code(APIErrorCode::ConsentRequired)
        .kind(APIErrorKind::Validation)
        .detail(serde_json::json!({
            "marketplace": name,
            "notice_version": CONSENT_NOTICE_VERSION,
        })),
    )
}

/// Refuses, with `ConsentRequired`, unless a grant on the current notice
/// stands for this marketplace. An official-API marketplace passes without a
/// read.
pub(crate) async fn require_grant(
    state: &AppState,
    org: OrgId,
    marketplace: Marketplace,
) -> Result<(), APIError> {
    if marketplace.transport_class() == TransportClass::OfficialApi {
        return Ok(());
    }
    let standing = ConsentRepo::new(state.pool.clone())
        .standing(org, marketplace, CONSENT_NOTICE_VERSION)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    if standing.is_some() {
        Ok(())
    } else {
        Err(consent_refusal(marketplace))
    }
}

fn marketplace_of(raw: &str) -> Result<Marketplace, APIError> {
    Marketplace::ALL
        .into_iter()
        .find(|marketplace| wire_name(*marketplace) == raw)
        .ok_or_else(|| {
            APIError::new(
                StatusCode::NOT_FOUND,
                APIErrorEntry::new("no such marketplace")
                    .code(APIErrorCode::ResourceMissing)
                    .kind(APIErrorKind::NotFound),
            )
        })
}

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

pub(crate) async fn list_consents(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<ConsentsView>, APIError> {
    let history = ConsentRepo::new(state.pool.clone())
        .history(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(ConsentsView {
        notice_version: CONSENT_NOTICE_VERSION.to_owned(),
        consents: history.into_iter().map(ConsentView::of).collect(),
    }))
}

/// Records the seller's agreement.
///
/// The body has to say `agreed: true` and name the current notice, because
/// the record is of an explicit act on a specific text: a client that posts
/// without the flag, or against a notice it read before the current one was
/// published, has not made the agreement this row would claim.
pub(crate) async fn grant_consent(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, marketplace)): Path<(String, String)>,
    Json(body): Json<GrantBody>,
) -> Result<Json<ConsentView>, APIError> {
    let marketplace = marketplace_of(&marketplace)?;
    if marketplace.transport_class() == TransportClass::OfficialApi {
        return Err(validation(&format!(
            "{} publishes an official API and needs no seller-device consent",
            wire_name(marketplace)
        )));
    }
    if !body.agreed {
        return Err(validation(
            "the grant is recorded only from an explicit agreement",
        ));
    }
    if body.notice_version != CONSENT_NOTICE_VERSION {
        return Err(validation(
            "this notice is out of date; reload and read the current one",
        ));
    }
    let record = ConsentRepo::new(state.pool.clone())
        .grant(
            context.org,
            marketplace,
            CONSENT_NOTICE_VERSION,
            context.user.0,
            (state.wall)(),
        )
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    state.telemetry.capture(
        context.org,
        "consent_granted",
        serde_json::json!({ "marketplace": wire_name(marketplace) }),
    );
    Ok(Json(ConsentView::of(record)))
}

/// Ends the standing grant. Idempotent: with nothing standing, the answer is
/// a view that does not stand, and nothing is written.
pub(crate) async fn withdraw_consent(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, marketplace)): Path<(String, String)>,
) -> Result<Json<ConsentView>, APIError> {
    let marketplace = marketplace_of(&marketplace)?;
    let repo = ConsentRepo::new(state.pool.clone());
    let now = (state.wall)();
    let withdrawn = repo
        .withdraw(context.org, marketplace, context.user.0, now)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    if let Some(record) = withdrawn {
        return Ok(Json(ConsentView::of(record)));
    }
    // The newest row, if any, else a view saying nothing was ever granted;
    // either way `standing` is false.
    let latest = repo
        .history(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .into_iter()
        .find(|record| record.marketplace == marketplace);
    Ok(Json(latest.map_or_else(
        || ConsentView {
            marketplace,
            notice_version: CONSENT_NOTICE_VERSION.to_owned(),
            granted_at: now,
            withdrawn_at: Some(now),
            standing: false,
        },
        ConsentView::of,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refusal_names_the_marketplace_and_the_notice() {
        let refusal = consent_refusal(Marketplace::Tpt);
        assert_eq!(refusal.status, 403);
        let entry = &refusal.errors[0];
        assert_eq!(entry.code, Some(APIErrorCode::ConsentRequired));
        assert!(entry.message.starts_with("Tpt needs your permission"));
        assert_eq!(
            entry.detail,
            Some(serde_json::json!({
                "marketplace": "Tpt",
                "notice_version": CONSENT_NOTICE_VERSION,
            }))
        );
    }

    #[test]
    fn a_view_stands_only_on_the_current_notice() {
        let record = ConsentRecord {
            marketplace: Marketplace::Tes,
            notice_version: CONSENT_NOTICE_VERSION.to_owned(),
            granted_by: tam_types::Uuid([1; 16]),
            granted_at: Timestamp(1),
            withdrawn_at: None,
        };
        assert!(ConsentView::of(record.clone()).standing);
        assert!(
            !ConsentView::of(ConsentRecord {
                notice_version: "2020-01-01".to_owned(),
                ..record.clone()
            })
            .standing
        );
        assert!(
            !ConsentView::of(ConsentRecord {
                withdrawn_at: Some(Timestamp(2)),
                ..record
            })
            .standing
        );
    }
}
