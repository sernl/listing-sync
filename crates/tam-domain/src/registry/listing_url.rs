//! The page a seller opens to see one of their own listings on a marketplace.
//!
//! Derived from the identifier the binding already holds, never fetched: this
//! module performs no request and the shapes below come from what the adapter
//! crates observed in their captures. It answers `None` for a marketplace
//! whose page shape nothing here has observed, because a wrong URL on an "open
//! the live listing" control sends a seller to another author's page or to a
//! 404, which is worse than the control being absent.
//!
//! The inverse also lives here: a URL a seller pasted is parsed back to an
//! identifier by [`parse_listing_url`], so the two directions cannot drift.

use tam_marketplace::RemoteListingId;
use tam_types::Marketplace;

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

/// Why a pasted listing URL could not become an identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlRefusal {
    /// The URL is a listing page, but on a different marketplace than the one
    /// the mapping reaches. Named rather than folded into `Unrecognised`
    /// because the seller's remedy differs: this is the right link pasted onto
    /// the wrong row.
    WrongMarketplace { named: Marketplace },
    /// No marketplace here recognises the URL. Also the answer for a
    /// marketplace whose page shape is unobserved, which is Etsy today.
    Unrecognised,
}

/// The identifier a marketplace's own listing page names.
///
/// Host and path are both checked, because the identifier alone is evidence of
/// nothing: every marketplace's page ends in digits, so a parser that read only
/// the trailing number would accept one marketplace's link as another's. The
/// adapters' own parsers read a `Location` header from a redirect they had just
/// caused, where the marketplace was never in doubt; a URL a seller pasted has
/// no such provenance and is validated here instead.
///
/// Tes answers the canonical `/api/v2/resources/{id}` identity rather than the
/// page that was pasted. `DraftId::canonical_url` in `tam-marketplace-tes`
/// requires it: that string is what a bind writes and what
/// `mapping_one_bound_url` indexes, so a second spelling of one resource makes
/// a later write report a divergent landing against the mapping it just wrote.
pub fn parse_listing_url(expected: Marketplace, url: &str) -> Result<RemoteListingId, UrlRefusal> {
    let (host, path) = split_url(url).ok_or(UrlRefusal::Unrecognised)?;
    let found = recognise(&host, path).ok_or(UrlRefusal::Unrecognised)?;
    let named = found.marketplace();
    if named == expected {
        Ok(found)
    } else {
        Err(UrlRefusal::WrongMarketplace { named })
    }
}

/// The host without its scheme, credentials, port or `www.`, and the path
/// without its query or fragment. Hand-split rather than parsed: this crate is
/// in the pure set and depends on no URL library.
fn split_url(url: &str) -> Option<(String, &str)> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let rest = rest.split(['?', '#']).next()?;
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, path),
        None => (rest, ""),
    };
    // Credentials before an `@` are the classic way to dress one host as
    // another, so the host is what follows the last one.
    let authority = authority.rsplit('@').next()?;
    let host = authority.split(':').next()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
    Some((host, path))
}

fn recognise(host: &str, path: &str) -> Option<RemoteListingId> {
    let (head, rest) = match path.split_once('/') {
        Some(split) => split,
        None => (path, ""),
    };
    if host == "teacherspayteachers.com" && head.eq_ignore_ascii_case("Product") {
        return trailing_id(rest).map(|product_id| RemoteListingId::Tpt { product_id });
    }
    if host == "tes.com"
        && (head.eq_ignore_ascii_case("teaching-resource") || path.starts_with("api/v2/resources/"))
    {
        return trailing_id(path).map(|id| RemoteListingId::Tes {
            url: format!("{TES_ORIGIN}/api/v2/resources/{id}"),
        });
    }
    None
}

/// The identifier a listing path ends in: every shape here puts it last, after
/// a decorative slug where one exists.
fn trailing_id(path: &str) -> Option<u64> {
    path.trim_end_matches('/')
        .rsplit(['-', '/'])
        .next()
        .and_then(|tail| tail.parse::<u64>().ok())
}

#[cfg(test)]
mod tests {
    use super::{listing_url, parse_listing_url, UrlRefusal};
    use tam_marketplace::RemoteListingId;
    use tam_types::Marketplace;

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

    #[test]
    fn a_pasted_tpt_product_page_parses_to_its_numeric_id() {
        for pasted in [
            "https://www.teacherspayteachers.com/Product/fractions-pack-17511712",
            "https://teacherspayteachers.com/Product/listing-17511712",
            "http://www.teacherspayteachers.com/Product/x-17511712/",
            "https://www.teacherspayteachers.com/Product/x-17511712?utm_source=pin",
        ] {
            assert_eq!(
                parse_listing_url(Marketplace::Tpt, pasted),
                Ok(RemoteListingId::Tpt {
                    product_id: 17_511_712
                }),
                "the slug, scheme, www and query are all decoration: {pasted}"
            );
        }
    }

    #[test]
    fn a_pasted_tes_page_becomes_the_canonical_identity_rather_than_the_page() {
        for pasted in [
            "https://www.tes.com/teaching-resource/fractions-revision-pack-13264370",
            "https://www.tes.com/teaching-resource/-13264370",
            "https://www.tes.com/api/v2/resources/13264370",
        ] {
            assert_eq!(
                parse_listing_url(Marketplace::Tes, pasted),
                Ok(RemoteListingId::Tes {
                    url: "https://www.tes.com/api/v2/resources/13264370".to_owned()
                }),
                "one resource has one spelling in the bind columns: {pasted}"
            );
        }
    }

    #[test]
    fn the_right_link_on_the_wrong_row_is_refused_by_name() {
        assert_eq!(
            parse_listing_url(
                Marketplace::Tpt,
                "https://www.tes.com/teaching-resource/pack-13264370"
            ),
            Err(UrlRefusal::WrongMarketplace {
                named: Marketplace::Tes
            }),
            "a Tes page is a real listing page, just not this mapping's"
        );
        assert_eq!(
            parse_listing_url(
                Marketplace::Tes,
                "https://www.teacherspayteachers.com/Product/x-17511712"
            ),
            Err(UrlRefusal::WrongMarketplace {
                named: Marketplace::Tpt
            })
        );
    }

    /// The failure the adapters' own parsers would wave through: they read the
    /// trailing digits of a `Location` header and validate no host at all.
    #[test]
    fn a_bare_number_or_a_foreign_host_is_not_a_listing() {
        for pasted in [
            "17511712",
            "https://example.com/Product/x-17511712",
            "https://www.teacherspayteachers.example.com/Product/x-17511712",
            "https://www.teacherspayteachers.com/Store/seller-17511712",
            "https://www.teacherspayteachers.com/Product/no-digits",
            "https://www.tes.com/teaching-resource/",
            "",
        ] {
            assert_eq!(
                parse_listing_url(Marketplace::Tpt, pasted),
                Err(UrlRefusal::Unrecognised),
                "nothing here is a TPT product page: {pasted}"
            );
        }
    }

    #[test]
    fn a_host_dressed_up_with_credentials_is_read_as_its_real_host() {
        assert_eq!(
            parse_listing_url(
                Marketplace::Tpt,
                "https://www.teacherspayteachers.com@example.com/Product/x-17511712"
            ),
            Err(UrlRefusal::Unrecognised),
            "the host is what follows the last @, not what precedes it"
        );
    }

    #[test]
    fn etsy_can_be_neither_rendered_nor_parsed_yet() {
        assert_eq!(
            parse_listing_url(Marketplace::Etsy, "https://www.etsy.com/listing/1234567890"),
            Err(UrlRefusal::Unrecognised),
            "no Etsy adapter exists, so its page shape is unobserved in both directions"
        );
    }

    /// The two directions are one fact, so a change to either that broke the
    /// other would fail here rather than in a seller's browser.
    #[test]
    fn parsing_a_rendered_page_returns_the_identifier_it_was_rendered_from() {
        for id in [
            RemoteListingId::Tpt {
                product_id: 17_511_712,
            },
            RemoteListingId::Tes {
                url: "https://www.tes.com/api/v2/resources/13264370".to_owned(),
            },
        ] {
            let page = listing_url(&id).expect("both marketplaces render a page");
            assert_eq!(
                parse_listing_url(id.marketplace(), &page),
                Ok(id.clone()),
                "render then parse is the identity for {id:?}"
            );
        }
    }
}
