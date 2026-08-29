//! Who the session belongs to, as TPT itself asserts it.
//!
//! This is the identity read the two-phase link's claim phase consumes: the
//! broker seals a credential and leaves the connection `linking`, the caller
//! reads the account back through a lease on that very connection, and the
//! claim writes the digest that takes the global exclusivity lock.
//!
//! Only a server-asserted identity is admissible, which is why this parses a
//! response rather than accepting a seller-typed value. A storefront
//! identifier a seller could type would be a denial-of-service primitive: any
//! account could claim a store it does not own, and the real owner would then
//! be unable to link at all.
//!
//! Which response is read matters as much as that one is. Two committed
//! cassettes carry an `author` on the `Store` type and they are not
//! interchangeable:
//!
//! `MyProductListings`, as [`crate::endpoints::my_product_listings_request`]
//! builds it, passes neither `sellerId` nor `resourceIds`, so the server
//! resolves the catalogue from the session alone. Its author is therefore the
//! session's own seller, by construction, whatever the caller wanted.
//!
//! `UploadPageProductQuery` takes a product id from the caller. Its author is
//! the author of whatever product was named, which is the session's seller
//! only when the caller happened to name their own product. Claiming on it
//! would let a caller who can name any product id take the exclusivity lock on
//! that product's storefront and lock its real owner out — the same
//! denial-of-service primitive a seller-typed identifier would be, reached by
//! a different route.
//!
//! [`read_seller_store_id`] therefore issues the listings read and nothing
//! else, and it is the only function the claim path may go through. The two
//! cassettes carrying different store ids is a sanitisation artefact of two
//! separately-captured fixtures, not a disagreement about where the field
//! lives; both put it at `author.id` under `__typename: "Store"`.

use serde_json::Value;
use tam_marketplace::transport::{HttpResponse, Transport, TransportError};

/// A TPT store, by the id the platform puts on the `Store` type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreId(pub String);

/// The `__typename` that marks an author object as a storefront rather than
/// some other principal, and the two field names read off it.
const STORE_TYPENAME: &str = "Store";
const AUTHOR_FIELD: &str = "author";
const TYPENAME_FIELD: &str = "__typename";
const ID_FIELD: &str = "id";

/// The store id in a GraphQL response that carries an authored product.
///
/// This is the parser, not the identity decision. It reports whose store the
/// response names, and it is the caller's responsibility that the response was
/// one the server scoped to the session — see the module note on why
/// `UploadPageProductQuery` must never be that response. Go through
/// [`read_seller_store_id`] rather than calling this on a body of your own
/// choosing.
///
/// Searched for rather than read at a fixed path because the two operations
/// that carry it nest it differently — `UploadPageProductQuery` puts the
/// product at `data.products[]` and `MyProductListings` puts it under a
/// results envelope — and both are committed cassettes this must keep reading.
/// The `__typename` check is what keeps the search precise: an `author` that
/// is not a `Store` is not this identity and is skipped rather than guessed
/// at.
///
/// Every product in one seller's own catalogue has the same author, so the
/// first match is the answer; a body carrying two different store ids is not
/// a response to a seller's own catalogue read and returns `None` rather than
/// picking one.
#[must_use]
pub fn seller_store_id(body: &[u8]) -> Option<StoreId> {
    let parsed: Value = serde_json::from_slice(body).ok()?;
    let mut found: Option<String> = None;
    collect_store_ids(&parsed, &mut found)?;
    found.map(StoreId)
}

/// Walks the body for `Store` authors. Returns `None` the moment two
/// different ids appear, because a body speaking for two storefronts cannot
/// name this session's.
fn collect_store_ids(value: &Value, found: &mut Option<String>) -> Option<()> {
    match value {
        Value::Object(fields) => {
            if let Some(id) = fields.get(AUTHOR_FIELD).and_then(store_id_of) {
                match found {
                    Some(seen) if *seen != id => return None,
                    Some(_) => {}
                    None => *found = Some(id),
                }
            }
            for nested in fields.values() {
                collect_store_ids(nested, found)?;
            }
            Some(())
        }
        Value::Array(items) => {
            for item in items {
                collect_store_ids(item, found)?;
            }
            Some(())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Some(()),
    }
}

/// The id of an author object, but only where the platform typed it a store.
fn store_id_of(author: &Value) -> Option<String> {
    let fields = author.as_object()?;
    if fields.get(TYPENAME_FIELD)?.as_str()? != STORE_TYPENAME {
        return None;
    }
    fields.get(ID_FIELD)?.as_str().map(str::to_owned)
}

/// Reads the seller's own store id back through a transport.
///
/// The one identity read the claim path may use. It issues
/// `MyProductListings` with no seller and no resource selector, so the
/// storefront it names is the session's own by the server's choice rather than
/// the caller's — which is what makes the result admissible as a claim.
///
/// `Ok(None)` is a seller whose catalogue is empty, which is a real state at
/// link time and not an error — a brand-new store has nothing authored to
/// read an author from, so it links without a lock until it has published
/// something. That gap is why the claim is a separate phase rather than part
/// of the link.
pub async fn read_seller_store_id<T: Transport>(
    transport: &T,
) -> Result<Option<StoreId>, TransportError> {
    let response: HttpResponse = transport
        .send(crate::endpoints::my_product_listings_request(1, 0))
        .await?;
    Ok(seller_store_id(&response.body))
}

#[cfg(test)]
mod tests {
    use super::{seller_store_id, StoreId};
    use serde_json::Value;

    /// Pulls one interaction's response body out of a committed cassette, so
    /// the extraction is proved against the bytes TPT actually sent rather
    /// than against a hand-written sample of them.
    fn cassette_body(cassette: &str, index: usize) -> Vec<u8> {
        let parsed: Value = serde_json::from_str(cassette).expect("the cassette parses");
        parsed["interactions"][index]["response"]["body"]
            .as_str()
            .expect("the recorded body is a string")
            .as_bytes()
            .to_vec()
    }

    /// The upload-page shape parses, and that is precisely why the module
    /// note exists: the parser cannot tell whose product it was handed, so the
    /// safety comes from `read_seller_store_id` choosing the request, not from
    /// the parser refusing the body.
    #[test]
    fn the_parser_reads_the_upload_page_shape_which_is_why_it_is_not_the_identity_source() {
        let body = cassette_body(
            include_str!("../tests/cassettes/upload_page_product.json"),
            0,
        );
        assert_eq!(
            seller_store_id(&body),
            Some(StoreId("900000001".to_owned())),
            "the author parses out of this shape too, so nothing in the parser stops a \
             caller-named product's owner from being claimed — the request choice does"
        );
    }

    #[test]
    fn the_store_id_is_read_from_the_listings_cassette_the_claim_path_uses() {
        let body = cassette_body(
            include_str!("../tests/cassettes/my_product_listings.json"),
            0,
        );
        assert_eq!(
            seller_store_id(&body),
            Some(StoreId("90000001".to_owned())),
            "MyProductListings nests the author under a results envelope, and this is the \
             only response the claim path reads: the server scoped it to the session, so its \
             author is the session's own seller by construction"
        );
    }

    #[test]
    fn an_author_that_is_not_a_store_is_not_an_identity() {
        let body = br#"{"data":{"product":{"author":{"__typename":"User","id":"7"}}}}"#;
        assert_eq!(
            seller_store_id(body),
            None,
            "claiming on a principal TPT did not type as a storefront would lock the wrong \
             account, so an unrecognised author is skipped rather than guessed at"
        );
    }

    #[test]
    fn two_different_stores_in_one_body_name_nobody() {
        let body = br#"{"results":[
            {"author":{"__typename":"Store","id":"1"}},
            {"author":{"__typename":"Store","id":"2"}}
        ]}"#;
        assert_eq!(
            seller_store_id(body),
            None,
            "a body speaking for two storefronts is not a seller's own catalogue read, and \
             picking either would take a lock on an account this session may not hold"
        );
    }

    #[test]
    fn the_same_store_repeated_is_still_that_store() {
        let body = br#"{"results":[
            {"author":{"__typename":"Store","id":"90000001"}},
            {"author":{"__typename":"Store","id":"90000001"}}
        ]}"#;
        assert_eq!(
            seller_store_id(body),
            Some(StoreId("90000001".to_owned())),
            "every row of one seller's catalogue repeats the same author, which is the \
             ordinary case and must not read as a conflict"
        );
    }

    #[test]
    fn a_body_with_no_author_names_nobody() {
        assert_eq!(
            seller_store_id(br#"{"data":{"products":[]}}"#),
            None,
            "an empty catalogue names no author, which is a real state at link time"
        );
    }

    #[test]
    fn a_body_that_is_not_json_names_nobody() {
        assert_eq!(
            seller_store_id(b"<html>challenge</html>"),
            None,
            "a Cloudflare interstitial must not be mistaken for an identity"
        );
    }

    /// A transport that answers one canned body and records what it was asked
    /// for, so the identity read's own request shape is under test rather than
    /// assumed. `OnceLock` because exactly one request is expected and it is
    /// the only shared-mutable state the trait's `Sync` bound admits without a
    /// lock.
    #[derive(Default)]
    struct RecordingTransport {
        seen: std::sync::OnceLock<(String, String)>,
        body: Vec<u8>,
    }

    impl tam_marketplace::transport::Transport for RecordingTransport {
        async fn send(
            &self,
            request: tam_marketplace::transport::HttpRequest,
        ) -> Result<
            tam_marketplace::transport::HttpResponse,
            tam_marketplace::transport::TransportError,
        > {
            // The variables object rather than the whole body: the captured
            // query text *declares* `$sellerId` and `$resourceIds`, and what
            // decides whose catalogue the server resolves is whether either is
            // supplied a value.
            let variables = match &request.body {
                tam_marketplace::transport::RequestBody::Json(value) => value
                    .get("variables")
                    .map_or_else(|| "<none>".to_owned(), ToString::to_string),
                body @ (tam_marketplace::transport::RequestBody::Empty
                | tam_marketplace::transport::RequestBody::Bytes(_)
                | tam_marketplace::transport::RequestBody::Multipart { .. }) => {
                    format!("{body:?}")
                }
            };
            let recorded = (request.url.clone(), variables);
            drop(self.seen.set(recorded));
            Ok(tam_marketplace::transport::HttpResponse::plain(
                200,
                self.body.clone(),
            ))
        }
    }

    #[test]
    fn the_identity_read_asks_the_server_who_the_session_is() {
        let transport = RecordingTransport {
            seen: std::sync::OnceLock::new(),
            body: cassette_body(
                include_str!("../tests/cassettes/my_product_listings.json"),
                0,
            ),
        };
        let found = futures::executor::block_on(super::read_seller_store_id(&transport))
            .expect("the read succeeds");
        assert_eq!(
            found,
            Some(StoreId("90000001".to_owned())),
            "the identity comes back from the response"
        );

        let (url, variables) = transport.seen.get().expect("exactly one request was made");
        assert!(
            url.contains("opname=MyProductListings"),
            "the claim path must read the session-scoped enumeration, not a product the caller \
             named — an UploadPageProductQuery read would let whoever picks the product id \
             claim that product's storefront: {url}"
        );
        assert!(
            !variables.contains("sellerId") && !variables.contains("resourceIds"),
            "the query declares both variables and the request must supply neither, or the \
             storefront it names would be the caller's choice rather than the server's: \
             {variables}"
        );
    }
}
