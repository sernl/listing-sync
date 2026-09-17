//! The seller's own files over the wire: which machine holds which, which
//! resources use them, and what the file browser's filters and paging answer.
//!
//! Nothing here carries a byte of a file, and the fixture could not make one
//! travel: the device reports digests on its heartbeat and the catalogue
//! rows name the same digests, which is the whole of what the server knows.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::library::{LibraryFileView, LibraryView};
use tam_api::{router, AppState, Config, WallClock, SESSION_COOKIE};
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const LAPTOP: &str = "11112222333344445555666677778888";
const PHONE: &str = "99998888777766665555444433332222";
const NOW: Timestamp = Timestamp(1_756_000_000_000);

/// The instant every fixture check-in and most reads happen at.
fn t0() -> Timestamp {
    NOW
}

/// Twenty minutes later: past the ten-minute online window, so a machine
/// that checked in at `t0` reads as offline rather than as gone.
fn much_later() -> Timestamp {
    Timestamp(NOW.0 + 20 * 60 * 1_000)
}

/// Sixty-four lowercase hex characters, the way the wire spells a digest.
fn digest(byte: u8) -> String {
    format!("{byte:02x}").repeat(32)
}

fn state(pool: PgPool, wall: WallClock) -> AppState {
    AppState {
        exchange_rates: None,
        pool,
        config: Config::default(),
        wall,
        auth: None,
        backoffice: None,
        blobs: None,
    }
}

struct Answer {
    status: StatusCode,
    body: Vec<u8>,
}

impl Answer {
    #[expect(
        clippy::expect_used,
        reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
    )]
    fn json<T: serde::de::DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body).expect("the answer body parses")
    }
}

struct Call<'a> {
    method: Method,
    path: &'a str,
    token: &'a SessionToken,
    body: Option<serde_json::Value>,
    wall: WallClock,
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(pool: PgPool, call: Call<'_>) -> Answer {
    let request = Request::builder()
        .method(call.method)
        .uri(call.path)
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", call.token.to_hex()),
        );
    let request = match call.body {
        Some(json) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json.to_string())),
        None => request.body(Body::empty()),
    }
    .expect("the request builds");
    let response = router(state(pool, call.wall))
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
    Answer { status, body }
}

/// One resource carrying one payload file of the given digest. `deleted`
/// marks the resource removed, which is the state a link must not survive.
async fn resource(
    pool: &PgPool,
    org: OrgId,
    (id, title): (u8, &str),
    (hash, file_name): (u8, &str),
    deleted: bool,
) -> Result<(), sqlx::Error> {
    let org = uuid::Uuid::from_bytes(org.0 .0);
    let product = uuid::Uuid::from_bytes([id; 16]);
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(org.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO blob (org_id, hash, byte_len, object_key, dek_key_version, first_seen_at) \
         VALUES ($1, $2, 2048, 'fixture', 1, now()) ON CONFLICT DO NOTHING",
    )
    .bind(org)
    .bind([hash; 32].as_slice())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "WITH inserted AS ( \
             INSERT INTO product (org_id, id, title, body, body_format, price_kind, \
                 rights_state, created_at, updated_at, deleted_at) \
             VALUES ($1, $2, $3, 'body', 'markdown', 'free', 'unstated', \
                 now(), now(), CASE WHEN $4 THEN now() ELSE NULL END) \
             RETURNING org_id, id \
         ) INSERT INTO grade_declaration (org_id, product_id, source) \
           SELECT org_id, id, 'seller' FROM inserted",
    )
    .bind(org)
    .bind(product)
    .bind(title)
    .bind(deleted)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO product_file (org_id, id, product_id, position, role, kind, \
             hash, name, scan_state, scanned_at, created_at) \
         VALUES ($1, $2, $3, 0, 'payload', 'pdf', $4, $5, 'clean', now(), now())",
    )
    .bind(org)
    .bind(uuid::Uuid::from_bytes([id.wrapping_add(0x70); 16]))
    .bind(product)
    .bind([hash; 32].as_slice())
    .bind(file_name)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    for (org, name) in [(ORG_A, "org-a"), (ORG_B, "org-b")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the org seeds");
        // Several machines per tenant, which the free allowance does not
        // carry: this suite is about the file browser rather than the plan
        // gate in front of registering a second computer.
        tam_storage::EntitlementRepo::new(pool.clone())
            .grant(
                org,
                &tam_storage::NewGrant {
                    id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                    plan: tam_limits::Plan::Studio,
                    rung: None,
                    granted_by: tam_storage::GrantedBy::Operator,
                    grantor_user: None,
                    reason: Some("the file browser's fixture tenant"),
                    source_ref: None,
                    granted_at: Timestamp(1_000),
                    expires_at: None,
                },
            )
            .await
            .expect("the fixture grant seeds");
    }
    let sessions = SessionRepo::new(pool.clone());
    for (org, user, email, token) in [
        (ORG_A, UserId(Uuid([0x0A; 16])), "a@example.test", TOKEN_A),
        (ORG_B, UserId(Uuid([0x0B; 16])), "b@example.test", TOKEN_B),
    ] {
        sessions
            .create_user(org, user, email, Timestamp(1_000))
            .await
            .expect("the user provisions");
        sessions
            .mint(&token, user, Timestamp(9_000_000_000_000), Timestamp(1_000))
            .await
            .expect("the session mints");
    }
}

/// Registers one machine and has it report the digests it holds, which is
/// the only way a holding is ever recorded.
async fn machine(pool: &PgPool, token: &SessionToken, id: &str, name: &str, holds: &[u8]) {
    let registered = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: "/v1/devices",
            token,
            body: Some(serde_json::json!({
                "id": id,
                "name": name,
                "os": "windows",
                "arch": "x86_64",
                "app_version": "0.1.0",
            })),
            wall: t0,
        },
    )
    .await;
    assert_eq!(
        registered.status,
        StatusCode::OK,
        "the fixture machine must register"
    );
    let holdings: Vec<serde_json::Value> = holds
        .iter()
        .map(|byte| serde_json::json!({ "hash": digest(*byte), "byte_len": 2048 }))
        .collect();
    let beat = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{id}/heartbeat"),
            token,
            body: Some(serde_json::json!({
                "sessions": [],
                "library": {
                    "node_id": format!("node-{id}"),
                    "direct_addrs": ["127.0.0.1:4001"],
                    "holdings": holdings,
                },
            })),
            wall: t0,
        },
    )
    .await;
    assert_eq!(
        beat.status,
        StatusCode::OK,
        "the fixture check-in must land, or the holdings it reports prove nothing"
    );
}

async fn library(pool: &PgPool, token: &SessionToken, query: &str, wall: WallClock) -> LibraryView {
    let answer = call(
        pool.clone(),
        Call {
            method: Method::GET,
            path: &format!("/v1/library{query}"),
            token,
            body: None,
            wall,
        },
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "the library lists");
    answer.json()
}

fn find<'a>(view: &'a LibraryView, hash: &str) -> Option<&'a LibraryFileView> {
    view.files.iter().find(|file| file.hash == hash)
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_file_names_every_live_resource_using_it_and_no_others(pool: PgPool) {
    provision(&pool).await;
    // One digest, three resources of this seller's carrying it: two live and
    // one deleted. Reuse is the ordinary case — a teacher's cover sheet in
    // every pack — and the deleted one is the link that must not survive.
    resource(&pool, ORG_A, (0x11, "Fractions"), (0xF1, "pack.pdf"), false)
        .await
        .expect("the resource fixture seeds");
    resource(&pool, ORG_A, (0x12, "Decimals"), (0xF1, "pack.pdf"), false)
        .await
        .expect("the resource fixture seeds");
    resource(
        &pool,
        ORG_A,
        (0x13, "Retired pack"),
        (0xF1, "pack.pdf"),
        true,
    )
    .await
    .expect("the resource fixture seeds");
    // The other tenant's resource carries the very same digest, which is the
    // case a join on hash alone would leak.
    resource(
        &pool,
        ORG_B,
        (0x21, "Another seller's pack"),
        (0xF1, "pack.pdf"),
        false,
    )
    .await
    .expect("the resource fixture seeds");
    machine(&pool, &TOKEN_A, LAPTOP, "founder-pc", &[0xF1]).await;

    let view = library(&pool, &TOKEN_A, "", t0).await;
    let file = find(&view, &digest(0xF1)).expect("the requested file is listed");
    let mut titles: Vec<&str> = file
        .resources
        .iter()
        .map(|resource| resource.title.as_str())
        .collect();
    titles.sort_unstable();
    assert_eq!(
        titles,
        ["Decimals", "Fractions"],
        "every live resource using the file, the deleted one and the other tenant's excluded"
    );
    assert_eq!(
        file.resources
            .iter()
            .filter(|resource| resource.id == uuid::Uuid::from_bytes([0x11; 16]).to_string())
            .count(),
        1,
        "the identifier is the one /resources/{{id}} takes"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn another_tenants_holdings_and_resources_are_not_this_sellers_files(pool: PgPool) {
    provision(&pool).await;
    resource(&pool, ORG_A, (0x11, "Fractions"), (0xF1, "pack.pdf"), false)
        .await
        .expect("the resource fixture seeds");
    resource(
        &pool,
        ORG_B,
        (0x21, "Their pack"),
        (0xF2, "theirs.pdf"),
        false,
    )
    .await
    .expect("the resource fixture seeds");
    machine(&pool, &TOKEN_A, LAPTOP, "founder-pc", &[0xF1]).await;
    machine(&pool, &TOKEN_B, PHONE, "their-pc", &[0xF2]).await;

    let mine = library(&pool, &TOKEN_A, "", t0).await;
    assert_eq!(mine.total, 1, "one digest, which is this seller's own");
    assert_eq!(mine.files[0].hash, digest(0xF1));

    let theirs = library(&pool, &TOKEN_B, "", t0).await;
    assert_eq!(theirs.total, 1);
    assert_eq!(theirs.files[0].hash, digest(0xF2));
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_digest_no_machine_reports_is_missing_and_a_kept_file_no_resource_uses_is_unlinked(
    pool: PgPool,
) {
    provision(&pool).await;
    // A resource whose payload no machine of the seller's holds any more:
    // the state the old whole-library list could not show at all, because it
    // only ever listed what a device reported.
    resource(&pool, ORG_A, (0x11, "Lost pack"), (0xF1, "lost.pdf"), false)
        .await
        .expect("the resource fixture seeds");
    // And a file the machine keeps that no resource uses.
    machine(&pool, &TOKEN_A, LAPTOP, "founder-pc", &[0xF2]).await;

    let missing = library(&pool, &TOKEN_A, "?availability=missing", t0).await;
    assert_eq!(missing.total, 1);
    assert_eq!(missing.files[0].hash, digest(0xF1));
    assert!(
        missing.files[0].holders.is_empty(),
        "no machine holds it, and the answer must not invent one"
    );
    assert_eq!(missing.files[0].file_name.as_deref(), Some("lost.pdf"));

    let unlinked = library(&pool, &TOKEN_A, "?linked=unlinked", t0).await;
    assert_eq!(unlinked.total, 1);
    assert_eq!(unlinked.files[0].hash, digest(0xF2));
    assert!(unlinked.files[0].resources.is_empty());
    assert_eq!(
        unlinked.files[0].holders.len(),
        1,
        "the machine that keeps it is still named"
    );

    let linked = library(&pool, &TOKEN_A, "?linked=linked", t0).await;
    assert_eq!(linked.total, 1);
    assert_eq!(linked.files[0].hash, digest(0xF1));
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn online_and_offline_are_told_apart_by_the_last_check_in(pool: PgPool) {
    provision(&pool).await;
    resource(&pool, ORG_A, (0x11, "Fractions"), (0xF1, "pack.pdf"), false)
        .await
        .expect("the resource fixture seeds");
    machine(&pool, &TOKEN_A, LAPTOP, "founder-pc", &[0xF1]).await;

    let fresh = library(&pool, &TOKEN_A, "?availability=online", t0).await;
    assert_eq!(fresh.total, 1, "the machine checked in a moment ago");
    assert!(fresh.files[0].holders[0].online);

    let stale = library(&pool, &TOKEN_A, "?availability=online", much_later).await;
    assert_eq!(
        stale.total, 0,
        "twenty minutes on, the machine is not there"
    );
    let offline = library(&pool, &TOKEN_A, "?availability=offline", much_later).await;
    assert_eq!(offline.total, 1);
    assert!(!offline.files[0].holders[0].online);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_filtered_page_is_bounded_stable_and_counted_against_the_filter(pool: PgPool) {
    provision(&pool).await;
    // Four files a search matches and one it does not, so the total is the
    // filter's rather than the library's.
    for (id, hash, title, file_name) in [
        (0x11u8, 0xF1u8, "Pack one", "worksheet-a.pdf"),
        (0x12, 0xF2, "Pack two", "worksheet-b.pdf"),
        (0x13, 0xF3, "Pack three", "worksheet-c.pdf"),
        (0x14, 0xF4, "Pack four", "worksheet-d.pdf"),
        (0x15, 0xF5, "Pack five", "cover.png"),
    ] {
        resource(&pool, ORG_A, (id, title), (hash, file_name), false)
            .await
            .expect("the resource fixture seeds");
    }
    machine(
        &pool,
        &TOKEN_A,
        LAPTOP,
        "founder-pc",
        &[0xF1, 0xF2, 0xF3, 0xF4, 0xF5],
    )
    .await;

    let first = library(&pool, &TOKEN_A, "?q=worksheet&limit=2&offset=0", t0).await;
    assert_eq!(first.total, 4, "the figure counts the filter, not the page");
    assert_eq!(first.limit, 2);
    assert_eq!(first.offset, 0);
    assert_eq!(first.files.len(), 2);

    let second = library(&pool, &TOKEN_A, "?q=worksheet&limit=2&offset=2", t0).await;
    assert_eq!(second.total, 4);
    assert_eq!(second.files.len(), 2);

    let mut seen: Vec<&str> = first
        .files
        .iter()
        .chain(second.files.iter())
        .map(|file| file.hash.as_str())
        .collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(
        seen.len(),
        4,
        "two pages of two cover the four matches without repeating one"
    );

    // A page past the end is empty and still says how many there are, so the
    // pager can offer the way back rather than reading it as none.
    let past = library(&pool, &TOKEN_A, "?q=worksheet&limit=2&offset=100", t0).await;
    assert!(past.files.is_empty());
    assert_eq!(past.total, 4);
    assert_eq!(past.offset, 100);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_search_phrase_belongs_to_one_resource_not_concatenated_titles(pool: PgPool) {
    provision(&pool).await;
    resource(&pool, ORG_A, (0x11, "Left"), (0xF1, "sheet.pdf"), false)
        .await
        .expect("the resource fixture seeds");
    resource(&pool, ORG_A, (0x12, "Right"), (0xF1, "sheet.pdf"), false)
        .await
        .expect("the resource fixture seeds");
    resource(
        &pool,
        ORG_A,
        (0x13, "Left Right workbook"),
        (0xF2, "workbook.pdf"),
        false,
    )
    .await
    .expect("the resource fixture seeds");

    let view = library(&pool, &TOKEN_A, "?q=Left%20Right", t0).await;
    assert_eq!(view.total, 1);
    assert_eq!(view.files.len(), 1);
    assert_eq!(view.files[0].hash, digest(0xF2));
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_machines_files_are_asked_for_by_its_own_id(pool: PgPool) {
    provision(&pool).await;
    resource(&pool, ORG_A, (0x11, "Fractions"), (0xF1, "pack.pdf"), false)
        .await
        .expect("the resource fixture seeds");
    resource(&pool, ORG_A, (0x12, "Decimals"), (0xF2, "other.pdf"), false)
        .await
        .expect("the resource fixture seeds");
    machine(&pool, &TOKEN_A, LAPTOP, "founder-pc", &[0xF1]).await;
    machine(&pool, &TOKEN_A, PHONE, "Pixel", &[0xF2]).await;

    let laptop = library(&pool, &TOKEN_A, &format!("?device={LAPTOP}"), t0).await;
    assert_eq!(laptop.total, 1);
    assert_eq!(laptop.files[0].hash, digest(0xF1));

    let both = library(&pool, &TOKEN_A, "", t0).await;
    assert_eq!(both.total, 2, "unfiltered, the seller sees both machines'");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_page_the_filters_cannot_name_is_refused_rather_than_answered_emptily(pool: PgPool) {
    provision(&pool).await;
    for query in [
        "?availability=somewhere",
        "?linked=perhaps",
        "?limit=0",
        "?limit=101",
    ] {
        let answer = call(
            pool.clone(),
            Call {
                method: Method::GET,
                path: &format!("/v1/library{query}"),
                token: &TOKEN_A,
                body: None,
                wall: t0,
            },
        )
        .await;
        assert_eq!(
            answer.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{query} names no page, and an empty list would read as no files"
        );
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_copy_is_asked_for_only_where_another_machine_holds_the_file(pool: PgPool) {
    provision(&pool).await;
    resource(&pool, ORG_A, (0x11, "Fractions"), (0xF1, "pack.pdf"), false)
        .await
        .expect("the resource fixture seeds");
    machine(&pool, &TOKEN_A, LAPTOP, "founder-pc", &[0xF1]).await;
    machine(&pool, &TOKEN_A, PHONE, "Pixel", &[]).await;

    let refused = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{LAPTOP}/library/want"),
            token: &TOKEN_A,
            body: Some(serde_json::json!({ "hash": digest(0xF1) })),
            wall: t0,
        },
    )
    .await;
    assert_eq!(
        refused.status,
        StatusCode::NOT_FOUND,
        "only this machine holds it, so nothing could ever satisfy the ask"
    );

    let asked = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{PHONE}/library/want"),
            token: &TOKEN_A,
            body: Some(serde_json::json!({ "hash": digest(0xF1) })),
            wall: t0,
        },
    )
    .await;
    assert_eq!(asked.status, StatusCode::NO_CONTENT);

    let view = library(&pool, &TOKEN_A, "", t0).await;
    assert_eq!(
        find(&view, &digest(0xF1))
            .expect("the requested file is listed")
            .wanted_by,
        vec![PHONE.to_owned()],
        "the machine that asked is named, and the page it rides is the same one"
    );
}
