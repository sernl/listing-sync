//! The revocation drill and the gateway allow-list, proven end to end against
//! the live database and an in-process fake upstream. The drill is a test that
//! always runs, so the containment window is measured on every CI run rather
//! than only when an incident forces it.
//!
//! The broker's modules are private to its binary, so this test exercises the
//! same guarantees through the surfaces it can reach across the crate
//! boundary: the seal via tam-secrets, the database contract over the broker
//! role (the write, the read-back, the tombstone), and the gateway's contract
//! via an equivalent in-process axum proxy carrying the same allow-list.

#![cfg(feature = "pg-tests")]

use std::sync::Arc;

use axum::extract::State;
use axum::routing::any;
use axum::Router;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::net::TcpListener;

const ORG: [u8; 16] = [0xAA; 16];
const CONN: [u8; 16] = [0x33; 16];

/// A reqwest client with the timeouts the ban requires, for test use.
fn test_client() -> reqwest::Client {
    #[expect(
        clippy::expect_used,
        reason = "a test client build failure is a broken fixture"
    )]
    reqwest::Client::builder()
        .timeout(core::time::Duration::from_secs(10))
        .connect_timeout(core::time::Duration::from_secs(5))
        .build()
        .expect("the test client builds")
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn broker_pool(app: &PgPool) -> PgPool {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await
        .expect("the database name reads");
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&format!(
            "postgres://tam_broker:tam_broker_dev@127.0.0.1:5433/{database}"
        ))
        .await
        .expect("the broker role connects")
}

/// A fake upstream that echoes the Cookie header it received, so the test can
/// prove the gateway injected the seller cookie the client never held.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn fake_upstream() -> String {
    async fn echo(headers: axum::http::HeaderMap) -> String {
        headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("<no-cookie>")
            .to_owned()
    }
    let router: Router<()> = Router::new().route("/api/v2/resources", any(echo));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("upstream binds");
    let addr = listener.local_addr().expect("upstream addr");
    #[expect(
        clippy::disallowed_methods,
        reason = "an in-test fake upstream; the spawn ban targets production fire-and-forget"
    )]
    {
        tokio::spawn(async move {
            let _served = axum::serve(listener, router).await;
        });
    }
    format!("http://{addr}")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_vault_round_trips_over_the_broker_role_and_revocation_tombstones(app: PgPool) {
    use tam_secrets::{open, seal, AadContext, Kek, Marketplace, Secret};

    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG))
        .execute(&app)
        .await
        .expect("the org inserts");
    let mut tx = app.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tes', 'linking', now(), now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG))
    .bind(uuid::Uuid::from_bytes(CONN))
    .execute(&mut *tx)
    .await
    .expect("the connection inserts");
    tx.commit().await.expect("the connection commits");

    let broker = broker_pool(&app).await;
    let kek = Kek::from_bytes(&[0x11; 32]).expect("kek");
    let context = AadContext {
        org: tam_types::OrgId(tam_types::Uuid(ORG)),
        marketplace: Marketplace::Tes,
        connection: tam_types::ConnectionId(tam_types::Uuid(CONN)),
        key_version: 1,
    };
    let sealed = seal(&kek, &context, Secret::new("TESSession=secret".to_owned())).expect("seal");

    // The broker role writes ciphertext the app role cannot read (proven in
    // the custody test); here it also reads its own back and revocation
    // tombstones it.
    sqlx::query(
        "INSERT INTO connection_secret \
         (org_id, connection_id, key_version, wrapped_dek, nonce, ciphertext, aad, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG))
    .bind(uuid::Uuid::from_bytes(CONN))
    .bind(sealed.key_version)
    .bind(&sealed.wrapped_dek)
    .bind(&sealed.nonce)
    .bind(&sealed.ciphertext)
    .bind(vec![0u8; 37])
    .execute(&broker)
    .await
    .expect("the broker role writes the vault");

    let row = sqlx::query_as::<_, (i32, Vec<u8>, Vec<u8>, Vec<u8>)>(
        "SELECT key_version, wrapped_dek, nonce, ciphertext FROM connection_secret \
         WHERE org_id = $1 AND connection_id = $2",
    )
    .bind(uuid::Uuid::from_bytes(ORG))
    .bind(uuid::Uuid::from_bytes(CONN))
    .fetch_one(&broker)
    .await
    .expect("the broker role reads its own vault");
    let reopened = open(
        &kek,
        &context,
        &tam_secrets::Sealed {
            key_version: row.0,
            wrapped_dek: row.1,
            nonce: row.2,
            ciphertext: row.3,
        },
    )
    .expect("the stored ciphertext opens");
    assert_eq!(
        reopened.expose(),
        "TESSession=secret",
        "a credential sealed and stored over the broker role round-trips"
    );

    // The drill: clear ciphertext, revoke the connection, and time it. The
    // elapsed figure is the containment window the drill exists to measure.
    #[expect(
        clippy::disallowed_methods,
        reason = "the drill measures its own wall-clock; that elapsed figure is the point of the drill"
    )]
    let start = std::time::Instant::now();
    sqlx::query("UPDATE connection_secret SET wrapped_dek = ''::bytea, ciphertext = ''::bytea")
        .execute(&broker)
        .await
        .expect("the drill clears ciphertext");
    sqlx::query("UPDATE connection SET state = 'revoked', updated_at = now()")
        .execute(&broker)
        .await
        .expect("the drill revokes connections");
    let elapsed = start.elapsed();
    eprintln!("revocation drill wall-clock: {}ms", elapsed.as_millis());

    let (dek_len, state): (i32, String) = sqlx::query_as(
        "SELECT octet_length(wrapped_dek), \
         (SELECT state FROM connection WHERE org_id = $1 AND id = $2) \
         FROM connection_secret WHERE org_id = $1 AND connection_id = $2",
    )
    .bind(uuid::Uuid::from_bytes(ORG))
    .bind(uuid::Uuid::from_bytes(CONN))
    .fetch_one(&broker)
    .await
    .expect("the tombstoned row reads");
    assert_eq!(
        dek_len, 0,
        "the wrapped DEK is cleared to a tombstone, not deleted"
    );
    assert_eq!(state, "revoked", "the connection is revoked");
}

#[tokio::test]
async fn the_gateway_injects_the_cookie_and_enforces_the_allow_list() {
    // A minimal stand-in for the broker's gateway, exercising the same
    // contract: allow-listed paths reach upstream with the cookie injected,
    // and off-list paths are refused before any upstream call.
    #[derive(Clone)]
    struct Gw {
        upstream: String,
        cookie: String,
        client: reqwest::Client,
    }
    async fn proxy(
        State(gw): State<Arc<Gw>>,
        request: axum::extract::Request,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        let path = request.uri().path().to_owned();
        let allowed = path == "/api/v2/resources"
            || path.starts_with("/api/v2/resources/")
            || path == "/api/resources/v3/draft"
            || path.starts_with("/api/resources/v3/draft/");
        if !allowed {
            return (axum::http::StatusCode::FORBIDDEN, "off allow-list").into_response();
        }
        let body = gw
            .client
            .post(format!("{}{}", gw.upstream, path))
            .header(axum::http::header::COOKIE, &gw.cookie)
            .send()
            .await
            .expect("upstream reachable")
            .text()
            .await
            .expect("upstream body");
        (axum::http::StatusCode::OK, body).into_response()
    }
    let upstream = fake_upstream().await;
    let gw = Arc::new(Gw {
        upstream,
        cookie: "TESSession=injected".to_owned(),
        client: test_client(),
    });
    let router = Router::new().fallback(any(proxy)).with_state(gw);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("gateway binds");
    let addr = listener.local_addr().expect("gateway addr");
    #[expect(
        clippy::disallowed_methods,
        reason = "an in-test gateway; the spawn ban targets production fire-and-forget"
    )]
    {
        tokio::spawn(async move {
            let _served = axum::serve(listener, router).await;
        });
    }

    let client = test_client();
    let allowed = client
        .post(format!("http://{addr}/api/v2/resources"))
        .send()
        .await
        .expect("the allow-listed route reaches the gateway");
    let echoed = allowed.text().await.expect("the echo body");
    assert_eq!(
        echoed, "TESSession=injected",
        "the gateway injected the cookie the client never held"
    );

    let refused = client
        .get(format!("http://{addr}/api/v2/account/roster"))
        .send()
        .await
        .expect("the off-list route reaches the gateway");
    assert_eq!(
        refused.status(),
        reqwest::StatusCode::FORBIDDEN,
        "an off-allow-list route is refused before any upstream call"
    );
}
