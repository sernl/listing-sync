//! The closed sets a control chooses among, each carrying the wire id TPT's
//! own form posts.
//!
//! Grouped here rather than beside the fields that hold them because they
//! share one hazard: the id is the value and a menu position is not. TPT's
//! Answer Key menu runs N/A, Included, Not Included, Included with Rubric,
//! Rubric Only, Does Not Apply, which is wire ids 0, 1, 2, 4, 5, 3, so
//! anything inferring an id from a position writes "Included with Rubric" as
//! "Does Not Apply" and no seller can see that it happened. Every member here
//! answers `wire_id` and `from_wire_id`, and only [`AnswerKey`] answers
//! `menu_index` at all — nothing converts the other way.
//!
//! Nothing here has a `Default`. A tax code defaulted is a tax determination
//! the seller is contractually answerable for (D7), and a copyright
//! declaration defaulted is an attestation we made rather than they did (D13).

/// `data[Item][generate_thumbnail]`, whose three values the 2026-09-03 DOM
/// read closed. The four upload slots are the conditional body of
/// [`Self::UploadNow`] and are hidden under either other value, so thumbnails
/// supplied under one of those would be collected by a control TPT does not
/// render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbnailMode {
    AutoGenerate,
    UploadNow,
    UploadLater,
}

impl ThumbnailMode {
    pub const ALL: [Self; 3] = [Self::AutoGenerate, Self::UploadNow, Self::UploadLater];

    #[must_use]
    pub const fn wire_id(self) -> u8 {
        match self {
            Self::AutoGenerate => 1,
            Self::UploadNow => 2,
            Self::UploadLater => 3,
        }
    }

    #[must_use]
    pub fn from_wire_id(id: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.wire_id() == id)
    }

    /// The words the radio carries, verbatim from the DOM read.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::AutoGenerate => "Auto generate thumbnails from the product file",
            Self::UploadNow => "Upload thumbnails now",
            Self::UploadLater => "Upload thumbnails later",
        }
    }
}

/// The Files group: the payload, its two optional previews, and the thumbnail
/// `data[ItemTaxCode][tax_code_id]` — the row id, never the Avalara string.
///
/// Five members, whose wire id is the DOM index plus one. Never defaulted:
/// TPT's terms of service put the designation on the seller, so a tool that
/// picks one is making a tax determination its user answers for (D7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaxCode {
    DigitalAudio,
    DigitalBooks,
    DigitalImages,
    Videos,
    OtherDigitalGoods,
}

impl TaxCode {
    pub const ALL: [Self; 5] = [
        Self::DigitalAudio,
        Self::DigitalBooks,
        Self::DigitalImages,
        Self::Videos,
        Self::OtherDigitalGoods,
    ];

    #[must_use]
    pub const fn wire_id(self) -> u8 {
        match self {
            Self::DigitalAudio => 1,
            Self::DigitalBooks => 2,
            Self::DigitalImages => 3,
            Self::Videos => 4,
            Self::OtherDigitalGoods => 5,
        }
    }

    #[must_use]
    pub fn from_wire_id(id: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|code| code.wire_id() == id)
    }

    /// The Avalara code the read side returns for this row.
    #[must_use]
    pub const fn avalara_code(self) -> &'static str {
        match self {
            Self::DigitalAudio => "DA051011",
            Self::DigitalBooks => "DB031013",
            Self::DigitalImages => "DI010200",
            Self::Videos => "DV010200",
            Self::OtherDigitalGoods => "DO010000",
        }
    }
}

// ------------------------------------------------------------ the standards

/// The four jurisdictions TPT's create form offers, out of the 166 it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardsFramework {
    CommonCore,
    NextGenerationScience,
    TexasEssentialKnowledgeAndSkills,
    VirginiaStandardsOfLearning,
}

impl StandardsFramework {
    pub const ALL: [Self; 4] = [
        Self::CommonCore,
        Self::NextGenerationScience,
        Self::TexasEssentialKnowledgeAndSkills,
        Self::VirginiaStandardsOfLearning,
    ];

    /// TPT's own jurisdiction id, which is what `EducationStandardsQuery`
    /// expands.
    #[must_use]
    pub const fn jurisdiction_id(self) -> u32 {
        match self {
            Self::CommonCore => 3054,
            Self::NextGenerationScience => 3055,
            Self::TexasEssentialKnowledgeAndSkills => 3326,
            Self::VirginiaStandardsOfLearning => 5785,
        }
    }

    #[must_use]
    pub fn from_jurisdiction_id(id: u32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|framework| framework.jurisdiction_id() == id)
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::CommonCore => "Common Core State Standards",
            Self::NextGenerationScience => "Next Generation Science Standards",
            Self::TexasEssentialKnowledgeAndSkills => "Texas Essential Knowledge and Skills",
            Self::VirginiaStandardsOfLearning => "Virginia Standards of Learning",
        }
    }

    /// The words on the button that opens this jurisdiction's picker.
    #[must_use]
    pub const fn button_label(self) -> &'static str {
        match self {
            Self::CommonCore => "Select CCSS",
            Self::NextGenerationScience => "Select NGSS",
            Self::TexasEssentialKnowledgeAndSkills => "Select TEKS",
            Self::VirginiaStandardsOfLearning => "Select VA SOL",
        }
    }
}

// -------------------------------------------------------------- the details

/// `data[ItemsProperty][duration]`. Twenty-three members whose wire id is the
/// menu position, which is true today and is not relied on: the id is the
/// value and the position is derived from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeachingDuration(u8);

/// The labels in wire-id order, which for this vocabulary is also menu order.
/// Title case throughout, corrected against the DOM on 2026-09-03; the DOM is
/// the display authority.
const TEACHING_DURATION_LABELS: [&str; 23] = [
    "N/A",
    "30 Minutes",
    "40 Minutes",
    "45 Minutes",
    "50 Minutes",
    "55 Minutes",
    "1 Hour",
    "90 Minutes",
    "2 Hours",
    "3 Hours",
    "2 Days",
    "3 Days",
    "4 Days",
    "1 Week",
    "2 Weeks",
    "3 Weeks",
    "1 Month",
    "2 Months",
    "3 Months",
    "1 Semester",
    "1 Year",
    "Lifelong Tool",
    "Other",
];

impl TeachingDuration {
    pub const COUNT: u8 = 23;

    pub fn from_wire_id(id: u8) -> Option<Self> {
        (id < Self::COUNT).then_some(Self(id))
    }

    #[must_use]
    pub const fn wire_id(self) -> u8 {
        self.0
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        TEACHING_DURATION_LABELS[self.0 as usize]
    }

    /// Every member in wire-id order.
    pub fn all() -> impl Iterator<Item = Self> {
        (0..Self::COUNT).map(Self)
    }
}

/// `data[ItemsProperty][answer_key]`.
///
/// The one vocabulary in this module whose menu order is not its id order.
/// [`Self::menu_index`] carries TPT's own position so a form can render the
/// familiar order while the value stays the id; nothing anywhere converts the
/// other way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerKey {
    NotApplicable,
    Included,
    NotIncluded,
    DoesNotApply,
    IncludedWithRubric,
    RubricOnly,
}

impl AnswerKey {
    /// In wire-id order, which is the order this codebase iterates in.
    pub const ALL: [Self; 6] = [
        Self::NotApplicable,
        Self::Included,
        Self::NotIncluded,
        Self::DoesNotApply,
        Self::IncludedWithRubric,
        Self::RubricOnly,
    ];

    #[must_use]
    pub const fn wire_id(self) -> u8 {
        match self {
            Self::NotApplicable => 0,
            Self::Included => 1,
            Self::NotIncluded => 2,
            Self::DoesNotApply => 3,
            Self::IncludedWithRubric => 4,
            Self::RubricOnly => 5,
        }
    }

    /// Where TPT's own `#answerKey-menu` puts this member. Not the wire id:
    /// the menu runs N/A, Included, Not Included, Included with Rubric, Rubric
    /// Only, Does Not Apply, so positions 3 and 5 carry ids 4 and 3.
    #[must_use]
    pub const fn menu_index(self) -> u8 {
        match self {
            Self::NotApplicable => 0,
            Self::Included => 1,
            Self::NotIncluded => 2,
            Self::IncludedWithRubric => 3,
            Self::RubricOnly => 4,
            Self::DoesNotApply => 5,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotApplicable => "N/A",
            Self::Included => "Included",
            Self::NotIncluded => "Not Included",
            Self::DoesNotApply => "Does Not Apply",
            Self::IncludedWithRubric => "Included with Rubric",
            Self::RubricOnly => "Rubric Only",
        }
    }

    #[must_use]
    pub fn from_wire_id(id: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.wire_id() == id)
    }
}

// ------------------------------------------------------------ the copyright

/// Which of TPT's two attestations the seller made.
///
/// No `Default` and no conversion from a boolean, matching the adapter's
/// `AuthorshipDeclaration`, which records who attested and when. This records
/// *which* statement they attested to; the adapter records that they did. TPT
/// pre-selects value `1` on a blank form and a naive replay would therefore
/// post an attestation nobody made, which is the whole reason the option here
/// is absent rather than defaulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyrightDeclaration {
    OriginalWork,
    UsedCopyrightedMaterials,
}

impl CopyrightDeclaration {
    pub const ALL: [Self; 2] = [Self::OriginalWork, Self::UsedCopyrightedMaterials];

    #[must_use]
    pub const fn wire_id(self) -> u8 {
        match self {
            Self::OriginalWork => 1,
            Self::UsedCopyrightedMaterials => 2,
        }
    }

    #[must_use]
    pub fn from_wire_id(id: u8) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|declaration| declaration.wire_id() == id)
    }

    /// The statement, verbatim from the DOM read. Quoted in full wherever it
    /// is shown, because an abbreviated legal attestation is a different
    /// attestation.
    #[must_use]
    pub const fn statement(self) -> &'static str {
        match self {
            Self::OriginalWork => {
                "I attest that this product I am about to post is an original work and it \
                 does not infringe upon the Intellectual Property rights of others."
            }
            Self::UsedCopyrightedMaterials => {
                "I attest that I have used copyrighted and/or trademarked materials in my \
                 product and it does not infringe upon the Intellectual Property rights of \
                 others. I have either received express permission to use such materials, or \
                 I hereby certify that the use of such materials is otherwise non-infringing, \
                 for example as a fair use."
            }
        }
    }
}

/// The preamble the radio group sits under, verbatim from
/// `#label_intellectualPropertyRightsTitle`.
pub const COPYRIGHT_PREAMBLE: &str = "Intellectual Property Rights: By uploading the selected \
     material I certify that I have read and agree to the Teachers Pay Teachers terms of \
     service and that the selected material does not infringe the copyrights, trademark \
     rights, or any other rights of any third party, and:";

// --------------------------------------------------------------- the status

/// `data[Item][status_user]`. Draft versus live is a field rather than a
/// route, so publishing is an ordinary edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListingStatus {
    Draft,
    Live,
}

impl ListingStatus {
    pub const ALL: [Self; 2] = [Self::Draft, Self::Live];

    #[must_use]
    pub const fn wire_id(self) -> u8 {
        match self {
            Self::Draft => 0,
            Self::Live => 1,
        }
    }

    #[must_use]
    pub fn from_wire_id(id: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.wire_id() == id)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AnswerKey, CopyrightDeclaration, ListingStatus, TaxCode, TeachingDuration, ThumbnailMode,
    };

    /// The trap the DOM read found. A menu position is not an id, and a form
    /// that treated it as one would write "Included with Rubric" as "Does Not
    /// Apply" with nothing visible to the seller.
    #[test]
    fn the_answer_key_menu_order_is_not_its_id_order() {
        let by_menu: Vec<AnswerKey> = {
            let mut all = AnswerKey::ALL;
            all.sort_by_key(|key| key.menu_index());
            all.to_vec()
        };
        assert_eq!(
            by_menu.iter().map(|key| key.wire_id()).collect::<Vec<u8>>(),
            vec![0, 1, 2, 4, 5, 3],
            "the six `li` elements run N/A, Included, Not Included, Included with Rubric, \
             Rubric Only, Does Not Apply"
        );
        assert_eq!(
            AnswerKey::from_wire_id(3),
            Some(AnswerKey::DoesNotApply),
            "id 3 is Does Not Apply, which sits last in the menu"
        );
        assert_eq!(
            by_menu[3],
            AnswerKey::IncludedWithRubric,
            "menu position 3 is Included with Rubric, whose id is 4 — reading the id off the \
             position is the silent mis-map"
        );
    }

    #[test]
    fn every_listbox_member_round_trips_through_its_wire_id() {
        for key in AnswerKey::ALL {
            assert_eq!(
                AnswerKey::from_wire_id(key.wire_id()),
                Some(key),
                "answer key"
            );
        }
        for code in TaxCode::ALL {
            assert_eq!(
                TaxCode::from_wire_id(code.wire_id()),
                Some(code),
                "tax code"
            );
        }
        for mode in ThumbnailMode::ALL {
            assert_eq!(
                ThumbnailMode::from_wire_id(mode.wire_id()),
                Some(mode),
                "thumbnail mode"
            );
        }
        for status in ListingStatus::ALL {
            assert_eq!(
                ListingStatus::from_wire_id(status.wire_id()),
                Some(status),
                "listing status"
            );
        }
        for declaration in CopyrightDeclaration::ALL {
            assert_eq!(
                CopyrightDeclaration::from_wire_id(declaration.wire_id()),
                Some(declaration),
                "copyright declaration"
            );
        }
        assert_eq!(
            TeachingDuration::all().count(),
            23,
            "twenty-three durations, whose wire id is also the menu position"
        );
        assert_eq!(
            TeachingDuration::from_wire_id(23),
            None,
            "and nothing past the end of the menu"
        );
    }
}
