//! The standards search over the wire: what it serves from the committed
//! corpus, what it withholds, and the licence notices it carries.
//!
//! The withholding is the half worth testing hardest. A TPT node id is a
//! search-index identifier and exactly the kind that gets rebuilt, so one no
//! current capture vouches for must not reach a seller, and a deployment told
//! about no capture at all must serve none.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::product::standards::{prime, StandardsSearchView, StandardsState};
use tam_api::{router, AppState, Config, SESSION_COOKIE};
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const USER: UserId = UserId(Uuid([0x0A; 16]));
const TOKEN: SessionToken = SessionToken([0x41; 32]);
const NOW: Timestamp = Timestamp(5_000);

/// Common Core's own jurisdiction id, which the create form offers.
const CCSS: u32 = 3054;

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn state(pool: PgPool) -> AppState {
    prime().expect("the committed corpus parses");
    AppState {
        exchange_rates: None,
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
async fn provision(pool: &PgPool) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(pool)
        .await
        .expect("the org seeds");
    let sessions = SessionRepo::new(pool.clone());
    sessions
        .create_user(ORG, USER, "a@example.test", Timestamp(1_000))
        .await
        .expect("the user provisions");
    sessions
        .mint(&TOKEN, USER, Timestamp(100_000), Timestamp(1_000))
        .await
        .expect("the session mints");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn search(pool: PgPool, query: &str) -> (StatusCode, Vec<u8>) {
    let request = Request::builder()
        .method(Method::GET)
        .uri(query)
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", TOKEN.to_hex()),
        )
        .body(Body::empty())
        .expect("the request builds");
    let response = router(state(pool))
        .oneshot(request)
        .await
        .expect("the router serves");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes()
        .to_vec();
    (status, body)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> T {
    serde_json::from_slice(body).expect("the body parses")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_search_answers_from_the_ingested_corpus(pool: PgPool) {
    provision(&pool).await;
    let (status, body) = search(
        pool,
        &format!("/v1/standards/search?framework={CCSS}&q=fraction"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let view: StandardsSearchView = parse(&body);
    assert_eq!(
        view.state,
        StandardsState::Ingested,
        "the corpus is committed, so the route no longer answers not-ingested"
    );
    assert_eq!(view.framework, CCSS);
    assert!(
        !view.items.is_empty(),
        "a common word matches somewhere in eleven thousand standards"
    );
    for item in &view.items {
        assert!(!item.code.is_empty(), "a result always names its code");
        assert!(
            !item.statement.is_empty(),
            "the statement is what a seller reads, and it is served verbatim"
        );
        // Each item states the framework it came from rather than the one that
        // was asked for, so this catches a corpus leaking between frameworks
        // instead of merely restating the request back at itself.
        assert_eq!(
            item.framework, CCSS,
            "a search of one framework answers from that framework alone"
        );
    }
}

/// The whole point of the crawl window. The table is committed empty and no
/// window is configured, so every id is withheld twice over; a result carrying
/// one would mean an unverified search-index identifier reached a seller.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn no_node_id_is_served_without_a_capture_to_vouch_for_it(pool: PgPool) {
    provision(&pool).await;
    let (status, body) = search(
        pool,
        &format!("/v1/standards/search?framework={CCSS}&q=fraction"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let view: StandardsSearchView = parse(&body);
    assert!(
        !view.items.is_empty(),
        "the query matched something to check"
    );
    assert!(
        view.items.iter().all(|item| item.tpt_node_id.is_none()),
        "no capture vouches for any id yet, so none is served"
    );
}

/// Texas and Virginia are the case the handler's two-halves rule exists for:
/// their owners impose no notice, and the mirror's CC BY attribution is the
/// whole obligation. A response carrying none would publish mirrored data
/// unattributed.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn every_framework_serves_the_notices_its_licence_obliges(pool: PgPool) {
    provision(&pool).await;
    for framework in [3054u32, 3055, 3326, 5785] {
        let (status, body) = search(
            pool.clone(),
            &format!("/v1/standards/search?framework={framework}&q=a"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "framework {framework}");
        let view: StandardsSearchView = parse(&body);
        assert!(
            !view.notices.is_empty(),
            "framework {framework} displays under a licence that obliges a notice"
        );
        for notice in &view.notices {
            assert!(!notice.text.is_empty());
            assert!(!notice.placement.is_empty());
        }
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_framework_the_form_does_not_offer_is_refused(pool: PgPool) {
    provision(&pool).await;
    let (status, _body) = search(pool, "/v1/standards/search?framework=9999&q=fraction").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

/// An empty query answers nothing rather than the whole corpus: eleven
/// thousand rows is not a page a person reads, and a picker that dumped them
/// would be slower and less useful than one that waits for a word.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_empty_query_answers_no_items_rather_than_everything(pool: PgPool) {
    provision(&pool).await;
    let (status, body) = search(pool, &format!("/v1/standards/search?framework={CCSS}")).await;
    assert_eq!(status, StatusCode::OK);
    let view: StandardsSearchView = parse(&body);
    assert!(view.items.is_empty());
    assert!(
        !view.notices.is_empty(),
        "the licence obligation stands whether or not anything matched"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_surface_is_closed_without_a_session(pool: PgPool) {
    provision(&pool).await;
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/standards/search?framework={CCSS}&q=fraction"))
        .body(Body::empty())
        .expect("the request builds");
    let response = router(state(pool))
        .oneshot(request)
        .await
        .expect("the router serves");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// A code does not identify a standard. 814 TEKS codes name more than one
/// addressable node with a different statement, so a client keying on the code
/// can attach one standard's checkbox to another's row; `source_guid` is what
/// the client keys on, and it must be unique across everything served.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn every_item_carries_an_identifier_a_client_can_key_on(pool: PgPool) {
    provision(&pool).await;
    // Texas is the framework whose codes repeat, so it is the one worth asking.
    let (status, body) = search(pool, "/v1/standards/search?framework=3326&q=1.1").await;
    assert_eq!(status, StatusCode::OK);
    let view: StandardsSearchView = parse(&body);
    assert!(
        !view.items.is_empty(),
        "the query matched something to check"
    );

    let mut guids: Vec<&str> = view
        .items
        .iter()
        .map(|item| item.source_guid.as_str())
        .collect();
    assert!(
        guids.iter().all(|guid| !guid.is_empty()),
        "every item names the mirror's own identifier"
    );
    let served = guids.len();
    guids.sort_unstable();
    guids.dedup();
    assert_eq!(
        guids.len(),
        served,
        "no two items share an identifier, so keying on it is safe even where codes repeat"
    );
}

/// The scan is bounded before it runs. Without this an 8 KB query cost about
/// 370 ms of CPU and a 32 KB one about 1.5 s, on a request any signed-in member
/// could make as often as they liked.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_oversized_query_is_refused_before_anything_is_scanned(pool: PgPool) {
    provision(&pool).await;

    let long: String = "a".repeat(200);
    let (status, _body) = search(
        pool.clone(),
        &format!("/v1/standards/search?framework={CCSS}&q={long}"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a query longer than any standard's code or title is refused"
    );

    let many: String = (0..20)
        .map(|n| format!("word{n}"))
        .collect::<Vec<_>>()
        .join("+");
    let (status, _body) = search(
        pool,
        &format!("/v1/standards/search?framework={CCSS}&q={many}"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a query carrying more words than a title holds is refused"
    );
}
