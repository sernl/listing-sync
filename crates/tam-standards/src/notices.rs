//! What must be rendered wherever a standard is shown, and where.
//!
//! These are compliance obligations rather than presentation choices, so they
//! are constants with a test rather than strings in a template. Three
//! decisions in `docs/notes/design/vendoo-for-teachers-rethink.md` fix them:
//! D25 displays statement text verbatim and accepts the Common Core
//! attribution obligation, D18 complies with the NGSS trademark guidance by
//! carrying the prescribed disclaimer in the footer and using no logo, and D19
//! ingests TEKS from the Common Standards Project mirror under its CC BY
//! licence.
//!
//! One obligation is not a string and cannot be one: the Common Core grant
//! names four verbs -- copy, publish, distribute, display -- and modification
//! is not among them, so no standard's statement may be paraphrased,
//! shortened for a dropdown, or rewritten into generated listing copy. That is
//! enforced by never providing a function that does it.

use crate::model::{Framework, Licence};

/// Where a notice has to appear for the obligation to be discharged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// On any page that publishes or publicly displays the framework's
    /// content: the picker, the product form, and any listing preview that
    /// renders a code or statement.
    WhereverDisplayed,
    /// At the bottom of the site's home page and of every internal page that
    /// prominently uses the mark. WestEd's guidance states that placing it
    /// only in terms and conditions does not satisfy the requirement.
    SiteFooterAndEveryPageUsingTheMark,
    /// Beside the data itself, which under CC BY means wherever the mirrored
    /// rows are shown or redistributed.
    WithTheData,
}

/// One rendering obligation: the text, where it goes, and the clause it
/// discharges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notice {
    pub text: &'static str,
    pub placement: Placement,
    /// The published term this notice answers, so a reviewer can check the
    /// wording against its source rather than against this file.
    pub source: &'static str,
}

/// The Common Core attribution, verbatim from the public licence.
///
/// The licence requires that "Any publication or public display shall include
/// the following notice". States that adopted the standards in whole are
/// exempt; we are not a state.
pub const CCSS_COPYRIGHT_NOTICE: &str = "© Copyright 2010. National Governors Association Center for Best Practices and Council of Chief State School Officers. All rights reserved.";

/// The Common Core ownership acknowledgement the same clause requires beside
/// the notice: that the owners "shall be acknowledged as the sole owners and
/// developers", and that no claim to the contrary is made.
pub const CCSS_OWNERSHIP_ACKNOWLEDGEMENT: &str = "The Common Core State Standards are owned and were developed solely by the NGA Center for Best Practices and the Council of Chief State School Officers.";

/// The WestEd disclaimer, in the prescribed form with the mark substituted for
/// the blank.
///
/// The asterisk is the mark's own footnote marker: the guidance forbids ® and
/// ™ and requires an asterisk plus this footnote instead.
pub const NGSS_TRADEMARK_DISCLAIMER: &str = "*Next Generation Science Standards is a registered trademark of WestEd. Neither WestEd nor the lead states and partners that developed the Next Generation Science Standards were involved in the production of this product, and do not endorse it.";

/// The citation the NGSS copyright terms require.
pub const NGSS_CITATION: &str = "NGSS Lead States. 2013. Next Generation Science Standards: For States, By States. Washington, DC: The National Academies Press.";

/// What the NGSS guidance forbids, kept beside the disclaimer because a
/// reviewer checking the footer needs to check these at the same time.
pub const NGSS_MARK_RULES: &str = "The NGSS logo is not used, the ® and ™ symbols are not used, and the mark does not appear in a product title, company name, domain name, metatag or purchased keyword, and stays visually subordinate to our own marks.";

const CCSS_NOTICES: [Notice; 2] = [
    Notice {
        text: CCSS_COPYRIGHT_NOTICE,
        placement: Placement::WhereverDisplayed,
        source: "https://www.thecorestandards.org/public-license/",
    },
    Notice {
        text: CCSS_OWNERSHIP_ACKNOWLEDGEMENT,
        placement: Placement::WhereverDisplayed,
        source: "https://www.thecorestandards.org/public-license/",
    },
];

const NGSS_NOTICES: [Notice; 3] = [
    Notice {
        text: NGSS_TRADEMARK_DISCLAIMER,
        placement: Placement::SiteFooterAndEveryPageUsingTheMark,
        source: "https://www.nextgenscience.org/trademark-and-copyright/trademark-and-copyright",
    },
    Notice {
        text: NGSS_CITATION,
        placement: Placement::WhereverDisplayed,
        source: "https://www.nextgenscience.org/trademark-and-copyright/trademark-and-copyright",
    },
    Notice {
        text: NGSS_MARK_RULES,
        placement: Placement::SiteFooterAndEveryPageUsingTheMark,
        source: "https://www.nextgenscience.org/trademark-and-copyright/trademark-and-copyright",
    },
];

/// Texas and Virginia carry no owner-imposed notice of their own: the TEKS are
/// Chapter 110-128 of Title 19 of the Texas Administrative Code and TEA's own
/// feed declares no licence, and Virginia states none on any page reachable to
/// us. Their obligation is the mirror's CC BY attribution, which
/// `mirror_attribution` builds from the licence the mirror declared, so the
/// framework list here is empty by fact rather than by omission.
const NO_OWNER_NOTICE: [Notice; 0] = [];

/// Every notice that must render wherever this framework's content is shown.
///
/// The mirror's own CC BY attribution is not here because it names a rights
/// holder that varies per set; build it with `mirror_attribution` from the
/// licences the manifest recorded.
pub fn required_notices(framework: Framework) -> &'static [Notice] {
    match framework {
        Framework::Ccss => &CCSS_NOTICES,
        Framework::Ngss => &NGSS_NOTICES,
        Framework::Teks | Framework::VaSol => &NO_OWNER_NOTICE,
    }
}

/// The CC BY attribution for a mirrored set, built from the licence the mirror
/// declared rather than from a hardcoded holder.
///
/// The Common Standards Project declares the licence per set and the holders
/// differ across them -- D2L Corporation on the Achievement Standards Network
/// lineage, Common Curriculum, Inc. elsewhere -- so a single constant would be
/// wrong for some of the rows it sat beside.
pub fn mirror_attribution(licence: &Licence) -> Option<String> {
    if licence.title.is_empty() && licence.rights_holder.is_empty() {
        return None;
    }
    let holder = if licence.rights_holder.is_empty() {
        "the Common Standards Project".to_owned()
    } else {
        licence.rights_holder.clone()
    };
    let terms = if licence.title.is_empty() {
        String::new()
    } else if licence.url.is_empty() {
        format!(", under {}", licence.title)
    } else {
        format!(", under {} ({})", licence.title, licence.url)
    };
    Some(format!(
        "Standards data via the Common Standards Project, © {holder}{terms}."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FRAMEWORKS;

    #[test]
    fn every_notice_carries_text_and_a_source() {
        for framework in FRAMEWORKS {
            for notice in required_notices(framework) {
                assert!(
                    !notice.text.trim().is_empty(),
                    "{framework} has an empty notice"
                );
                assert!(
                    notice.source.starts_with("https://"),
                    "{framework} notice cites `{}`, which is not a source a reviewer can open",
                    notice.source
                );
            }
        }
    }

    #[test]
    fn common_core_carries_the_owners_prescribed_notice() {
        let notices = required_notices(Framework::Ccss);
        assert!(
            notices
                .iter()
                .any(|notice| notice.text == CCSS_COPYRIGHT_NOTICE),
            "the prescribed copyright notice is required on any publication or public display"
        );
        assert!(
            CCSS_COPYRIGHT_NOTICE
                .contains("National Governors Association Center for Best Practices")
                && CCSS_COPYRIGHT_NOTICE.contains("Council of Chief State School Officers")
                && CCSS_COPYRIGHT_NOTICE.contains("© Copyright 2010"),
            "the notice must name both owners and the year, verbatim"
        );
        assert!(
            notices
                .iter()
                .all(|notice| notice.placement == Placement::WhereverDisplayed),
            "the Common Core obligation attaches to display, not to a footer"
        );
    }

    #[test]
    fn ngss_carries_the_prescribed_disclaimer_in_the_footer_and_claims_no_endorsement() {
        let notices = required_notices(Framework::Ngss);
        let disclaimer = notices
            .iter()
            .find(|notice| notice.text == NGSS_TRADEMARK_DISCLAIMER)
            .expect("the WestEd disclaimer is required");
        assert_eq!(
            disclaimer.placement,
            Placement::SiteFooterAndEveryPageUsingTheMark,
            "WestEd's guidance places it at the bottom of the home page and every page using the mark"
        );
        assert!(
            NGSS_TRADEMARK_DISCLAIMER.contains("registered trademark of WestEd")
                && NGSS_TRADEMARK_DISCLAIMER.contains("do not endorse it"),
            "the disclaimer must keep the prescribed wording"
        );
        assert!(
            !NGSS_TRADEMARK_DISCLAIMER.contains('®') && !NGSS_TRADEMARK_DISCLAIMER.contains('™'),
            "the guidance forbids the symbols and requires the asterisk footnote instead"
        );
        assert!(
            NGSS_TRADEMARK_DISCLAIMER.starts_with('*'),
            "the asterisk is the footnote marker the guidance mandates"
        );
        assert!(
            notices.iter().any(|notice| notice.text == NGSS_CITATION),
            "the National Academies Press citation is required"
        );
    }

    #[test]
    fn texas_and_virginia_carry_no_owner_notice_and_say_so() {
        assert!(required_notices(Framework::Teks).is_empty());
        assert!(required_notices(Framework::VaSol).is_empty());
    }

    #[test]
    fn the_mirror_attribution_names_the_holder_the_set_declared() {
        let licence = Licence {
            title: "CC BY 3.0 US".to_owned(),
            url: "http://creativecommons.org/licenses/by/3.0/us/".to_owned(),
            rights_holder: "D2L Corporation".to_owned(),
        };
        let rendered = mirror_attribution(&licence).expect("a declared licence attributes");
        assert!(
            rendered.contains("D2L Corporation"),
            "the holder is named: {rendered}"
        );
        assert!(
            rendered.contains("CC BY 3.0 US"),
            "the terms are named: {rendered}"
        );
        assert_eq!(
            mirror_attribution(&Licence::default()),
            None,
            "a set that declared nothing attributes nothing rather than inventing a holder"
        );
    }
}
