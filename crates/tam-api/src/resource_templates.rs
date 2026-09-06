//! Named starting points for a new resource: the Template Manager's second
//! tab, over the wire.
//!
//! A template is a partial `DraftInput` — the same shape
//! `POST /{version}/authoring/check` reports on — saved under a name so that
//! the next resource of a kind starts filled in. Five routes and one table:
//! the console lists them into a picker without their drafts, fetches the one
//! the seller chose, saves one, renames or re-saves one, and removes one.
//! Nothing here is enqueued, lowered, or read by the engine.
//!
//! The draft is stored as the client sent it rather than re-serialised from
//! `DraftInput`, because `DraftInput` is the form's own deserialisation target
//! and carries no `Serialize`; a round trip through it would need one, and the
//! shape that reaches the console has to be the shape its form reads. What
//! makes that safe is [`refusals`], which runs the create form's own value
//! rules over the draft before the column sees it, and the byte ceiling below.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_authoring::{refusal_of, selection_caps, stated_price_refusal, RefusalView};
use tam_domain::product::{
    AuthoringError, FacetSlug, Picker, ProductName, ThumbnailMode, UploadRef,
};
use tam_storage::{
    NewResourceTemplate, ResourceTemplateRecord, ResourceTemplateRepo, ResourceTemplateSummary,
    TemplateChange, TemplateEdit, TemplateWrite, TEMPLATES_PER_ORG_MAX,
};
use tam_types::{Timestamp, Uuid};

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::product::{record_of, DraftInput};
use crate::session::OrgContext;
use crate::text::is_typed_text;
use crate::AppState;

/// The longest template name the form accepts, counted in characters rather
/// than bytes so a name written in accented or non-Latin letters is measured
/// the way the seller who typed it sees it. It names one row in a picker, so
/// the bound is a menu entry's worth rather than a title's.
pub const NAME_MAX_CHARS: usize = 80;

/// The largest draft this route stores, measured as serde_json's compact
/// rendering.
///
/// The create form's own fields — a description, a few dozen facet slugs, four
/// thumbnail digests and a handful of numbers — are orders of magnitude under
/// this, so no template a seller writes meets it. What it stops is the other
/// use of an unbounded jsonb column any tenant can write: an upload channel.
///
/// Migration 0056 carries a CHECK for a writer that is not this route, and it
/// is deliberately a different, looser number measured on a different string.
/// Postgres renders `jsonb` back with a space after every `:` and every `,`, so
/// the column's `octet_length(draft::text)` is always at least as large as this
/// bound and the two must not be equal: a draft sitting exactly here would
/// otherwise pass the route and fail the constraint, which is a fault where an
/// answer belongs. The densest separator packing a JSON value admits is an
/// array of one-character elements, one comma per two bytes, so the column's
/// string is at most one and a half times this one — 98304 bytes — against a
/// CHECK of 131072.
///
/// That arithmetic holds only because [`holds_only_integers`] refuses a
/// fractional number; see there for what it would otherwise cost.
pub const DRAFT_MAX_BYTES: usize = 65_536;

fn storage_fault(state: &AppState, error: &tam_storage::StorageError) -> APIError {
    state.internal(&error.to_string())
}

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

fn missing() -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        APIErrorEntry::new("no such template")
            .code(APIErrorCode::ResourceMissing)
            .kind(APIErrorKind::NotFound),
    )
}

fn parse_id(raw: &str) -> Result<Uuid, APIError> {
    uuid::Uuid::parse_str(raw)
        .map(|parsed| Uuid(*parsed.as_bytes()))
        .map_err(|_| validation("the identifier is not a UUID"))
}

/// One template as the console reads it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceTemplateView {
    pub id: Uuid,
    pub name: String,
    /// The partial draft, in the shape the create form's own controls hold.
    pub draft: serde_json::Value,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ResourceTemplateView {
    fn of(record: ResourceTemplateRecord) -> Self {
        Self {
            id: record.id,
            name: record.name,
            draft: record.draft,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

/// One template as the picker lists it: everything but the draft.
///
/// The draft is absent rather than empty, because the listing is unpaged and
/// [`TEMPLATES_PER_ORG_MAX`] bounds only the row count: a shelf of drafts at
/// [`DRAFT_MAX_BYTES`] would make every picker open serialise several
/// megabytes. The console fetches the one the seller chose from [`get`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceTemplateHead {
    pub id: Uuid,
    pub name: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ResourceTemplateHead {
    fn of(summary: ResourceTemplateSummary) -> Self {
        Self {
            id: summary.id,
            name: summary.name,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
        }
    }
}

/// Every template this organisation has. Unpaged, because
/// [`TEMPLATES_PER_ORG_MAX`] bounds it and a picker that arrives in pages is
/// not a picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceTemplatesView {
    pub templates: Vec<ResourceTemplateHead>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    pub name: String,
    pub draft: serde_json::Value,
}

/// What an edit replaces. Both fields optional, and both absent is refused:
/// a request that changes nothing would still move `updated_at`, which is a
/// lie about when the seller last touched the template.
#[derive(Debug, Default, Deserialize)]
pub struct UpdateBody {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub draft: Option<serde_json::Value>,
}

/// The name as it will be stored, or the refusal a seller can act on.
///
/// Trimmed rather than refused for surrounding space, and measured after
/// trimming, so a name is bounded as it will be stored.
fn validated_name(raw: &str) -> Result<&str, APIError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(validation("a template needs a name"));
    }
    if trimmed.chars().count() > NAME_MAX_CHARS {
        return Err(validation(&format!(
            "a template name is at most {NAME_MAX_CHARS} characters"
        )));
    }
    // Before the column sees it. A zero byte passes both checks above -- trim
    // does not strip it and it counts as one character -- and no Postgres
    // `text` column can hold one, so without this the name arrives as a fault
    // rather than as the refusal it is.
    if !is_typed_text(trimmed) {
        return Err(validation(
            "a template name cannot contain control characters",
        ));
    }
    Ok(trimmed)
}

/// Keeps one of the create form's own refusals, rendered as the form renders
/// it, so the message and the numbers a seller reads cannot drift from the
/// form's.
fn refuse(error: &AuthoringError, entries: &mut Vec<APIErrorEntry>) {
    entries.push(entry_of(&refusal_of(error)));
}

/// One refusal in the shape the rest of this API reports one, keeping the
/// group and control the create form routes it by.
fn entry_of(refusal: &RefusalView) -> APIErrorEntry {
    let entry = APIErrorEntry::new(&refusal.message).kind(APIErrorKind::Validation);
    match serde_json::to_value(refusal) {
        Ok(detail) => entry.detail(detail),
        Err(_) => entry,
    }
}

/// What a template's draft may not hold.
///
/// The create form's own rules, applied to a draft that is deliberately
/// half-filled. A template exists to be finished later, so a field left blank
/// is never a refusal here — no title, no payload, no price, no tax code, no
/// grades and no copyright attestation are all ordinary states of a template.
/// A field it does hold is refused by exactly the rule the form would refuse
/// it by, which is what stops a template carrying a value the New resource
/// form would later reject.
///
/// Four layers, and only the last states a rule of its own. [`record_of`] is
/// the create path's own partial-tolerant validator: every listbox id against
/// its vocabulary, every upload handle against its digest shape, every facet
/// slug it stores. The smart constructors above it cover the fields
/// `record_of` skips — the title's length, the grades, and the payload and
/// preview handles, which the sidecar does not hold.
///
/// The stated-price floor is borrowed rather than restated too, through
/// [`stated_price_refusal`], which exists in `tam-authoring` for this caller:
/// `verdict` applies that rule alongside two absence rules a template is
/// exempt from, and the three cannot be separated from outside that crate.
///
/// The last is the selection caps, and it is a restatement of two
/// comparisons in `TptBaseProduct::check`. It has to be: that checker takes a
/// whole product, requiring a title and a payload by type, and
/// `tam_authoring::verdict` therefore never reaches it for a draft missing
/// either — which a template usually is. Fabricating a title and a digest to
/// get past the type would be a larger and less honest restatement than these
/// two lines. What is not restated is what a seller reads: the caps come from
/// [`selection_caps`], the refusals are `AuthoringError` values, and
/// [`refusal_of`] renders them, so the message and the numbers cannot drift
/// from the form's.
fn refusals(draft: &DraftInput) -> Result<(), APIError> {
    let mut entries: Vec<APIErrorEntry> = Vec::new();

    // A blank title is a template waiting to be finished; a title that is
    // present and over the form's cap is one the form would refuse.
    if !draft.name.trim().is_empty() {
        if let Err(error) = ProductName::new(&draft.name) {
            refuse(&error, &mut entries);
        }
    }
    for slug in &draft.grades {
        if let Err(error) = FacetSlug::new(slug) {
            refuse(&error, &mut entries);
        }
    }
    for hash in [draft.payload_hash.as_deref(), draft.preview_hash.as_deref()]
        .into_iter()
        .flatten()
    {
        if let Err(error) = UploadRef::new(hash) {
            refuse(&error, &mut entries);
        }
    }

    let record = match record_of(draft) {
        Ok(record) => Some(record),
        Err(refused) => {
            entries.extend(refused.errors);
            None
        }
    };

    // The other half of `verdict`, which `record_of` does not reach:
    // `draft_refusals`. Two of its three rules are absences a template is
    // exempt from — a blank price and a blank tax code — and the third is a
    // price the seller typed below TPT's own floor, which `POST
    // /{version}/products` refuses through `refuse_unsubmittable`. Borrowed
    // whole from `tam-authoring` rather than restated, sentence and floor
    // together.
    if let Some(refusal) = stated_price_refusal(draft) {
        entries.push(entry_of(&refusal));
    }

    if let Some(record) = record {
        let caps = selection_caps();
        let counted = [
            (Picker::Grades, draft.grades.len(), caps.grades),
            (
                Picker::SubjectAreas,
                record.categories.subject_areas.len(),
                caps.subject_areas,
            ),
            (Picker::Tags, record.categories.tags.len(), caps.tags),
            (
                Picker::Formats,
                record.categories.formats.len(),
                caps.formats,
            ),
            (Picker::Thumbnails, record.thumbnails.len(), caps.thumbnails),
        ];
        for (picker, chosen, cap) in counted {
            // An unmeasured cap refuses nothing: we hold no number to measure
            // the selection against, and inventing one would refuse a set TPT
            // itself accepts.
            if let Some(cap) = cap {
                if chosen > cap {
                    refuse(
                        &AuthoringError::OverCap {
                            picker,
                            chosen,
                            cap,
                        },
                        &mut entries,
                    );
                }
            }
        }
        if !record.thumbnails.is_empty() && record.thumbnail_mode != ThumbnailMode::UploadNow {
            refuse(
                &AuthoringError::ThumbnailsWithoutUploadNow {
                    mode: record.thumbnail_mode,
                },
                &mut entries,
            );
        }
    }

    if entries.is_empty() {
        Ok(())
    } else {
        Err(APIError::with_entries(
            StatusCode::UNPROCESSABLE_ENTITY,
            entries,
        ))
    }
}

/// Whether a zero byte sits anywhere in this document, in a string or in a
/// key.
///
/// Postgres refuses `\u0000` inside a `jsonb` value, and serde_json renders a
/// zero byte as exactly that escape, so a draft carrying one arrives at the
/// column as a fault rather than as the answer below. Searched over the parsed
/// document rather than over its rendering, because a string whose own text is
/// the six characters `\u0000` renders indistinguishably from the escape and
/// would otherwise be refused for a byte it does not hold.
///
/// Iterative rather than recursive: serde_json bounds parse depth, but a
/// walker that cannot overflow needs no reader to check that it still does.
fn holds_zero_byte(value: &serde_json::Value) -> bool {
    let mut pending = vec![value];
    while let Some(held) = pending.pop() {
        match held {
            serde_json::Value::String(text) => {
                if text.contains('\0') {
                    return true;
                }
            }
            serde_json::Value::Array(items) => pending.extend(items),
            serde_json::Value::Object(fields) => {
                for (key, field) in fields {
                    if key.contains('\0') {
                        return true;
                    }
                    pending.push(field);
                }
            }
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            }
        }
    }
    false
}

/// Whether every number in this document is one Postgres will render back at
/// the same length.
///
/// The reason is the column's byte CHECK, and it is the only thing that makes
/// [`DRAFT_MAX_BYTES`]'s arithmetic sound. `jsonb` stores a number as
/// `numeric` and renders it in full, while serde_json renders an `f64` through
/// its shortest round-trip form: `1e+308` is six bytes here and three hundred
/// and nine there. A draft padded with nine thousand of those measures 65529
/// bytes compact and 2910969 bytes as `draft::text` — measured, not reasoned —
/// so no fixed multiple of the compact bound could be a backstop while a
/// fractional number is admissible.
///
/// Refusing them costs nothing a seller can notice: every number the create
/// form holds is an integer — the prices in minor units, the page count, the
/// listbox ids, the standards framework and node ids — so a draft carrying a
/// fraction is not a draft the form could have produced. An integer serde_json
/// holds as `i64` or `u64` renders identically on both sides, which is what
/// leaves separators as the only expansion left to bound.
///
/// The walk is the shape [`holds_zero_byte`] uses, and for the same reason.
fn holds_only_integers(value: &serde_json::Value) -> bool {
    let mut pending = vec![value];
    while let Some(held) = pending.pop() {
        match held {
            serde_json::Value::Number(number) => {
                if !number.is_i64() && !number.is_u64() {
                    return false;
                }
            }
            serde_json::Value::Array(items) => pending.extend(items),
            serde_json::Value::Object(fields) => pending.extend(fields.values()),
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::String(_) => {
            }
        }
    }
    true
}

/// The draft as it will be stored, having passed everything the create form
/// would refuse it by.
fn validated_draft(draft: &serde_json::Value) -> Result<(), APIError> {
    if !draft.is_object() {
        return Err(validation(
            "a draft is a JSON object of the create form's own fields",
        ));
    }
    let rendered = serde_json::to_string(draft)
        .map_err(|_| validation("the draft is not JSON this server can store"))?;
    if rendered.len() > DRAFT_MAX_BYTES {
        return Err(validation(&format!(
            "a draft is at most {DRAFT_MAX_BYTES} bytes of JSON, and this one is {}",
            rendered.len()
        )));
    }
    if holds_zero_byte(draft) {
        return Err(validation("a draft cannot contain a zero byte"));
    }
    if !holds_only_integers(draft) {
        return Err(validation(
            "a draft holds whole numbers only; the create form has no fractional field",
        ));
    }
    let read: DraftInput = serde_json::from_value(draft.clone()).map_err(|error| {
        validation(&format!(
            "the draft is not a shape the create form holds: {error}"
        ))
    })?;
    refusals(&read)
}

/// Every template this organisation has, in the order the picker shows them,
/// without their drafts.
///
/// Whole rather than paged, because one organisation holds at most
/// [`TEMPLATES_PER_ORG_MAX`] of them. The draft is fetched by [`get`] once the
/// seller has chosen one: the row count alone does not bound a response whose
/// every row may carry [`DRAFT_MAX_BYTES`], and a picker opened repeatedly is
/// the cheapest read on this surface to make expensive.
pub(crate) async fn list(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<ResourceTemplatesView>, APIError> {
    let summaries = ResourceTemplateRepo::new(state.pool.clone())
        .list(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(ResourceTemplatesView {
        templates: summaries
            .into_iter()
            .map(ResourceTemplateHead::of)
            .collect(),
    }))
}

/// One template whole, with the draft the New resource form is prefilled from.
///
/// A template of another organisation answers not-found rather than forbidden,
/// because the identifier is the one thing a caller could guess and the two
/// answers would tell them which guesses were right.
pub(crate) async fn get(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, template)): Path<(String, String)>,
) -> Result<Json<ResourceTemplateView>, APIError> {
    let id = parse_id(&template)?;
    let record = ResourceTemplateRepo::new(state.pool.clone())
        .get(context.org, id)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(missing)?;
    Ok(Json(ResourceTemplateView::of(record)))
}

/// Saves one new template, answering it as stored rather than as sent: the
/// name is trimmed on the way in, so a client echoing its own body would
/// render a value this row does not hold.
///
/// The bounds, every one of them a validation refusal carrying a sentence the
/// form can render. The name is 1 to [`NAME_MAX_CHARS`] characters after
/// trimming and may carry no control character; one organisation may hold only
/// one template of a name, compared case-insensitively; the draft is a JSON
/// object of at most [`DRAFT_MAX_BYTES`] bytes carrying no zero byte, must
/// deserialise as the create form's own draft, and may hold no value that form
/// would refuse — see [`refusals`] for which of the form's rules a
/// deliberately half-filled draft is exempt from. One organisation may hold at
/// most [`TEMPLATES_PER_ORG_MAX`] templates in all, which is what bounds the
/// unpaged listing above.
pub(crate) async fn create(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<CreateBody>,
) -> Result<(StatusCode, Json<ResourceTemplateView>), APIError> {
    let name = validated_name(&body.name)?;
    validated_draft(&body.draft)?;
    let written = ResourceTemplateRepo::new(state.pool.clone())
        .create(
            context.org,
            &NewResourceTemplate {
                id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                name,
                draft: &body.draft,
                created_at: (state.wall)(),
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    match written {
        TemplateWrite::Saved(record) => {
            Ok((StatusCode::CREATED, Json(ResourceTemplateView::of(record))))
        }
        // Validation rather than a conflict status, for the reason every other
        // refusal on this surface is: the client renders the sentence, and
        // `APIErrorKind` has no conflict member a fourth status code would not
        // duplicate.
        TemplateWrite::NameTaken => Err(validation("you already have a template of that name")),
        TemplateWrite::TooMany => Err(validation(&format!(
            "you have {TEMPLATES_PER_ORG_MAX} templates already; \
             remove one before saving another"
        ))),
    }
}

/// Replaces a template's name, its draft, or both.
///
/// A part the request leaves out keeps the value it has. Both left out is
/// refused rather than accepted as a no-op, because the write would still move
/// `updated_at` and misstate when the seller last touched the template. The
/// draft is replaced whole rather than merged: the console renders every field
/// and sends them all back, so a merge would make clearing one impossible to
/// express.
///
/// Every bound [`create`] applies holds here too, save the per-organisation
/// ceiling, which an edit cannot cross because it adds no row. A name already
/// held by another of this organisation's templates is refused; a name this
/// same template already holds, in any casing, is not.
pub(crate) async fn update(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, template)): Path<(String, String)>,
    Json(body): Json<UpdateBody>,
) -> Result<Json<ResourceTemplateView>, APIError> {
    let id = parse_id(&template)?;
    let name = match body.name.as_deref() {
        Some(raw) => Some(validated_name(raw)?),
        None => None,
    };
    if let Some(draft) = body.draft.as_ref() {
        validated_draft(draft)?;
    }
    // The refusal is the type's, not a guard beside it: `TemplateEdit` has no
    // empty inhabitant, so an edit naming neither part cannot be built and
    // cannot reach a statement that would move `updated_at` for nothing.
    let edit = TemplateEdit::of(name, body.draft.as_ref())
        .ok_or_else(|| validation("an edit names a new name, a new draft, or both"))?;
    let written = ResourceTemplateRepo::new(state.pool.clone())
        .update(context.org, id, &edit, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    match written {
        TemplateChange::Saved(record) => Ok(Json(ResourceTemplateView::of(record))),
        TemplateChange::Missing => Err(missing()),
        TemplateChange::NameTaken => Err(validation("you already have a template of that name")),
    }
}

/// Removes one template.
///
/// Nothing references it, so nothing else changes: a template prefills a form,
/// and the resource that form created carries no trace of which template
/// filled it. A second delete of the same identifier answers not-found rather
/// than a success that did nothing.
pub(crate) async fn delete(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, template)): Path<(String, String)>,
) -> Result<StatusCode, APIError> {
    let id = parse_id(&template)?;
    let removed = ResourceTemplateRepo::new(state.pool.clone())
        .delete(context.org, id)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(missing())
    }
}

#[cfg(test)]
mod tests {
    use super::{validated_draft, validated_name, DRAFT_MAX_BYTES, NAME_MAX_CHARS};
    use crate::error::APIErrorKind;
    use axum::http::StatusCode;

    fn refused(draft: &serde_json::Value) -> Vec<String> {
        let error = validated_draft(draft).expect_err("this draft is refused");
        assert_eq!(
            error.status_code(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "a draft the form would refuse is the caller's error rather than a fault"
        );
        for entry in &error.errors {
            assert_eq!(
                entry.kind,
                Some(APIErrorKind::Validation),
                "every refusal carries the kind the client branches on"
            );
        }
        error
            .errors
            .iter()
            .map(|entry| entry.message.clone())
            .collect()
    }

    #[test]
    fn a_name_is_trimmed_bounded_and_free_of_control_characters() {
        assert_eq!(
            validated_name("  Autumn unit \n").ok(),
            Some("Autumn unit"),
            "the stored name is the trimmed one"
        );
        assert!(
            validated_name("   ").is_err(),
            "a name that is only whitespace is no name"
        );
        // Two bytes per character, so a byte-counting bound would refuse the
        // name a character-counting one accepts.
        let longest = "\u{e9}".repeat(NAME_MAX_CHARS);
        assert!(
            validated_name(&format!(" {longest} ")).is_ok(),
            "{NAME_MAX_CHARS} characters is accepted, and surrounding space is not counted"
        );
        assert!(
            validated_name(&"\u{e9}".repeat(NAME_MAX_CHARS + 1)).is_err(),
            "one character past the bound is refused"
        );
        assert!(
            validated_name("Autumn\u{0}unit").is_err(),
            "a zero byte no text column can hold is refused here rather than by Postgres"
        );
    }

    /// The whole point of the tab: a template is a form nobody has finished.
    #[test]
    fn a_draft_that_states_almost_nothing_is_accepted() {
        assert!(
            validated_draft(&serde_json::json!({})).is_ok(),
            "an empty draft is the blank template a seller starts from"
        );
        assert!(
            validated_draft(&serde_json::json!({
                "description": "Differentiated three ways.\n\n- Easy\n- Medium",
                "grades": ["1st-grade"],
                "free": false,
            }))
            .is_ok(),
            "no title, no payload, no price and no copyright attestation are all \
             ordinary states of a template, and a description carries newlines"
        );
    }

    #[test]
    fn a_value_the_create_form_would_refuse_is_refused_here() {
        assert!(
            !refused(&serde_json::json!({ "name": "t".repeat(400) })).is_empty(),
            "a title over the form's own cap is a value, not an absence"
        );
        assert!(
            !refused(&serde_json::json!({ "payload_hash": "not-a-digest" })).is_empty(),
            "an upload handle this server did not issue is refused"
        );
        assert!(
            !refused(&serde_json::json!({ "grades": [" "] })).is_empty(),
            "a blank facet slug is refused, and grades are the one facet the sidecar skips"
        );
        assert!(
            !refused(&serde_json::json!({ "tax_code_id": 200 })).is_empty(),
            "a listbox id outside its vocabulary is refused"
        );
        assert!(
            !refused(&serde_json::json!({ "answer_key_id": 99 })).is_empty(),
            "and so is one in a vocabulary whose ids are not its menu positions"
        );
    }

    /// The rule `tam_authoring::verdict` cannot reach for a partial draft,
    /// which is why this module states it rather than borrowing it.
    #[test]
    fn a_picker_over_its_measured_cap_is_refused_without_a_title_or_a_payload() {
        let messages = refused(&serde_json::json!({
            "grades": ["1st-grade", "2nd-grade", "3rd-grade", "4th-grade", "5th-grade"],
        }));
        assert!(
            messages
                .iter()
                .any(|message| message.contains("Grade Level")),
            "the refusal names the control the seller is looking at: {messages:?}"
        );
    }

    #[test]
    fn thumbnails_attached_under_a_mode_with_no_slots_are_refused() {
        let digest = "0".repeat(64);
        let messages = refused(&serde_json::json!({
            "thumbnail_mode": 1,
            "thumbnail_hashes": [digest],
        }));
        assert!(
            !messages.is_empty(),
            "a file collected under a mode the form renders no slots for is a value it refuses"
        );
    }

    #[test]
    fn a_draft_that_is_not_the_form_s_own_shape_is_refused() {
        for wrong in [
            serde_json::json!([]),
            serde_json::json!("a draft"),
            serde_json::json!(7),
            serde_json::Value::Null,
        ] {
            assert!(
                validated_draft(&wrong).is_err(),
                "a draft is a JSON object of the form's fields: {wrong}"
            );
        }
        assert!(
            validated_draft(&serde_json::json!({ "grades": "1st-grade" })).is_err(),
            "a field of the wrong type is refused before the column sees it"
        );
    }

    /// The rule the create route enforces through `refuse_unsubmittable` and
    /// this one has to enforce identically: a price the seller typed below
    /// TPT's floor is a value, not an absence, and a template holding one
    /// prefills a form that cannot submit.
    #[test]
    fn a_stated_price_below_the_floor_is_refused_in_the_form_s_own_words() {
        for stated in [1, 94, -100] {
            let messages = refused(&serde_json::json!({
                "free": false,
                "price_minor_units": stated,
            }));
            assert!(
                messages
                    .iter()
                    .any(|message| message == "Raise the price to at least $0.95."),
                "the sentence is the create form's own, borrowed rather than retyped, \
                 for a stated price of {stated}: {messages:?}"
            );
        }
        assert!(
            validated_draft(&serde_json::json!({ "free": false, "price_minor_units": 95 })).is_ok(),
            "a price on the floor clears it"
        );
    }

    /// The absences the same rule fires on for a submission, which a template
    /// is exempt from. Without these the fix above would refuse every template
    /// that has not reached the price control yet.
    #[test]
    fn an_unstated_price_is_not_a_refusal_for_a_template() {
        for ordinary in [
            serde_json::json!({ "free": false }),
            serde_json::json!({}),
            serde_json::json!({ "free": true }),
            // A paid draft with a price and no tax code: `verdict` refuses this
            // for a submission and a template is not one.
            serde_json::json!({ "free": false, "price_minor_units": 500 }),
        ] {
            assert!(
                validated_draft(&ordinary).is_ok(),
                "a price the seller has not typed yet is an ordinary state of a \
                 half-filled form: {ordinary}"
            );
        }
    }

    /// The guard that makes the price rule a rule about a paid draft.
    ///
    /// Missed by every test above: deleting `if draft.free` from
    /// `stated_price_refusal` leaves them all passing, because the free drafts
    /// they use state no price at all and the rule then stops at the absent
    /// amount instead of at the guard.
    #[test]
    fn a_free_draft_is_never_measured_against_the_paid_floor() {
        for free in [
            serde_json::json!({ "free": true, "price_minor_units": 1 }),
            serde_json::json!({ "free": true, "price_minor_units": 0 }),
            serde_json::json!({ "free": true, "price_minor_units": -100 }),
        ] {
            assert!(
                validated_draft(&free).is_ok(),
                "a free listing has no price to be below a floor, and the create form \
                 reads it as free whatever the amount control still holds: {free}"
            );
        }
    }

    /// The boundary M2 lived on: the route's bound and the column's CHECK
    /// measure different strings, so a draft sitting exactly on the route's
    /// bound must still be one the column accepts.
    #[test]
    fn the_byte_ceiling_is_exact_at_its_own_boundary() {
        // `{"description":""}` is eighteen bytes of wrapper.
        let filling =
            |compact: usize| serde_json::json!({ "description": "a".repeat(compact - 18) });
        let on_the_cap = filling(DRAFT_MAX_BYTES);
        assert_eq!(
            serde_json::to_string(&on_the_cap)
                .map(|rendered| rendered.len())
                .ok(),
            Some(DRAFT_MAX_BYTES),
            "the fixture renders to exactly the cap, or this test is measuring \
             something else"
        );
        assert!(
            validated_draft(&on_the_cap).is_ok(),
            "a draft exactly on the cap is accepted, and migration 0056's own \
             backstop is loose enough to hold it once Postgres re-renders it"
        );
        let past = filling(DRAFT_MAX_BYTES + 1);
        assert!(
            validated_draft(&past).is_err(),
            "and one byte past it is refused"
        );
    }

    /// What makes the column's backstop a fixed multiple of the route's bound
    /// rather than an unbounded one.
    #[test]
    fn a_fractional_number_is_refused_however_it_is_hidden() {
        for carrying in [
            serde_json::json!({ "pages_or_slides": 1.5 }),
            serde_json::json!({ "pad": 1e308 }),
            serde_json::json!({ "pad": [1, 2, 1e308] }),
            serde_json::json!({ "pad": { "nested": 0.1 } }),
        ] {
            assert!(
                validated_draft(&carrying).is_err(),
                "Postgres renders a numeric in full where serde_json renders an f64 \
                 shortest, so a fraction breaks the ceiling's arithmetic: {carrying}"
            );
        }
        assert!(
            validated_draft(&serde_json::json!({
                "free": false,
                "price_minor_units": 500,
                "pages_or_slides": 12,
                "tax_code_id": 1,
                "pad": [1, 2, 3],
            }))
            .is_ok(),
            "every number the create form actually holds is a whole one, at any depth"
        );
    }

    #[test]
    fn a_draft_past_the_byte_ceiling_is_refused() {
        let huge = serde_json::json!({ "description": "a".repeat(DRAFT_MAX_BYTES) });
        assert!(
            validated_draft(&huge).is_err(),
            "an unbounded jsonb column any tenant can write is an upload channel"
        );
    }

    #[test]
    fn a_zero_byte_anywhere_in_a_draft_is_refused() {
        for carrying in [
            serde_json::json!({ "description": "before\u{0}after" }),
            serde_json::json!({ "grades": ["1st-grade", "2nd\u{0}grade"] }),
            serde_json::json!({ "before\u{0}after": 1 }),
        ] {
            assert!(
                validated_draft(&carrying).is_err(),
                "Postgres refuses a zero byte inside jsonb, so it must arrive as an \
                 answer rather than as a fault: {carrying}"
            );
        }
        assert!(
            validated_draft(&serde_json::json!({ "description": "escaped as \\u0000" })).is_ok(),
            "a description whose own text spells the escape holds no zero byte"
        );
    }
}
