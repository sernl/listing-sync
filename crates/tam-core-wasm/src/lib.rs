//! The core, in the browser.
//!
//! Glue and nothing else. Every rule this module answers with is decided by
//! `tam-authoring` and `tam-domain`, which the server calls through the same
//! functions, so the answer a seller reads as they type is the answer
//! `POST /v1/authoring/check` would give. A rule stated here rather than
//! called would be a third copy of it, which is the duplication this crate
//! exists to remove (D28).
//!
//! The boundary is JSON strings in both directions. `serde-wasm-bindgen` would
//! type the crossing more finely and is deliberately not used: a string has
//! one encoding to agree on, and the strings crossing here are the same ones
//! the HTTP endpoint already carries, so the browser and the server are
//! reading one wire format rather than two.
//!
//! Nothing here performs I/O and nothing panics across the boundary. Every
//! function returns a JSON string; a failure is the error shape documented on
//! [`error`] rather than a trap, because a trap poisons the module instance
//! and takes every later call with it.

use serde::{Deserialize, Serialize};
use tam_authoring::{verdict_with, DraftInput};
use tam_domain::product::SelectionCaps;
use tam_domain::registry::{registry, truncate, FieldSpec, LengthCap};
use tam_types::InventoryId;
use tam_types::LengthUnit;
use wasm_bindgen::prelude::wasm_bindgen;

// ------------------------------------------------------------ the failures

/// The error shape, as a JSON string.
///
/// `{"error":{"kind":"...","message":"..."}}`, where `kind` is one of
/// `malformed_draft`, `malformed_caps`, `unknown_marketplace` or
/// `encode_failed`. A distinct top-level key rather than a verdict variant: a
/// verdict with no refusals means the draft is submittable, so a boundary
/// failure rendered as one would read as approval.
///
/// The message is quoted by the encoder rather than escaped here, because
/// hand-written JSON escaping is a second implementation of a thing that has
/// to be exactly right; serialising a `&str` has no failure path, so the
/// fallback below is unreachable and exists only because `expect` is denied.
fn error(kind: &'static str, message: &str) -> String {
    let quoted = serde_json::to_string(message).unwrap_or_else(|_| String::from(r#""""#));
    format!(r#"{{"error":{{"kind":"{kind}","message":{quoted}}}}}"#)
}

/// A value as JSON, or the error shape where it could not be encoded.
///
/// Encoding cannot fail for any type this module returns — no map with a
/// non-string key and no non-finite float reaches one — so the fallback is
/// unreachable in practice and is written anyway, because the alternative to
/// writing it is `expect`, and a panic here takes the whole module instance
/// down rather than one call.
fn encoded<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|failure| error("encode_failed", &failure.to_string()))
}

// -------------------------------------------------------------- the caps

/// The measured caps as a caller may state them.
///
/// Mirrors [`SelectionCaps`] on the wire, which carries no `serde` derive of
/// its own because `tam-domain` holds no serialisation. A field that is absent
/// or null is unmeasured, never unlimited, exactly as the model reads it.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
struct CapsInput {
    #[serde(default)]
    grades: Option<usize>,
    #[serde(default)]
    subject_areas: Option<usize>,
    #[serde(default)]
    tags: Option<usize>,
    #[serde(default)]
    formats: Option<usize>,
    #[serde(default)]
    thumbnails: Option<usize>,
}

impl From<CapsInput> for SelectionCaps {
    fn from(input: CapsInput) -> Self {
        Self {
            grades: input.grades,
            subject_areas: input.subject_areas,
            tags: input.tags,
            formats: input.formats,
            thumbnails: input.thumbnails,
        }
    }
}

// ------------------------------------------------------------- the exports

/// The version of the core the page is running, so a stale bundle is
/// identifiable rather than merely suspected.
#[must_use]
#[wasm_bindgen]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// The caps the committed capture states, as JSON.
///
/// The same numbers `GET /v1/authoring/vocabulary` serves, read from the same
/// compiled-in capture rather than fetched, so a counter beside a picker reads
/// the number the check will measure against even before the page has loaded
/// its vocabulary.
#[must_use]
#[wasm_bindgen]
pub fn selection_caps() -> String {
    let caps = tam_authoring::selection_caps();
    encoded(&CapsView {
        grades: caps.grades,
        subject_areas: caps.subject_areas,
        tags: caps.tags,
        formats: caps.formats,
        thumbnails: caps.thumbnails,
    })
}

#[derive(Debug, Clone, Copy, Serialize)]
struct CapsView {
    grades: Option<usize>,
    subject_areas: Option<usize>,
    tags: Option<usize>,
    formats: Option<usize>,
    thumbnails: Option<usize>,
}

/// What the form refuses, decided by the same function the server calls.
///
/// `draft_json` is the body of `POST /v1/authoring/check`, and the answer is
/// that endpoint's own `CheckView`, byte for byte. `caps_json` is `null` on
/// every production path, which reads the compiled-in capture and so cannot
/// disagree with the server; a caller that states caps is a test holding them
/// fixed.
#[must_use]
#[wasm_bindgen]
#[expect(
    clippy::needless_pass_by_value,
    reason = "wasm-bindgen's ABI has no Option<&str>: OptionFromWasmAbi is implemented for String"
)]
pub fn check_draft(draft_json: &str, caps_json: Option<String>) -> String {
    let draft: DraftInput = match serde_json::from_str(draft_json) {
        Ok(draft) => draft,
        Err(failure) => return error("malformed_draft", &failure.to_string()),
    };
    let caps = match caps_json.as_deref() {
        None | Some("null" | "") => tam_authoring::selection_caps(),
        Some(stated) => match serde_json::from_str::<CapsInput>(stated) {
            Ok(caps) => caps.into(),
            Err(failure) => return error("malformed_caps", &failure.to_string()),
        },
    };
    encoded(&verdict_with(&draft, caps))
}

// -------------------------------------------------------- the projection

/// One canonical field as one marketplace will carry it.
#[derive(Debug, Clone, Serialize)]
struct ProjectedRow {
    /// `title`, `description` or `price`.
    key: &'static str,
    label: &'static str,
    /// What this marketplace carries, after any cap it declares has been
    /// applied. The projected value rather than the authored one, because the
    /// authored one is what the seller is already looking at.
    value: String,
    /// The declared bound, where one is on file. Absent is unmeasured, never
    /// unlimited.
    cap: Option<CapView>,
    /// True where the platform is documented or measured to refuse a create
    /// without the field. False records no such finding, which is not evidence
    /// that the field is optional.
    required: bool,
    /// What this platform drops, where the cap actually bites. Null is
    /// "nothing dropped", and it is decided by comparing the projected value
    /// with the authored one rather than by predicting it.
    loss: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct CapView {
    limit: usize,
    /// `bytes`, `utf16_code_units`, `codepoints` or `grapheme_clusters`.
    unit: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct ProjectionView {
    inventory: InventoryId,
    rows: Vec<ProjectedRow>,
    /// The axes this preview does not decide, named so the page says so rather
    /// than implying the list is complete. The equivalence relation lives in
    /// the database, and a client that guessed at it would be inventing a
    /// mapping rather than reading one.
    undecided_axes: Vec<&'static str>,
}

const fn unit_token(unit: LengthUnit) -> &'static str {
    match unit {
        LengthUnit::Bytes => "bytes",
        LengthUnit::Utf16CodeUnits => "utf16_code_units",
        LengthUnit::Codepoints => "codepoints",
        LengthUnit::GraphemeClusters => "grapheme_clusters",
    }
}

/// One field's row: the value this platform carries and what it drops.
fn row(key: &'static str, label: &'static str, authored: &str, spec: FieldSpec) -> ProjectedRow {
    let (value, loss) = match spec.cap {
        None => (authored.to_owned(), None),
        Some(cap) => project_text(authored, cap),
    };
    ProjectedRow {
        key,
        label,
        value,
        cap: spec.cap.map(|LengthCap { limit, unit }| CapView {
            limit,
            unit: unit_token(unit),
        }),
        required: spec.required,
        loss,
    }
}

/// The text as the platform will hold it, and what fell off.
///
/// The loss is measured from the two strings rather than predicted from the
/// cap, so a cap whose unit counts something other than characters reports
/// what was actually dropped instead of an arithmetic guess.
fn project_text(authored: &str, cap: LengthCap) -> (String, Option<String>) {
    let kept = truncate(authored, cap);
    if kept.len() == authored.len() {
        return (kept, None);
    }
    let dropped = authored.chars().count() - kept.chars().count();
    let loss = format!(
        "{} of {} characters are dropped: this field takes {} {}.",
        dropped,
        authored.chars().count(),
        cap.limit,
        unit_token(cap.unit).replace('_', " ")
    );
    (kept, Some(loss))
}

/// What one marketplace will carry for the fields a draft states, decided from
/// the compiled-in field registry.
///
/// Scoped to the three fields a draft holds text for, because those are the
/// ones the registry can decide without the database. The equivalence axes —
/// subject, topic, phase, licence — resolve over a relation that lives in
/// Postgres, and they are named in `undecided_axes` rather than guessed at: a
/// projection invented in the browser would be a mapping nobody recorded.
#[must_use]
#[wasm_bindgen]
pub fn project_preview(draft_json: &str, marketplace: &str) -> String {
    let draft: DraftInput = match serde_json::from_str(draft_json) {
        Ok(draft) => draft,
        Err(failure) => return error("malformed_draft", &failure.to_string()),
    };
    let Ok(inventory) = serde_json::from_str::<InventoryId>(&format!("\"{marketplace}\"")) else {
        return error(
            "unknown_marketplace",
            &format!("{marketplace} is not a marketplace this build knows"),
        );
    };
    let held = registry(inventory);
    let price = if draft.free {
        "Free".to_owned()
    } else {
        draft
            .price_minor_units
            .map_or_else(String::new, |minor| format!("{minor} minor units"))
    };
    encoded(&ProjectionView {
        inventory,
        rows: vec![
            row("title", "Title", &draft.name, held.canonical.title),
            row(
                "description",
                "Description",
                &draft.description,
                held.canonical.description,
            ),
            row("price", "Price", &price, held.canonical.price),
        ],
        undecided_axes: held
            .equivalence_axes
            .iter()
            .map(|binding| axis_token(binding.axis))
            .collect(),
    })
}

const fn axis_token(axis: tam_domain::TermKind) -> &'static str {
    match axis {
        tam_domain::TermKind::Subject => "subject",
        tam_domain::TermKind::Topic => "topic",
        tam_domain::TermKind::Phase => "phase",
        tam_domain::TermKind::Licence => "licence",
        tam_domain::TermKind::ResourceType => "resource_type",
    }
}
