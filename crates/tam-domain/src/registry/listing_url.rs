//! The page a seller opens to see one of their own listings on a marketplace.
//!
//! Derived from the identifier the binding already holds, never fetched: this
//! module performs no request and the shapes below come from what the adapter
//! crates observed in their captures. It answers `None` for a marketplace
//! whose page shape nothing here has observed, because a wrong URL on an "open
//! the live listing" control sends a seller to another author's page or to a
//! 404, which is worse than the control being absent.

use tam_marketplace::RemoteListingId;

/// TPT's origin, restated rather than imported.
///
/// `tam-marketplace-tpt` holds the same constant, but this crate is in the
/// pure set and does not depend on the adapter crates; importing it would
/// invert the layering. The adapter's copy is the other one.
const TPT_ORIGIN: &str = "https://www.teacherspayteachers.com";

/// Tes's origin, restated for the same reason as [`TPT_ORIGIN`].
const TES_ORIGIN: &str = "https://www.tes.com";

/// The slug in a TPT product path, which is decorative.
///
/// `tam-marketplace-tpt`'s `classify` module establishes that a product path
/// carrying the wrong slug still serves the correct product, so the slug
/// carries no meaning and one constant word serves every listing. A
/// title-derived slug was considered and rejected: it would put a join onto
/// `product` on every mapping read to decorate a path whose decoration is
/// discarded by the marketplace.
const TPT_SLUG: &str = "listing";

/// The listing's own page, or `None` where no page shape is known.
///
/// Etsy answers `None` throughout: no adapter crate exists for it yet, so
/// nothing here has observed its page shape and inventing one would be a
/// guess rendered to a seller as a link.
#[must_use]
pub fn listing_url(id: &RemoteListingId) -> Option<String> {
    match id {
        RemoteListingId::Tpt { product_id } => {
            Some(format!("{TPT_ORIGIN}/Product/{TPT_SLUG}-{product_id}"))
        }
        RemoteListingId::Tes { url } => {
            tes_resource_id(url).map(|id| format!("{TES_ORIGIN}/teaching-resource/-{id}"))
        }
        RemoteListingId::Etsy { .. } => None,
    }
}

/// The numeric resource identifier inside a stored Tes URL.
///
/// Two shapes reach the binding and both end in the identifier: the create
/// path stores the canonical `…/api/v2/resources/{id}`, which is an API route
/// rather than a page, and the import path stores a `…/teaching-resource/{slug}-{id}`
/// page. Reading the identifier out of either is what lets one function serve
/// both without the caller knowing which wrote the row.
fn tes_resource_id(url: &str) -> Option<&str> {
    let path = url.split(['?', '#']).next()?.trim_end_matches('/');
    let last = path.rsplit('/').next()?;
    // A page path carries `{slug}-{id}` and the slug may itself contain
    // hyphens, so the identifier is what follows the final one.
    let candidate = last.rsplit('-').next()?;
    if candidate.is_empty() || !candidate.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(candidate)
}

#[cfg(test)]
mod tests {
    use super::listing_url;
    use tam_marketplace::RemoteListingId;

    #[test]
    fn a_tpt_listing_is_its_numeric_id_under_a_constant_slug() {
        assert_eq!(
            listing_url(&RemoteListingId::Tpt {
                product_id: 17_511_712
            }),
            Some("https://www.teacherspayteachers.com/Product/listing-17511712".to_owned()),
            "the slug is cosmetic, so one constant word serves every product"
        );
    }

    #[test]
    fn the_canonical_api_form_normalises_to_the_page_a_seller_opens() {
        assert_eq!(
            listing_url(&RemoteListingId::Tes {
                url: "https://www.tes.com/api/v2/resources/13264370".to_owned()
            }),
            Some("https://www.tes.com/teaching-resource/-13264370".to_owned()),
            "the create path stores an API route, and a seller opening it would get JSON"
        );
    }

    #[test]
    fn the_page_form_survives_the_same_normalisation() {
        for stored in [
            "https://www.tes.com/teaching-resource/-13264370",
            "https://www.tes.com/teaching-resource/fractions-revision-pack-13264370",
            "https://www.tes.com/teaching-resource/x-13264370/",
        ] {
            assert_eq!(
                listing_url(&RemoteListingId::Tes {
                    url: stored.to_owned()
                }),
                Some("https://www.tes.com/teaching-resource/-13264370".to_owned()),
                "every stored page shape answers the one canonical page: {stored}"
            );
        }
    }

    #[test]
    fn a_query_or_fragment_is_not_part_of_the_identifier() {
        assert_eq!(
            listing_url(&RemoteListingId::Tes {
                url: "https://www.tes.com/teaching-resource/pack-13264370?utm_source=x".to_owned()
            }),
            Some("https://www.tes.com/teaching-resource/-13264370".to_owned())
        );
    }

    #[test]
    fn a_stored_value_matching_neither_shape_answers_nothing() {
        for stored in [
            "https://www.tes.com/teaching-resource/no-digits-here",
            "https://www.tes.com/teaching-resource/",
            "not a url at all",
            "",
        ] {
            assert_eq!(
                listing_url(&RemoteListingId::Tes {
                    url: stored.to_owned()
                }),
                None,
                "a link that would send a seller to the wrong page is not offered: {stored}"
            );
        }
    }

    #[test]
    fn etsy_has_no_page_shape_here_yet() {
        assert_eq!(
            listing_url(&RemoteListingId::Etsy {
                listing_id: 1_234_567_890
            }),
            None,
            "no Etsy adapter exists, so its page shape is unobserved rather than known"
        );
    }
}
