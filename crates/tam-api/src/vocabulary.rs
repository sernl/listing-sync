//! One marketplace's authoring vocabulary, served as data so the create form
//! is driven by the registry rather than by a second copy of it in
//! TypeScript.
//!
//! Everything under `registry` is [`tam_domain::registry`] rendered onto the
//! wire and nothing else: the canonical field specs, the native fields with
//! their captured vocabularies, the equivalence axes each native carries, and
//! the axes an inventory was measured to lack. The three-way distinction the
//! registry draws between a captured closed set, a set documented closed
//! whose members are not held, and an unmeasured one survives the crossing,
//! because a form that rendered the second as a free-text box would submit
//! values the platform refuses.
//!
//! `authoring` carries the four facts the registry does not hold because they
//! are properties of an adapter's write rather than of a platform's field
//! table. Each is cited at its declaration. They live here rather than in the
//! pure core so that the registry stays what its own module doc says it is —
//! a table of measured field facts — and moving one down later is a
//! refactor with no wire change.

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_domain::registry::{
    registry, AxisBinding, Cardinality, CountCap, Delegation, FieldDirection, FieldSpec,
    InventoryRegistry, LengthCap, NativeField, NativeVocabulary, NonDelegable,
};
use tam_domain::TermKind;
use tam_types::{CopyFormat, FieldKey, InventoryId, LengthUnit, Marketplace};

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

// ------------------------------------------------------------- closed sets

/// Which way a native field crosses the adapter seam, on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectionView {
    Written,
    ReadOnly,
    Both,
}

impl DirectionView {
    pub const ALL: [Self; 3] = [Self::Written, Self::ReadOnly, Self::Both];

    const fn of(direction: FieldDirection) -> Self {
        match direction {
            FieldDirection::Written => Self::Written,
            FieldDirection::ReadOnly => Self::ReadOnly,
            FieldDirection::Both => Self::Both,
        }
    }
}

/// What is known about a native field's admissible values, as a tag the form
/// branches on. `closed` carries its members; `closed_uncaptured` does not,
/// and a form must render it as a value the seller supplies at their own risk
/// rather than as a select with no options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VocabularyKind {
    Closed,
    ClosedUncaptured,
    Free,
    Numeric,
    Unmeasured,
}

impl VocabularyKind {
    pub const ALL: [Self; 5] = [
        Self::Closed,
        Self::ClosedUncaptured,
        Self::Free,
        Self::Numeric,
        Self::Unmeasured,
    ];

    const fn of(vocabulary: NativeVocabulary) -> Self {
        match vocabulary {
            NativeVocabulary::Closed(_) => Self::Closed,
            NativeVocabulary::ClosedUncaptured => Self::ClosedUncaptured,
            NativeVocabulary::Free => Self::Free,
            NativeVocabulary::Numeric => Self::Numeric,
            NativeVocabulary::Unmeasured => Self::Unmeasured,
        }
    }
}

/// How many values one field takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardinalityKind {
    One,
    Many,
}

impl CardinalityKind {
    pub const ALL: [Self; 2] = [Self::One, Self::Many];
}

/// Whether the seller may hand this axis or field to a best-fit computation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationKind {
    ByOptIn,
    Never,
}

impl DelegationKind {
    pub const ALL: [Self; 2] = [Self::ByOptIn, Self::Never];
}

/// Why a field refuses delegation, so the control renders a reason rather
/// than a disabled box with no explanation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NonDelegableReason {
    LegalContent,
}

impl NonDelegableReason {
    pub const ALL: [Self; 1] = [Self::LegalContent];

    const fn of(reason: NonDelegable) -> Self {
        match reason {
            NonDelegable::LegalContent => Self::LegalContent,
        }
    }
}

/// How many payload files one create carries onto the wire.
///
/// Not in the registry because it is a property of the adapter's submit
/// rather than of a field: Tes uploads every payload file
/// (`crates/tam-marketplace-tes/src/flows.rs:635`) and TPT's create takes
/// exactly one into the product slot, refusing any other count
/// (`crates/tam-marketplace-tpt/src/flows.rs:634-646`). This is the fact that
/// makes an exploded multi-entry ZIP TES-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadFileRule {
    /// Every payload file the product holds is uploaded.
    EveryPayloadFile,
    /// Exactly one, and any other count is refused before a request is made.
    ExactlyOne,
}

impl PayloadFileRule {
    pub const ALL: [Self; 2] = [Self::EveryPayloadFile, Self::ExactlyOne];
}

/// What the platform's wire does with the body's declared format.
///
/// Tes carries the declaration through `descriptionRawType`
/// (`crates/tam-domain/src/registry/tes.rs`, the `descriptionRawType`
/// native); TPT's wire is HTML and the projection renders a Markdown body
/// into it (`crates/tam-marketplace-tpt/src/write_model.rs:822`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyWire {
    /// The declared format travels; the seller's choice of Markdown or HTML
    /// reaches the listing.
    CarriesDeclaredFormat,
    /// The wire is HTML and a Markdown body is rendered into it.
    RendersToHtml,
}

impl BodyWire {
    pub const ALL: [Self; 2] = [Self::CarriesDeclaredFormat, Self::RendersToHtml];
}

// ------------------------------------------------------------------- views

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VocabularyView {
    pub inventory: InventoryId,
    pub marketplace: Marketplace,
    pub canonical: Vec<CanonicalFieldView>,
    pub natives: Vec<NativeFieldView>,
    pub axes: Vec<AxisView>,
    /// Axes this inventory was measured to lack. An axis here is a disclosed
    /// loss the form states up front; an axis merely absent from `axes` is
    /// unmeasured and blocks at projection instead.
    pub absent_axes: Vec<TermKind>,
    pub authoring: AuthoringView,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CapView {
    pub limit: usize,
    pub unit: LengthUnit,
}

impl CapView {
    const fn of(cap: LengthCap) -> Self {
        Self {
            limit: cap.limit,
            unit: cap.unit,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalFieldView {
    /// `title`, `description`, `price`, `taxonomy`, `grades` or `files`.
    pub field: String,
    /// Absent means unmeasured, never unlimited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap: Option<CapView>,
    /// True only where a refusal is measured or documented. False records no
    /// such finding, which is not evidence the field is optional.
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationView {
    pub kind: DelegationKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<NonDelegableReason>,
}

impl DelegationView {
    const fn of(delegation: Delegation) -> Self {
        match delegation {
            Delegation::ByOptIn => Self {
                kind: DelegationKind::ByOptIn,
                reason: None,
            },
            Delegation::Never(reason) => Self {
                kind: DelegationKind::Never,
                reason: Some(NonDelegableReason::of(reason)),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeFieldView {
    pub name: String,
    pub direction: DirectionView,
    pub required: bool,
    pub vocabulary: VocabularyKind,
    /// The captured members, present only for `closed`. An empty array here
    /// would be a claim nobody made, so the field is absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
    pub delegation: DelegationView,
}

impl NativeFieldView {
    fn of(native: NativeField) -> Self {
        Self {
            name: native.name.to_owned(),
            direction: DirectionView::of(native.direction),
            required: native.required,
            vocabulary: VocabularyKind::of(native.vocabulary),
            values: match native.vocabulary {
                NativeVocabulary::Closed(values) => {
                    Some(values.iter().map(|value| (*value).to_owned()).collect())
                }
                NativeVocabulary::ClosedUncaptured
                | NativeVocabulary::Free
                | NativeVocabulary::Numeric
                | NativeVocabulary::Unmeasured => None,
            },
            delegation: DelegationView::of(native.delegation),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxisView {
    pub axis: TermKind,
    /// The native field this axis lands in, which is the key into `natives`.
    pub native: String,
    pub cardinality: CardinalityKind,
    /// The platform's own selection limit where one is measured. Absent means
    /// unmeasured, never unlimited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap: Option<usize>,
    pub delegation: DelegationView,
    /// Read off the native field beside it rather than restated, so the two
    /// declarations cannot disagree.
    pub required: bool,
}

impl AxisView {
    fn of(entry: &InventoryRegistry, binding: AxisBinding) -> Self {
        let (cardinality, cap) = match binding.cardinality {
            Cardinality::One => (CardinalityKind::One, None),
            Cardinality::Many { cap } => {
                (CardinalityKind::Many, cap.map(|CountCap { limit }| limit))
            }
        };
        Self {
            axis: binding.axis,
            native: binding.native.to_owned(),
            cardinality,
            cap,
            delegation: DelegationView::of(binding.delegation),
            required: entry
                .native(binding.native)
                .is_some_and(|native| native.required),
        }
    }
}

/// The write-side facts a form needs that the field registry does not record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthoringView {
    pub payload_files: PayloadFileRule,
    pub body_wire: BodyWire,
    /// The body formats a seller may author in. Both everywhere: TPT renders
    /// Markdown into its HTML wire rather than refusing it.
    pub body_formats: Vec<CopyFormat>,
    /// The lowest price the adapter will post, in the wire's own minor units.
    /// Absent where no floor is on file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_floor_minor_units: Option<i64>,
    /// The licence gate, where the platform has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub licence: Option<LicenceGateView>,
    /// A legal attestation the write carries, which the form displays and
    /// never re-asks because it is held per connection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestation: Option<AttestationView>,
}

/// Which licence values a create may carry, split by the branch the platform
/// gates the write on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenceGateView {
    /// The native field the choice lands in.
    pub native: String,
    /// The values a free listing may carry.
    pub free: Vec<String>,
    /// The values a paid listing may carry.
    pub paid: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationView {
    pub native: String,
    /// True where the statement is recorded against the marketplace
    /// connection, so the form displays which attestation the write will
    /// carry instead of asking for it again.
    pub held_per_connection: bool,
}

// --------------------------------------------------------------- authoring

/// Tes writes four of the seven licences its refdata store holds, and its API
/// gates the write on the price: a free resource carries one of the three
/// Creative Commons values and a paid one carries `TES-PAID`
/// (`crates/tam-domain/src/registry/tes.rs`, the `licence` native;
/// `crates/tam-marketplace-tes/src/endpoints.rs:29-46`, whose `TesLicence` is
/// the writable four). `TES-PAID-SCHOOL` and the two legacy values read back
/// and are never offered on write, so they are absent from both branches.
const TES_LICENCE_FREE: [&str; 3] = ["CC-BY", "CC-BY-ND", "CC-BY-SA"];
const TES_LICENCE_PAID: [&str; 1] = ["TES-PAID"];

/// TPT's create form's own floor, `min_price: 0.95` in the page bootstrap of
/// both captured renders, enforced before a request is made
/// (`crates/tam-marketplace-tpt/src/write_model.rs:106`).
const TPT_MIN_PRICE_MINOR_UNITS: i64 = 95;

/// The attestation TPT's create posts, held against the connection rather
/// than asked per product (`crates/tam-domain/src/registry/tpt.rs`, the
/// `ItemsProperty.copyright_declaration` native;
/// `crates/tam-storage/migrations/0031_connection_custody.sql:56`).
const TPT_ATTESTATION: &str = "ItemsProperty.copyright_declaration";

fn authoring(inventory: InventoryId) -> AuthoringView {
    match inventory {
        InventoryId::TesGb | InventoryId::TesUs | InventoryId::TesNz => AuthoringView {
            payload_files: PayloadFileRule::EveryPayloadFile,
            body_wire: BodyWire::CarriesDeclaredFormat,
            body_formats: CopyFormat::ALL.to_vec(),
            price_floor_minor_units: None,
            licence: Some(LicenceGateView {
                native: "licence".to_owned(),
                free: TES_LICENCE_FREE.iter().map(|v| (*v).to_owned()).collect(),
                paid: TES_LICENCE_PAID.iter().map(|v| (*v).to_owned()).collect(),
            }),
            attestation: None,
        },
        InventoryId::Tpt => AuthoringView {
            payload_files: PayloadFileRule::ExactlyOne,
            body_wire: BodyWire::RendersToHtml,
            body_formats: CopyFormat::ALL.to_vec(),
            price_floor_minor_units: Some(TPT_MIN_PRICE_MINOR_UNITS),
            licence: None,
            attestation: Some(AttestationView {
                native: TPT_ATTESTATION.to_owned(),
                held_per_connection: true,
            }),
        },
        // No Etsy adapter exists, so nothing is claimed about its write. The
        // registry's own entry is what this serves, and `authoring` states
        // the neutral shape rather than a guess.
        InventoryId::Etsy => AuthoringView {
            payload_files: PayloadFileRule::EveryPayloadFile,
            body_wire: BodyWire::CarriesDeclaredFormat,
            body_formats: CopyFormat::ALL.to_vec(),
            price_floor_minor_units: None,
            licence: None,
            attestation: None,
        },
    }
}

const fn field_name(key: FieldKey) -> &'static str {
    match key {
        FieldKey::Title => "title",
        FieldKey::Description => "description",
        FieldKey::Price => "price",
        FieldKey::Taxonomy => "taxonomy",
        FieldKey::Grades => "grades",
        FieldKey::Files => "files",
    }
}

const CANONICAL_KEYS: [FieldKey; 6] = [
    FieldKey::Title,
    FieldKey::Description,
    FieldKey::Price,
    FieldKey::Taxonomy,
    FieldKey::Grades,
    FieldKey::Files,
];

/// The whole payload for one inventory, assembled from the registry.
#[must_use]
pub fn view(inventory: InventoryId) -> VocabularyView {
    let entry = registry(inventory);
    VocabularyView {
        inventory,
        marketplace: inventory.marketplace(),
        canonical: CANONICAL_KEYS
            .into_iter()
            .map(|key| {
                let FieldSpec { cap, required } = *entry.canonical.get(key);
                CanonicalFieldView {
                    field: field_name(key).to_owned(),
                    cap: cap.map(CapView::of),
                    required,
                }
            })
            .collect(),
        natives: entry
            .natives
            .iter()
            .copied()
            .map(NativeFieldView::of)
            .collect(),
        axes: entry
            .equivalence_axes
            .iter()
            .copied()
            .map(|binding| AxisView::of(entry, binding))
            .collect(),
        absent_axes: entry.absent_axes.iter().map(|absent| absent.0).collect(),
        authoring: authoring(inventory),
    }
}

/// The wire spelling of an inventory in a path segment: the same token the
/// JSON bodies carry, so a client holds one vocabulary rather than two.
fn parse_inventory(raw: &str) -> Option<InventoryId> {
    InventoryId::ALL
        .into_iter()
        .find(|inventory| serde_json::to_value(inventory).ok() == Some(raw.into()))
}

pub(crate) async fn vocabulary_view(
    State(_state): State<AppState>,
    _context: OrgContext,
    Path((_version, inventory)): Path<(String, String)>,
) -> Result<Json<VocabularyView>, APIError> {
    let inventory = parse_inventory(&inventory).ok_or_else(|| {
        APIError::new(
            axum::http::StatusCode::NOT_FOUND,
            APIErrorEntry::new("no such inventory")
                .code(APIErrorCode::ResourceMissing)
                .kind(APIErrorKind::NotFound),
        )
    })?;
    Ok(Json(view(inventory)))
}

#[cfg(test)]
mod tests {
    use super::{
        parse_inventory, view, BodyWire, CardinalityKind, DelegationKind, NonDelegableReason,
        PayloadFileRule, VocabularyKind,
    };
    use tam_domain::TermKind;
    use tam_types::InventoryId;

    fn native<'a>(rendered: &'a super::VocabularyView, name: &str) -> &'a super::NativeFieldView {
        rendered
            .natives
            .iter()
            .find(|native| native.name == name)
            .unwrap_or_else(|| panic!("{name} is a native field of this inventory"))
    }

    #[test]
    fn every_inventory_renders_and_names_itself() {
        for inventory in InventoryId::ALL {
            let rendered = view(inventory);
            assert_eq!(
                rendered.inventory, inventory,
                "the payload is keyed on the inventory it was asked for"
            );
            assert_eq!(
                rendered.canonical.len(),
                6,
                "one entry per canonical field, so a form can render all six"
            );
        }
    }

    #[test]
    fn the_tes_licence_is_the_one_required_field_and_refuses_delegation() {
        let rendered = view(InventoryId::TesGb);
        let licence = native(&rendered, "licence");
        assert!(
            licence.required,
            "Tes licence is the only field declared required anywhere in the registry"
        );
        assert_eq!(
            licence.vocabulary,
            VocabularyKind::Closed,
            "the seven refdata values are captured, so the form renders a select"
        );
        assert_eq!(
            licence.values.as_ref().map(Vec::len),
            Some(7),
            "the read side holds seven, of which the gate below offers four"
        );
        assert_eq!(
            (licence.delegation.kind, licence.delegation.reason),
            (
                DelegationKind::Never,
                Some(NonDelegableReason::LegalContent)
            ),
            "issuing a rights grant is the seller's, and the form disables the control with \
             this reason"
        );
        let gate = rendered
            .authoring
            .licence
            .as_ref()
            .expect("Tes gates the licence on the price branch");
        assert_eq!(
            (gate.free.len(), gate.paid.as_slice()),
            (3, ["TES-PAID".to_owned()].as_slice()),
            "a free resource picks among the Creative Commons values and a paid one is TES-PAID"
        );
    }

    #[test]
    fn the_country_fork_reaches_the_form_as_the_phase_axis_native() {
        let phase = |inventory| {
            view(inventory)
                .axes
                .into_iter()
                .find(|axis| axis.axis == TermKind::Phase)
                .map(|axis| axis.native)
        };
        assert_eq!(
            (
                phase(InventoryId::TesGb),
                phase(InventoryId::TesUs),
                phase(InventoryId::TesNz)
            ),
            (
                Some("ageRanges".to_owned()),
                Some("yearGroups".to_owned()),
                Some("yearGroups".to_owned())
            ),
            "the uploader takes ageRanges for GB and yearGroups everywhere else, so the form \
             renders the seven-band picker or the thirty-year-group one"
        );
    }

    #[test]
    fn tes_takes_one_resource_type_and_tpt_folds_it_into_the_flat_tag_array() {
        let tes = view(InventoryId::TesGb)
            .axes
            .into_iter()
            .find(|axis| axis.axis == TermKind::ResourceType)
            .expect("Tes binds a resource type");
        assert_eq!(
            (tes.cardinality, tes.native.as_str()),
            (CardinalityKind::One, "mainType"),
            "Tes takes exactly one of the nine writable ids"
        );
        let tpt = view(InventoryId::Tpt)
            .axes
            .into_iter()
            .find(|axis| axis.axis == TermKind::ResourceType)
            .expect("TPT binds a resource type");
        assert_eq!(
            (tpt.cardinality, tpt.native.as_str()),
            (CardinalityKind::Many, "taxonomyTags"),
            "TPT folds it into the same flat namespace as everything else"
        );
    }

    #[test]
    fn tpt_declares_the_licence_axis_absent_rather_than_merely_unbound() {
        let rendered = view(InventoryId::Tpt);
        assert_eq!(
            rendered.absent_axes,
            vec![TermKind::Licence],
            "a licence projected into TPT is a disclosed loss, not a question"
        );
        assert!(
            rendered.authoring.licence.is_none(),
            "TPT holds no licence field, so the form shows a loss where Tes shows a selector"
        );
    }

    #[test]
    fn the_two_write_shapes_the_registry_does_not_hold_reach_the_form() {
        let tpt = view(InventoryId::Tpt).authoring;
        assert_eq!(
            (
                tpt.payload_files,
                tpt.body_wire,
                tpt.price_floor_minor_units
            ),
            (
                PayloadFileRule::ExactlyOne,
                BodyWire::RendersToHtml,
                Some(95)
            ),
            "a TPT create takes one file into the product slot, renders markdown to HTML, \
             and refuses a price below the form's own floor"
        );
        assert_eq!(
            tpt.attestation.map(|held| held.held_per_connection),
            Some(true),
            "the copyright declaration is held per connection, so the form displays it \
             rather than re-asking"
        );
        let tes = view(InventoryId::TesGb).authoring;
        assert_eq!(
            (tes.payload_files, tes.price_floor_minor_units),
            (PayloadFileRule::EveryPayloadFile, None),
            "Tes uploads every payload file and no Tes floor has been measured"
        );
    }

    #[test]
    fn the_taxonomy_tag_namespace_is_closed_without_being_captured() {
        let tags = view(InventoryId::Tpt);
        let tags = native(&tags, "taxonomyTags");
        assert_eq!(
            (tags.vocabulary, tags.values.is_none()),
            (VocabularyKind::ClosedUncaptured, true),
            "358 slugs live in the catalogue file, so the form must not render an empty select"
        );
    }

    #[test]
    fn the_path_segment_is_the_same_token_the_json_bodies_carry() {
        for inventory in InventoryId::ALL {
            let token = serde_json::to_value(inventory).expect("an inventory serialises");
            let token = token.as_str().expect("as a string");
            assert_eq!(
                parse_inventory(token),
                Some(inventory),
                "the path spelling round-trips through the wire spelling"
            );
        }
        assert_eq!(
            parse_inventory("tesgb"),
            None,
            "a spelling this server never issues is not accepted"
        );
    }
}
