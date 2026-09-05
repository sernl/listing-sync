//! The catalogue export over the wire: what the document says, what it is
//! called, and what it must never contain.
//!
//! Two tenants throughout, because the property worth proving is the one a
//! single-tenant fixture cannot express: an export carries the caller's own
//! catalogue and no row of anyone else's, whoever asks.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::{router, AppState, Config, SESSION_COOKIE};
use tam_domain::{Binding, FieldPolicies, FieldPolicy, Mapping, PublishMode, Verification};
use tam_marketplace::{RemoteLifecycle, RemoteListingId};
use tam_storage::{LabelRepo, MappingRepo, ProductRepo, SessionRepo, SessionToken};
use tam_types::{
    ContentHash, CopyFormat, Currency, FileBytes, FileId, FileKind, FileRole, InventoryId,
    ListingCopy, MappingId, Money, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile,
    ProductId, ScanOutcome, Timestamp, Title, UserId, Uuid,
};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const USER_B: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const PRODUCT_A: ProductId = ProductId(Uuid([0x01; 16]));
const PRODUCT_B: ProductId = ProductId(Uuid([0x02; 16]));

/// The clock the router reads, which is what names the file: 2026-09-05.
const NOW: Timestamp = Timestamp(1_788_607_353_000);
/// When the fixture catalogue was written: 2026-08-01T09:15:00Z.
const SEEDED: Timestamp = Timestamp(1_785_575_700_000);
/// After [`NOW`], so the fixture session is live when the router reads it.
const SESSION_EXPIRY: Timestamp = Timestamp(1_790_000_000_000);

/// A title that is hostile in both of the ways a cell can be: it opens with a
/// formula character and it carries a comma.
const TITLE_A: &str = "=Fractions, decimals";
const TITLE_B: &str = "Org B place value pack";

fn state(pool: PgPool) -> AppState {
    AppState {
        pool,
        config: Config::default(),
        wall: || NOW,
        auth: None,
        backoffice: None,
        blobs: None,
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool, org: OrgId, user: UserId, token: &SessionToken, name: &str) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .bind(name)
        .execute(pool)
        .await
        .expect("the org seeds");
    let sessions = SessionRepo::new(pool.clone());
    sessions
        .create_user(org, user, &format!("{name}@example.test"), SEEDED)
        .await
        .expect("the user provisions");
    sessions
        .mint(token, user, SESSION_EXPIRY, SEEDED)
        .await
        .expect("the session mints");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_product(pool: &PgPool, org: OrgId, product: ProductId, title: &str, at: Timestamp) {
    let price = Money::new(499, Currency::Gbp).expect("the fixture price is positive");
    ProductRepo::new(pool.clone())
        .insert(
            org,
            &tam_domain::CanonicalProduct {
                id: product,
                org,
                title: Title(title.to_owned()),
                body: ListingCopy {
                    body: "Fixture body.".to_owned(),
                    format: CopyFormat::Markdown,
                },
                payload: Some(PayloadSet::new(
                    ProductFile {
                        id: FileId(Uuid(product.0 .0)),
                        role: FileRole::Payload,
                        kind: FileKind::Pdf,
                        bytes: FileBytes::Held {
                            hash: ContentHash([0x51; 32]),
                            byte_len: 4,
                            scan: ScanOutcome::Pending,
                        },
                    },
                    vec![],
                )),
                cover: None,
                previews: vec![],
                subjects: vec![],
                grades: tam_domain::GradeDeclaration {
                    source: tam_domain::DeclarationSource::Seller,
                    raw: vec![],
                    derived: None,
                },
                price: PriceIntent::Paid(price),
                rights: tam_domain::RightsDeclaration::Unstated,
                native_residue: vec![],
            },
            at,
        )
        .await
        .expect("the product inserts");
}

/// Where one listing stands, which is the only part of a fixture mapping
/// these tests vary.
struct Listing {
    binding: Binding,
    price_rule: PriceRule,
    lifecycle: RemoteLifecycle,
}

fn mapping(
    id: MappingId,
    org: OrgId,
    product: ProductId,
    inventory: InventoryId,
    listing: Listing,
) -> Mapping {
    Mapping {
        id,
        org,
        product,
        inventory,
        binding: listing.binding,
        policies: FieldPolicies {
            title: FieldPolicy::Managed,
            description: FieldPolicy::Managed,
            price: FieldPolicy::Managed,
            taxonomy: FieldPolicy::Managed,
            grades: FieldPolicy::Managed,
            files: FieldPolicy::Managed,
        },
        price_rule: listing.price_rule,
        publish: PublishMode::DryRun,
        lifecycle: listing.lifecycle,
    }
}

/// A distinct product identifier per index, so a fixture larger than a page
/// needs no table of literals.
fn numbered_product(index: u16) -> ProductId {
    let mut bytes = [0x10; 16];
    let [high, low] = index.to_be_bytes();
    bytes[0] = high;
    bytes[1] = low;
    ProductId(Uuid(bytes))
}

fn bound_to_tpt() -> Binding {
    Binding::Bound {
        id: RemoteListingId::Tpt {
            product_id: 17_511_712,
        },
        first_seen: SEEDED,
        verified: Verification::Stale { since: SEEDED },
    }
}

struct Answer {
    status: StatusCode,
    content_type: Option<String>,
    disposition: Option<String>,
    body: Vec<u8>,
}

impl Answer {
    #[expect(
        clippy::expect_used,
        reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
    )]
    fn text(&self) -> String {
        String::from_utf8(self.body.clone()).expect("the document is UTF-8")
    }

    /// The document's records, read the way a CSV reader reads them rather
    /// than by splitting on CRLF.
    ///
    /// The difference is the whole point: a quoted field may legitimately
    /// carry a CRLF, and a helper that split on every one would report a
    /// correct two-record document as three and make the row assertions
    /// meaningless. Nothing is filtered out either, so a blank record fails an
    /// assertion instead of disappearing from the count.
    fn records(&self) -> Vec<String> {
        let text = self.text();
        let mut records = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        let mut characters = text.chars().peekable();
        while let Some(character) = characters.next() {
            match character {
                '"' => {
                    quoted = !quoted;
                    current.push(character);
                }
                '\r' if !quoted && characters.peek() == Some(&'\n') => {
                    characters.next();
                    records.push(core::mem::take(&mut current));
                }
                _ => current.push(character),
            }
        }
        if !current.is_empty() {
            records.push(current);
        }
        records
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn export(pool: PgPool, token: &SessionToken) -> Answer {
    let request = Request::builder()
        .method(Method::GET)
        .uri("/v1/products/export")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        )
        .body(Body::empty())
        .expect("the request builds");
    let response = router(state(pool))
        .oneshot(request)
        .await
        .expect("the router serves");
    let status = response.status();
    let read = |name: &header::HeaderName| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let content_type = read(&header::CONTENT_TYPE);
    let disposition = read(&header::CONTENT_DISPOSITION);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes()
        .to_vec();
    Answer {
        status,
        content_type,
        disposition,
        body,
    }
}

const HEADER: &str = "Resource ID,Title,Price,Currency,Labels,Created,Updated,\
                      TES GB status,TES GB price,TES GB link,\
                      TES US status,TES US price,TES US link,\
                      TES NZ status,TES NZ price,TES NZ link,\
                      TPT status,TPT price,TPT link,\
                      Etsy status,Etsy price,Etsy link";

/// One tenant's whole catalogue, cell for cell: the header a spreadsheet
/// reads, the price and labels this seller holds, and one column group per
/// inventory whether or not a listing exists in it.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_export_writes_one_row_per_resource_with_every_inventory_beside_it(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_product(&pool, ORG_A, PRODUCT_A, TITLE_A, SEEDED).await;
    LabelRepo::new(pool.clone())
        .set_for_product(
            ORG_A,
            PRODUCT_A,
            &["Year 5".to_owned(), "Algebra".to_owned()],
            SEEDED,
        )
        .await
        .expect("the labels attach");
    let listed = Money::new(599, Currency::Usd).expect("the listed price is positive");
    let mappings = MappingRepo::new(pool.clone());
    mappings
        .insert(
            ORG_A,
            &mapping(
                MappingId(Uuid([0x31; 16])),
                ORG_A,
                PRODUCT_A,
                InventoryId::Tpt,
                Listing {
                    binding: bound_to_tpt(),
                    price_rule: PriceRule::Explicit(PriceIntent::Paid(listed)),
                    lifecycle: RemoteLifecycle::Live { since: SEEDED },
                },
            ),
            0,
            SEEDED,
        )
        .await
        .expect("the bound mapping inserts");
    mappings
        .insert(
            ORG_A,
            &mapping(
                MappingId(Uuid([0x32; 16])),
                ORG_A,
                PRODUCT_A,
                InventoryId::TesGb,
                Listing {
                    binding: Binding::Unbound,
                    price_rule: PriceRule::Explicit(PriceIntent::Free),
                    lifecycle: RemoteLifecycle::Absent,
                },
            ),
            0,
            SEEDED,
        )
        .await
        .expect("the unbound mapping inserts");

    let answer = export(pool, &TOKEN_A).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        answer.content_type.as_deref(),
        Some("text/csv; charset=utf-8"),
        "the document announces itself as the CSV it is"
    );
    assert_eq!(
        answer.disposition.as_deref(),
        Some("attachment; filename=\"teachouse-resources-2026-09-05.csv\""),
        "the file is named for the day the clock reads"
    );
    assert!(
        !answer.body.starts_with(&[0xEF, 0xBB, 0xBF]),
        "a byte-order mark would be the first thing a parser choked on"
    );

    let records = answer.records();
    assert_eq!(records.len(), 2, "one header and one resource");
    assert_eq!(records[0], HEADER);

    let expected = [
        PRODUCT_A.0.to_hyphenated(),
        // Defused, because a spreadsheet opening this file would otherwise
        // evaluate the title; quoted, because it carries a comma.
        "\"'=Fractions, decimals\"".to_owned(),
        "4.99".to_owned(),
        "GBP".to_owned(),
        "Algebra; Year 5".to_owned(),
        "2026-08-01T09:15:00Z".to_owned(),
        "2026-08-01T09:15:00Z".to_owned(),
        "unsent".to_owned(),
        "Free".to_owned(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        "live".to_owned(),
        "5.99 USD".to_owned(),
        "https://www.teacherspayteachers.com/Product/listing-17511712".to_owned(),
        String::new(),
        String::new(),
        String::new(),
    ]
    .join(",");
    assert_eq!(
        records[1], expected,
        "the resource row reads back cell for cell"
    );
}

/// The tenancy fence, from both sides: each seller's export is their own
/// catalogue, and neither document carries so much as the other's identifier.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_tenants_export_never_carries_another_tenants_catalogue(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, "org-b").await;
    seed_product(&pool, ORG_A, PRODUCT_A, TITLE_A, SEEDED).await;
    seed_product(&pool, ORG_B, PRODUCT_B, TITLE_B, SEEDED).await;
    MappingRepo::new(pool.clone())
        .insert(
            ORG_B,
            &mapping(
                MappingId(Uuid([0x33; 16])),
                ORG_B,
                PRODUCT_B,
                InventoryId::Tpt,
                Listing {
                    binding: bound_to_tpt(),
                    price_rule: PriceRule::Explicit(PriceIntent::Free),
                    lifecycle: RemoteLifecycle::Live { since: SEEDED },
                },
            ),
            0,
            SEEDED,
        )
        .await
        .expect("the other tenant's mapping inserts");

    let theirs = export(pool.clone(), &TOKEN_A).await;
    assert_eq!(theirs.status, StatusCode::OK);
    let document = theirs.text();
    assert_eq!(
        theirs.records().len(),
        2,
        "one header and this tenant's one row"
    );
    assert!(
        !document.contains(TITLE_B),
        "another tenant's title reached this document"
    );
    assert!(
        !document.contains(&PRODUCT_B.0.to_hyphenated()),
        "another tenant's identifier reached this document"
    );

    let ours = export(pool, &TOKEN_B).await;
    assert_eq!(ours.status, StatusCode::OK);
    let document = ours.text();
    assert!(
        document.contains(TITLE_B),
        "a seller's own resource is missing from their own export"
    );
    assert!(
        !document.contains(&PRODUCT_A.0.to_hyphenated()),
        "the first tenant's identifier reached the second's document"
    );
}

/// A seller with nothing filed gets the header and nothing else. That this
/// answers a document at all is what proves `export` reaches this route rather
/// than `/{version}/products/{product}`, which would refuse it as a malformed
/// identifier.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_empty_catalogue_exports_a_header_and_nothing_else(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;

    let answer = export(pool, &TOKEN_A).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.records(), vec![HEADER.to_owned()]);
}

/// A catalogue one resource longer than a page, which is the only shape that
/// falsifies the walk.
///
/// The page size is read from the handler rather than restated, so the test
/// keeps crossing the boundary if the number moves. A walk that stopped one
/// page early would silently truncate every large catalogue, and a walk that
/// failed to advance its cursor would repeat the first page forever; both ship
/// green against a one-page fixture.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_catalogue_longer_than_one_page_exports_every_resource_once(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let seeded = u16::try_from(tam_api::export::PAGE + 1).expect("a page and one fits a u16");
    let mut expected: Vec<String> = Vec::with_capacity(usize::from(seeded));
    for index in 0..seeded {
        let product = numbered_product(index);
        // A distinct instant each, so the walk is ordered by `created_at` and
        // not only by the identifier that breaks its ties.
        seed_product(
            &pool,
            ORG_A,
            product,
            "Fixture product",
            Timestamp(SEEDED.0 + i64::from(index)),
        )
        .await;
        expected.push(product.0.to_hyphenated());
    }

    let answer = export(pool, &TOKEN_A).await;
    assert_eq!(answer.status, StatusCode::OK);
    let records = answer.records();
    assert_eq!(
        records.len(),
        usize::from(seeded) + 1,
        "one header and every seeded resource exactly once"
    );
    let exported: Vec<String> = records
        .iter()
        .skip(1)
        .filter_map(|record| record.split(',').next().map(str::to_owned))
        .collect();
    assert_eq!(
        exported, expected,
        "the walk crosses its page boundary without dropping, repeating or reordering a resource"
    );
}

/// A title carrying a record separator, over the wire.
///
/// `field()` quotes it, which its own unit test proves; what this proves is
/// that the document a client receives still parses as one record per
/// resource, which splitting on CRLF would not show.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_title_carrying_a_record_separator_stays_one_record(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_product(&pool, ORG_A, PRODUCT_A, "Two\r\nlines", SEEDED).await;
    LabelRepo::new(pool.clone())
        .set_for_product(ORG_A, PRODUCT_A, &["Year 5; Algebra".to_owned()], SEEDED)
        .await
        .expect("the label attaches");

    let answer = export(pool, &TOKEN_A).await;
    assert_eq!(answer.status, StatusCode::OK);
    let records = answer.records();
    assert_eq!(records.len(), 2, "one header and one resource");
    assert!(
        records[1].starts_with(&format!(
            "{},\"Two\r\nlines\",4.99,GBP,Year 5;; Algebra,",
            PRODUCT_A.0.to_hyphenated()
        )),
        "the title is quoted whole and the label's own separator is doubled: {}",
        records[1]
    );
}

/// The same session floor as the catalogue listing: no cookie, no document.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_export_without_a_session_is_refused(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let request = Request::builder()
        .method(Method::GET)
        .uri("/v1/products/export")
        .body(Body::empty())
        .expect("the request builds");
    let response = router(state(pool))
        .oneshot(request)
        .await
        .expect("the router serves");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
