//! What the form refuses, and what it merely says.
//!
//! The selection caps are a parameter rather than a constant. They are facts
//! about a form that was measured on a particular day, they live in
//! `docs/design/data/tpt-vocabulary.json`, and `tam_taxonomy::TptForm` is the
//! reader. This crate cannot call that reader — `tam-taxonomy` depends on
//! `tam-domain`, so the edge only runs one way — so the caller supplies
//! [`SelectionCaps`] and the numbers still come from the capture rather than
//! from a literal typed here. A cap that is `None` is unmeasured, never
//! unlimited, and an unmeasured cap refuses nothing.
//!
//! [`TptBaseProduct::check`] accumulates rather than short-circuiting: a
//! seller who left three controls wrong should read three messages, not one
//! at a time.

use tam_types::CopyFormat;

use crate::product::fields::FREE_RESOURCE_PAGE_GUIDANCE;
use crate::product::vocabularies::ThumbnailMode;
use crate::product::{PriceGroup, TptBaseProduct};

// ------------------------------------------------------------------- caps

/// How many members each create-form picker accepts, as the committed capture
/// states them.
///
/// Supplied by the caller from `tam_taxonomy::TptForm` rather than read here,
/// because the dependency edge runs the other way. `None` is unmeasured: the
/// 2026-08-30 create posted four subject-area slugs against a form that says
/// three, so that number is the form's claim rather than a refusal and
/// enforcing it would refuse a set TPT already accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SelectionCaps {
    pub grades: Option<usize>,
    pub subject_areas: Option<usize>,
    pub tags: Option<usize>,
    pub formats: Option<usize>,
    pub thumbnails: Option<usize>,
}

/// Which picker a cap violation is about, so a refusal names the control the
/// seller is looking at rather than a field name only the wire uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picker {
    Grades,
    SubjectAreas,
    Tags,
    Formats,
    Thumbnails,
}

impl Picker {
    pub const ALL: [Self; 5] = [
        Self::Grades,
        Self::SubjectAreas,
        Self::Tags,
        Self::Formats,
        Self::Thumbnails,
    ];

    /// The words TPT's own form puts on this control.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Grades => "Grade Level",
            Self::SubjectAreas => "Subject Area",
            Self::Tags => "Tag",
            Self::Formats => "Format",
            Self::Thumbnails => "Thumbnails",
        }
    }

    const fn cap_in(self, caps: SelectionCaps) -> Option<usize> {
        match self {
            Self::Grades => caps.grades,
            Self::SubjectAreas => caps.subject_areas,
            Self::Tags => caps.tags,
            Self::Formats => caps.formats,
            Self::Thumbnails => caps.thumbnails,
        }
    }
}

/// Something the form must refuse before anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthoringError {
    NameMissing,
    NameTooLong {
        used: usize,
        limit: usize,
    },
    EmptySlug,
    MalformedUploadRef,
    /// The whole set went over a measured cap. The count is reported rather
    /// than a truncated set, because a set cut to length is a different
    /// product from the one the seller described.
    OverCap {
        picker: Picker,
        chosen: usize,
        cap: usize,
    },
    /// TPT marks Grade Level, Subject Area and Tag required.
    PickerEmpty {
        picker: Picker,
    },
    /// Thumbnails were supplied under a mode whose slots the form does not
    /// render.
    ThumbnailsWithoutUploadNow {
        mode: ThumbnailMode,
    },
    PriceCurrencyMismatch,
    /// No attestation was chosen. Never defaulted: the statement is the
    /// seller's and a default makes it ours.
    CopyrightUnstated,
}

impl AuthoringError {
    /// The group whose heading the seller should be sent to. Named so an
    /// inline message can point at a control rather than at the form.
    #[must_use]
    pub const fn group(&self) -> FormGroup {
        match self {
            Self::NameMissing | Self::NameTooLong { .. } => FormGroup::Name,
            Self::MalformedUploadRef | Self::ThumbnailsWithoutUploadNow { .. } => FormGroup::Files,
            Self::PriceCurrencyMismatch => FormGroup::Price,
            Self::EmptySlug | Self::PickerEmpty { .. } => FormGroup::Categories,
            Self::OverCap { picker, .. } => match picker {
                Picker::Thumbnails => FormGroup::Files,
                Picker::Grades | Picker::SubjectAreas | Picker::Tags | Picker::Formats => {
                    FormGroup::Categories
                }
            },
            Self::CopyrightUnstated => FormGroup::Copyright,
        }
    }
}

/// Something worth telling the seller that is not a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoringWarning {
    /// TPT's Free Resource tooltip states "Free resources should be 10 pages
    /// or fewer." It is guidance on the form rather than a validated bound, so
    /// it never blocks: a seller who breaches it would otherwise learn it from
    /// TPT after the write instead of from us before it.
    FreeResourceOverPageGuidance { pages: u32, guidance: u32 },
}

/// The nine headings the form renders, in TPT's own order, with Education
/// Standards lifted out of Categories where TPT nests it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormGroup {
    Name,
    Files,
    Description,
    Price,
    Categories,
    EducationStandards,
    Details,
    Copyright,
    ProductStatus,
}

impl FormGroup {
    pub const ALL: [Self; 9] = [
        Self::Name,
        Self::Files,
        Self::Description,
        Self::Price,
        Self::Categories,
        Self::EducationStandards,
        Self::Details,
        Self::Copyright,
        Self::ProductStatus,
    ];

    /// The token this group is named by on the wire, so a refusal a client
    /// receives can be routed to the heading that holds the control.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Files => "files",
            Self::Description => "description",
            Self::Price => "price",
            Self::Categories => "categories",
            Self::EducationStandards => "education_standards",
            Self::Details => "details",
            Self::Copyright => "copyright",
            Self::ProductStatus => "product_status",
        }
    }

    /// The heading as TPT's own `.SectionHeading__headingTitle` reads it.
    #[must_use]
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Files => "Files",
            Self::Description => "Description",
            Self::Price => "Price",
            Self::Categories => "Categories",
            Self::EducationStandards => "Education Standards",
            Self::Details => "Details",
            Self::Copyright => "Copyright",
            Self::ProductStatus => "Product Status",
        }
    }
}

/// Everything the form should say before it submits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub errors: Vec<AuthoringError>,
    pub warnings: Vec<AuthoringWarning>,
}

impl Report {
    #[must_use]
    pub fn submittable(&self) -> bool {
        self.errors.is_empty()
    }
}

impl TptBaseProduct {
    /// Everything the form refuses, and everything it merely says, in one
    /// pass.
    ///
    /// Errors accumulate rather than short-circuiting: a seller who left three
    /// controls wrong should be shown three messages, not one at a time. The
    /// caps arrive from the capture through [`SelectionCaps`], and a cap the
    /// capture does not state refuses nothing and warns instead.
    #[must_use]
    pub fn check(&self, caps: SelectionCaps) -> Report {
        let mut report = Report::default();

        let required = [
            (Picker::Grades, &self.categories.grades),
            (Picker::SubjectAreas, &self.categories.subject_areas),
            (Picker::Tags, &self.categories.tags),
        ];
        for (picker, chosen) in required {
            if chosen.is_empty() {
                report.errors.push(AuthoringError::PickerEmpty { picker });
            }
        }

        let counted = [
            (Picker::Grades, self.categories.grades.len()),
            (Picker::SubjectAreas, self.categories.subject_areas.len()),
            (Picker::Tags, self.categories.tags.len()),
            (Picker::Formats, self.categories.formats.len()),
            (Picker::Thumbnails, self.files.thumbnails.len()),
        ];
        for (picker, chosen) in counted {
            // An unmeasured cap refuses nothing and says nothing: we hold no
            // number to measure the selection against, and the form renders a
            // plain count rather than a counter with an invented ceiling.
            if let Some(cap) = picker.cap_in(caps) {
                if chosen > cap {
                    report.errors.push(AuthoringError::OverCap {
                        picker,
                        chosen,
                        cap,
                    });
                }
            }
        }

        if !self.files.thumbnails.is_empty()
            && self.files.thumbnail_mode != ThumbnailMode::UploadNow
        {
            report
                .errors
                .push(AuthoringError::ThumbnailsWithoutUploadNow {
                    mode: self.files.thumbnail_mode,
                });
        }

        if self.copyright.is_none() {
            report.errors.push(AuthoringError::CopyrightUnstated);
        }

        if let (PriceGroup::Free, Some(pages)) = (&self.price, self.details.pages_or_slides) {
            if pages > FREE_RESOURCE_PAGE_GUIDANCE {
                report
                    .warnings
                    .push(AuthoringWarning::FreeResourceOverPageGuidance {
                        pages,
                        guidance: FREE_RESOURCE_PAGE_GUIDANCE,
                    });
            }
        }

        report
    }

    /// The description's declared format, which TPT's wire renders to HTML
    /// and Tes carries through.
    #[must_use]
    pub const fn body_format(&self) -> CopyFormat {
        self.description.format
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthoringError, AuthoringWarning, Picker};
    use crate::product::fields::{CategoryGroup, DetailGroup, UploadRef};
    use crate::product::fixtures::{product, slugs, CAPS, HASH};
    use crate::product::vocabularies::ThumbnailMode;

    #[test]
    fn a_complete_product_passes_with_nothing_to_say() {
        assert_eq!(
            product().check(CAPS),
            super::Report::default(),
            "the form refuses nothing and warns about nothing when every required control \
             is answered inside its cap"
        );
    }

    #[test]
    fn a_set_over_a_measured_cap_is_refused_by_the_picker_that_holds_it() {
        let mut over = product();
        over.categories.grades = slugs(&[
            "1st-grade",
            "2nd-grade",
            "3rd-grade",
            "4th-grade",
            "5th-grade",
        ]);
        assert_eq!(
            over.check(CAPS).errors,
            vec![AuthoringError::OverCap {
                picker: Picker::Grades,
                chosen: 5,
                cap: 4,
            }],
            "five grades against the capture's own limit of four is refused whole, and the \
             refusal names the control rather than a wire field"
        );
    }

    /// The cap the capture contradicts. Enforcing it would refuse a set TPT
    /// itself accepted on 2026-08-30, so four subject areas pass and the form
    /// renders a plain count rather than a counter against a number nobody
    /// holds.
    #[test]
    fn an_unmeasured_cap_refuses_nothing() {
        let mut many = product();
        many.categories.subject_areas = slugs(&["math", "science", "ela", "art"]);
        assert_eq!(
            many.check(CAPS),
            super::Report::default(),
            "a cap absent from the capture is unmeasured, never unlimited, and refusing on \
             one would raise a question about a set TPT already accepted"
        );
    }

    #[test]
    fn the_three_pickers_tpt_marks_required_are_refused_when_empty() {
        let mut bare = product();
        bare.categories = CategoryGroup::default();
        let errors = bare.check(CAPS).errors;
        for picker in [Picker::Grades, Picker::SubjectAreas, Picker::Tags] {
            assert!(
                errors.contains(&AuthoringError::PickerEmpty { picker }),
                "{} carries a Required marker on TPT's own form",
                picker.label()
            );
        }
        assert!(
            !errors.contains(&AuthoringError::PickerEmpty {
                picker: Picker::Formats
            }),
            "Format carries no required marker, so an empty one is not a refusal"
        );
    }

    /// The attestation is the seller's legal statement. TPT pre-selects value
    /// 1 on a blank form; ours refuses submission instead.
    #[test]
    fn an_unstated_copyright_declaration_blocks_submission() {
        let mut unstated = product();
        unstated.copyright = None;
        let report = unstated.check(CAPS);
        assert!(
            !report.submittable(),
            "nothing is written until the seller has actually attested"
        );
        assert_eq!(
            report.errors,
            vec![AuthoringError::CopyrightUnstated],
            "and the only thing wrong with the product is that one unanswered control"
        );
    }

    #[test]
    fn thumbnails_supplied_under_a_mode_with_no_slots_are_refused() {
        for mode in [ThumbnailMode::AutoGenerate, ThumbnailMode::UploadLater] {
            let mut deferred = product();
            deferred.files.thumbnail_mode = mode;
            deferred.files.thumbnails = vec![UploadRef::new(HASH).expect("a hex digest")];
            assert!(
                deferred
                    .check(CAPS)
                    .errors
                    .contains(&AuthoringError::ThumbnailsWithoutUploadNow { mode }),
                "the four slots are the conditional body of `upload thumbnails now`, so a \
                 file collected under {mode:?} was collected by a control TPT does not render"
            );
        }
    }

    /// The free-resource tooltip is guidance, so it is said rather than
    /// enforced.
    #[test]
    fn a_long_free_resource_warns_without_blocking() {
        let mut long = product();
        long.details = DetailGroup {
            pages_or_slides: Some(24),
            ..DetailGroup::default()
        };
        let report = long.check(CAPS);
        assert!(
            report.submittable(),
            "TPT states 10 pages as a tooltip and not as a validated bound"
        );
        assert!(
            report
                .warnings
                .contains(&AuthoringWarning::FreeResourceOverPageGuidance {
                    pages: 24,
                    guidance: 10,
                }),
            "and the seller reads it here rather than from TPT after the write"
        );
    }
}
