//! The shapes marketplaces issue their own identifiers in.
//!
//! A native identifier is minted where a vocabulary is derived and spent where
//! a listing is posted, and those are different crates on different branches
//! of the workspace graph. The grammar lives here, at the root both branches
//! already reach, so the derivation that seeds an identifier and the adapter
//! that posts it cannot drift apart on what the marketplace would answer to.

/// The shape TPT issues a taxonomy-tag identifier in: lowercase ASCII
/// letters, digits and hyphens, and never digits alone — `4th-grade`,
/// `homeschool` and `pdf` across the read capture, against the numeric ids
/// TPT uses for seller shelves and for its create form's own grade selector.
///
/// This is a provenance test rather than a vocabulary. TPT's registry binds
/// every equivalence axis to `taxonomyTags`, so every crosswalked term that
/// reaches a TPT listing is addressed by one of these slugs. A term projected
/// from another marketplace keeps that marketplace's own identifier — a Tes
/// topic arrives as `1000448`, a Tes licence as `TES-PAID`, a TPT grade taken
/// off the create form's selector as `8` — and posting any of them writes it
/// into a live listing verbatim. Translating one marketplace's vocabulary
/// into another's is what a crosswalk edge is for, and until an edge does it
/// an identifier TPT cannot have issued is refused rather than guessed at.
#[must_use]
pub fn is_tpt_tag_slug(native: &str) -> bool {
    !native.is_empty()
        && native.bytes().any(|byte| !byte.is_ascii_digit())
        && native
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::is_tpt_tag_slug;

    #[test]
    fn the_slugs_tpt_keys_its_own_facets_by_are_admitted() {
        for native in [
            "4th-grade",
            "homeschool",
            "pdf",
            "math",
            "not-grade-specific",
        ] {
            assert!(
                is_tpt_tag_slug(native),
                "{native:?} is a slug the TPT capture keys a facet by"
            );
        }
    }

    /// A Tes topic id and a Tes resource-type id; a TPT grade legacy id off
    /// the create form's own selector; two Tes licence tokens, hyphenated
    /// like slugs and upper case; and three that belong to nobody.
    #[test]
    fn an_identifier_another_marketplace_issued_is_refused() {
        let foreign = [
            "1000448",
            "99001",
            "8",
            "TES-PAID",
            "CC-BY",
            "",
            "unit plans",
            "Math",
        ];
        for native in foreign {
            assert!(
                !is_tpt_tag_slug(native),
                "{native:?} is not the shape TPT issues a taxonomy tag in"
            );
        }
    }
}
