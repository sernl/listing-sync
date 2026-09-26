//! Where else a seller sells: the request the Marketplaces page's last card
//! sends, and the operator's listing of what has been asked for.
//!
//! One write and one read, and the read is on the other side of the tenant
//! fence: the seller's own POST takes [`OrgContext`] and reaches their
//! organisation and no other, while the operator listing takes
//! [`OperatorContext`] and runs on the backoffice pool, whose whole reach is
//! migration 0055's SELECT grant and read policy.
//!
//! Nothing here changes what the platform supports. Which marketplaces exist
//! is the closed `Marketplace` enum's answer, and a row in this table is a
//! message to a human rather than an input to any decision the server makes.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{
    MarketplaceRequestBackofficeRepo, MarketplaceRequestRecord, MarketplaceRequestRepo,
    MarketplaceRequestWrite, NewMarketplaceRequest, REQUESTS_PER_ORG_MAX, REQUEST_PAGE_LIMIT_MAX,
};
use tam_types::{OrgId, Timestamp, UserId, Uuid};

use crate::admin::backoffice;
use crate::error::{APIError, APIErrorEntry, APIErrorKind};
use crate::jobs::{decode_cursor, encode_cursor};
use crate::session::{OperatorContext, OrgContext};
use crate::text::is_typed_text;
use crate::AppState;

/// The longest marketplace name the form accepts, counted in characters rather
/// than bytes so a name written in accented or non-Latin letters is measured
/// the way the seller who typed it sees it.
pub const NAME_MAX_CHARS: usize = 120;

/// The longest web address the form accepts. Two kilobytes is the length every
/// browser and proxy in the path handles without truncating, and a link longer
/// than that is not a shop front.
pub const URL_MAX_CHARS: usize = 2048;

/// The longest description the form accepts. Long enough for a seller to say
/// what they sell and why it matters to them, short enough that the column is
/// a message rather than an upload.
pub const REASON_MAX_CHARS: usize = 2000;

/// How many rows the operator listing answers with when it is not told.
/// The ceiling on what it may be told is the repository's own, so a caller
/// cannot page the whole table in one request.
const LISTING_LIMIT_DEFAULT: i64 = 50;

fn storage_fault(state: &AppState, error: &tam_storage::StorageError) -> APIError {
    state.internal(&error.to_string())
}

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

/// One request as stored. Served to the seller who wrote it and to an
/// operator reading across tenants, in one shape: the two fields a seller does
/// not need — their own organisation and their own user — are ones they can
/// already read from `whoami`, so a second view would differ without
/// concealing anything.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceRequestView {
    pub id: Uuid,
    pub org: OrgId,
    pub requested_by: UserId,
    /// The address to answer at, on the operator listing. Absent on the
    /// seller's own answer, where the field's absence is a fact about the
    /// surface rather than about the person: it is their own address and this
    /// route is not where they read it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_by_email: Option<String>,
    pub name: String,
    pub url: String,
    pub reason: String,
    pub created_at: Timestamp,
}

/// What the operator listing answers: one page, and the token for the next
/// one where a next one exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceRequestsPage {
    pub requests: Vec<MarketplaceRequestView>,
    /// Absent on the last page. Opaque: it is this server's own encoding of
    /// where the page ended, and anything else is refused rather than guessed
    /// at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// The page an operator asks for. Both fields optional, so the bare route
/// answers the newest page.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ListingParams {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

impl MarketplaceRequestView {
    fn of(record: MarketplaceRequestRecord) -> Self {
        Self {
            id: record.id,
            org: record.org,
            requested_by: record.requested_by,
            requested_by_email: record.requested_by_email,
            name: record.name,
            url: record.url,
            reason: record.reason,
            created_at: record.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    pub name: String,
    pub url: String,
    pub reason: String,
}

/// What the three fields must be for the row to be worth writing.
///
/// Trimmed rather than rejected for surrounding space, because a pasted web
/// address arrives with it and refusing that teaches a seller nothing. The
/// bounds are counted after trimming, so a field is measured as it will be
/// stored.
#[derive(Debug)]
struct Validated<'a> {
    name: &'a str,
    url: &'a str,
    reason: &'a str,
}

fn bounded<'a>(raw: &'a str, max: usize, empty: &str, what: &str) -> Result<&'a str, APIError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(validation(empty));
    }
    if trimmed.chars().count() > max {
        return Err(validation(&format!(
            "Keep {what} to {max} characters or fewer."
        )));
    }
    // Before the column sees it. A zero byte passes every check above -- trim
    // does not strip it and it counts as one character -- and no Postgres
    // `text` column can hold one, so without this the field arrives as a fault
    // rather than as the refusal it is.
    if !is_typed_text(trimmed) {
        return Err(validation(&format!(
            "Remove hidden characters from {what}."
        )));
    }
    Ok(trimmed)
}

/// Whether this is an address a browser could open.
///
/// A scheme test and a host test, not a parser: the row is read by a person
/// who will click the link, and what that person needs is that it is an
/// http(s) address with something after the scheme. Anything stricter would
/// refuse shop URLs that work, which costs a request we wanted.
///
/// What that looseness admits, named so the next reader need not derive it: a
/// loopback or link-local host, a private address, an embedded credential, a
/// port. None of them is a server-side-request hazard here, because nothing
/// on this side ever fetches the value -- it is stored, listed, and clicked by
/// an operator. A route that did fetch it would need its own allow-list rather
/// than a stricter version of this.
///
/// The scheme is matched case-folded, because a seller who typed `HTTPS://`
/// typed an address, and refusing it with a message spelling the scheme in
/// lowercase reads as a server that cannot see what is in front of it.
fn is_web_address(url: &str) -> bool {
    let folded = url.to_ascii_lowercase();
    let Some(offset) = folded
        .strip_prefix("https://")
        .or_else(|| folded.strip_prefix("http://"))
        .map(|rest| url.len() - rest.len())
    else {
        return false;
    };
    let rest = url.get(offset..).unwrap_or("");
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    !host.is_empty() && !host.contains(char::is_whitespace)
}

fn validated(body: &CreateBody) -> Result<Validated<'_>, APIError> {
    let name = bounded(
        &body.name,
        NAME_MAX_CHARS,
        "Enter the marketplace's name.",
        "the marketplace name",
    )?;
    let url = bounded(
        &body.url,
        URL_MAX_CHARS,
        "Enter the marketplace's web address.",
        "the web address",
    )?;
    if !is_web_address(url) {
        return Err(validation(
            "Enter a full web address that starts with https:// or http://.",
        ));
    }
    let reason = bounded(
        &body.reason,
        REASON_MAX_CHARS,
        "Tell us what you sell there.",
        "your description",
    )?;
    Ok(Validated { name, url, reason })
}

/// Records one seller's request, answering it as stored rather than as sent:
/// every field is trimmed on the way in, so a client echoing its own body
/// would render values this row does not hold.
///
/// The bounds, all of them refused as validation with a sentence the form can
/// render: the name is 1 to [`NAME_MAX_CHARS`] characters, the address is 1 to
/// [`URL_MAX_CHARS`] and must be an `http` or `https` address naming a host,
/// the description is 1 to [`REASON_MAX_CHARS`], none of the three may carry a
/// control character, one organisation may name any one address only once, and
/// one organisation may hold at most [`REQUESTS_PER_ORG_MAX`] requests in all.
/// The last two exist because this table has exactly one reader — an operator
/// page — and a tenant appending to it without bound pushes every other
/// tenant's request out of view.
pub(crate) async fn create(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<CreateBody>,
) -> Result<(StatusCode, Json<MarketplaceRequestView>), APIError> {
    let fields = validated(&body)?;
    let written = MarketplaceRequestRepo::new(state.pool.clone())
        .create(
            context.org,
            &NewMarketplaceRequest {
                id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                requested_by: context.user,
                name: fields.name,
                url: fields.url,
                reason: fields.reason,
                created_at: (state.wall)(),
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    match written {
        MarketplaceRequestWrite::Recorded(record) => Ok((
            StatusCode::CREATED,
            Json(MarketplaceRequestView::of(record)),
        )),
        // Validation rather than a conflict status, for the reason every other
        // refusal on this route is: the client renders the sentence, and
        // `APIErrorKind` has no conflict member to branch on that a fourth
        // status code would not duplicate.
        MarketplaceRequestWrite::AlreadyAsked => {
            Err(validation("You've already asked us about that address."))
        }
        MarketplaceRequestWrite::TooMany => Err(validation(&format!(
            "You have {REQUESTS_PER_ORG_MAX} requests with us already. Tell us about the next \
             one in a reply instead."
        ))),
    }
}

/// One page of every tenant's requests, newest first.
///
/// Paged rather than a fixed prefix: one tenant writing steadily would
/// otherwise hold the whole answer and every other tenant's request would be
/// unreachable through the API. `limit` defaults to [`LISTING_LIMIT_DEFAULT`]
/// and is clamped by the repository, and `cursor` is the token the previous
/// page answered with — anything else is refused rather than guessed at.
pub(crate) async fn list_all(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Query(params): Query<ListingParams>,
) -> Result<Json<MarketplaceRequestsPage>, APIError> {
    let cursor = match params.cursor.as_deref() {
        None => None,
        Some(raw) => Some(
            decode_cursor(raw)
                .ok_or_else(|| validation("This page link has expired. Reload the page."))?,
        ),
    };
    // Clamped here as well as in the repository, and by the same constant, so
    // that a caller asking for more than the ceiling still learns where its
    // page ended: a limit the repository silently reduced would never equal
    // the row count below, and the walk would stop on its first page.
    let limit = params
        .limit
        .unwrap_or(LISTING_LIMIT_DEFAULT)
        .clamp(1, REQUEST_PAGE_LIMIT_MAX);
    let records = MarketplaceRequestBackofficeRepo::new(backoffice(&state)?)
        .newest(cursor, limit)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    // A next page is offered only when this one filled, and the token is the
    // last row's own key, so a page that ended early ends the walk rather than
    // handing back a cursor that answers nothing.
    let next_cursor = (i64::try_from(records.len()).unwrap_or(i64::MAX) == limit)
        .then(|| {
            records.last().map(|last| {
                encode_cursor(&tam_storage::LedgerCursor {
                    created_at: last.created_at,
                    id: last.id,
                })
            })
        })
        .flatten();
    Ok(Json(MarketplaceRequestsPage {
        requests: records
            .into_iter()
            .map(MarketplaceRequestView::of)
            .collect(),
        next_cursor,
    }))
}

#[cfg(test)]
mod tests {
    use super::{is_web_address, validated, CreateBody, NAME_MAX_CHARS, REASON_MAX_CHARS};
    use crate::error::APIErrorKind;
    use axum::http::StatusCode;

    fn body(name: &str, url: &str, reason: &str) -> CreateBody {
        CreateBody {
            name: name.to_owned(),
            url: url.to_owned(),
            reason: reason.to_owned(),
        }
    }

    #[test]
    fn every_field_is_stored_trimmed() {
        let sent = body(
            "  Amped Up Learning \n",
            "\thttps://ampeduplearning.com/shop  ",
            " Science units. ",
        );
        let accepted = validated(&sent).expect("a well-formed request is accepted");
        assert_eq!(
            (accepted.name, accepted.url, accepted.reason),
            (
                "Amped Up Learning",
                "https://ampeduplearning.com/shop",
                "Science units."
            ),
            "the stored values are the trimmed ones"
        );
    }

    #[test]
    fn a_field_that_is_only_whitespace_is_refused_as_empty() {
        for request in [
            body("   ", "https://example.test", "why"),
            body("Somewhere", "  ", "why"),
            body("Somewhere", "https://example.test", "\t\n"),
        ] {
            let refused = validated(&request).expect_err("a blank field is refused");
            assert_eq!(
                refused.status_code(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "a blank field is the caller's error rather than a fault"
            );
            assert_eq!(
                refused.errors[0].kind,
                Some(APIErrorKind::Validation),
                "the refusal carries the kind the client branches on"
            );
        }
    }

    #[test]
    fn the_bounds_are_counted_in_characters_and_after_trimming() {
        // Two bytes per character, so a byte-counting bound would refuse the
        // name a character-counting one accepts.
        let longest = "\u{e9}".repeat(NAME_MAX_CHARS);
        assert!(
            validated(&body(
                &format!(" {longest} "),
                "https://example.test",
                "why"
            ))
            .is_ok(),
            "{NAME_MAX_CHARS} characters is accepted, and the surrounding space is not counted"
        );
        let refused = validated(&body(
            &"\u{e9}".repeat(NAME_MAX_CHARS + 1),
            "https://example.test",
            "why",
        ))
        .expect_err("one character too many is refused");
        assert!(
            refused.errors[0]
                .message
                .contains(&NAME_MAX_CHARS.to_string()),
            "the refusal states the bound it applied: {}",
            refused.errors[0].message
        );
        let long_reason = "a".repeat(REASON_MAX_CHARS + 1);
        assert!(
            validated(&body("Somewhere", "https://example.test", &long_reason)).is_err(),
            "the description carries a bound of its own"
        );
    }

    #[test]
    fn only_an_http_address_naming_a_host_is_accepted() {
        for accepted in [
            "https://example.test",
            "http://example.test",
            "https://example.test/shop?ref=1",
            "https://sub.example.test:8443/path",
        ] {
            assert!(
                is_web_address(accepted),
                "a browser can open this one: {accepted}"
            );
        }
        for refused in [
            "example.test",
            "ftp://example.test",
            "javascript:alert(1)",
            "https://",
            "https:// example.test",
            "file:///etc/passwd",
        ] {
            assert!(
                !is_web_address(refused),
                "this is not a marketplace a seller can be sent to: {refused}"
            );
        }
    }
}
