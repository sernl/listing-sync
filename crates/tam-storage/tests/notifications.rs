//! The seller's completion inbox, proven against the live database: one row
//! per run rather than per job, a migration's two legs summed into one, the
//! mail withheld from a run that changed nothing, the recipient read skipping
//! whoever opted out, and one tenant's completions invisible to another.

#![cfg(feature = "pg-tests")]

mod common;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_domain::{
    Binding, FieldPolicies, FieldPolicy, ItemOperation, JobItemId, Mapping, PublishMode,
};
use tam_marketplace::{IdempotencyKey, RemoteLifecycle};
use tam_storage::{
    settle_if_complete, JobRepo, MappingRepo, NewJob, NewJobItem, NotificationRepo, ProductRepo,
};
use tam_types::{
    Actor, ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, InventoryId, JobId,
    ListingCopy, MappingId, NotificationCounts, NotificationKind, OrgId, PayloadSet, PriceIntent,
    PriceRule, ProductFile, ProductId, ScanOutcome, Stamp, SystemComponent, Timestamp, Title,
    UserId, Uuid,
};

const T0: Timestamp = Timestamp(1_756_000_000_000);
const REQUEST: Uuid = Uuid([0x71; 16]);
const CREATE_JOB: JobId = JobId(Uuid([0x81; 16]));
const REMOVE_JOB: JobId = JobId(Uuid([0x82; 16]));

fn db_uuid(id: Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn engine_pool(app: &PgPool) -> PgPool {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await
        .expect("the database name is readable");
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&format!(
            "postgres://tam_engine:tam_engine_dev@127.0.0.1:5433/{database}"
        ))
        .await
        .expect("the engine role connects to the test database")
}

struct Tenant {
    org: OrgId,
    mapping: MappingId,
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_tenant(app: &PgPool, seed: u8) -> Tenant {
    let org = OrgId(Uuid([seed; 16]));
    let product = ProductId(Uuid([seed.wrapping_add(1); 16]));
    let mapping = MappingId(Uuid([seed.wrapping_add(2); 16]));
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
        .bind(db_uuid(org.0))
        .bind(format!("org-{seed}"))
        .execute(app)
        .await
        .expect("organisation row inserts");
    ProductRepo::new(app.clone())
        .insert(
            org,
            &tam_domain::CanonicalProduct {
                id: product,
                org,
                title: Title("Fixture".to_owned()),
                body: ListingCopy {
                    body: "Fixture".to_owned(),
                    format: CopyFormat::Markdown,
                },
                payload: Some(PayloadSet::new(
                    ProductFile {
                        id: FileId(Uuid([seed.wrapping_add(3); 16])),
                        role: FileRole::Payload,
                        kind: FileKind::Pdf,
                        bytes: FileBytes::Held {
                            hash: ContentHash([seed; 32]),
                            byte_len: 4,
                            scan: ScanOutcome::Pending,
                        },
                    },
                    vec![],
                )),
                cover: None,
                previews: vec![],
                subjects: Vec::new(),
                grades: tam_domain::GradeDeclaration {
                    source: tam_domain::DeclarationSource::Seller,
                    raw: vec![],
                    derived: None,
                },
                price: PriceIntent::Free,
                rights: tam_domain::RightsDeclaration::Unstated,
                native_residue: vec![],
            },
            T0,
        )
        .await
        .expect("the fixture product inserts");
    MappingRepo::new(app.clone())
        .insert(
            org,
            &Mapping {
                id: mapping,
                org,
                product,
                inventory: InventoryId::Tes,
                binding: Binding::Unbound,
                policies: FieldPolicies {
                    title: FieldPolicy::Managed,
                    description: FieldPolicy::Managed,
                    price: FieldPolicy::Managed,
                    taxonomy: FieldPolicy::Managed,
                    grades: FieldPolicy::Managed,
                    files: FieldPolicy::Managed,
                },
                price_rule: PriceRule::Explicit(PriceIntent::Free),
                publish: PublishMode::DryRun,
                lifecycle: RemoteLifecycle::Absent,
            },
            0,
            T0,
        )
        .await
        .expect("the fixture mapping inserts");
    Tenant { org, mapping }
}

/// A request naming the jobs it minted, which is what makes a settle a run's
/// completion rather than a job's.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_request(app: &PgPool, org: OrgId, disposition: &str, remove: Option<JobId>) {
    // Under the tenant's own role and its own pin: the drain writes requests as
    // tam_app, and migration 0063 gives the engine SELECT on this table and
    // nothing else, so a fixture writing as the engine would be proving a
    // privilege the settle path does not have.
    let mut tx = app.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(org.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO sync_request \
         (org_id, id, source, target, disposition, intent, state, \
          create_job_id, remove_job_id, requested_at, settled_at) \
         VALUES ($1, $2, 'tes', 'tpt', $3, 'draft', 'enqueued', $4, $5, now(), now())",
    )
    .bind(db_uuid(org.0))
    .bind(db_uuid(REQUEST))
    .bind(disposition)
    .bind(db_uuid(CREATE_JOB.0))
    .bind(remove.map(|job| db_uuid(job.0)))
    .execute(&mut *tx)
    .await
    .expect("the fixture request inserts");
    // The job names its request too, as production does when it mints one:
    // `run_of` resolves the run through `job.sync_request_id` rather than
    // through the request's own columns, so a fixture that set only one
    // direction would prove nothing about the path that runs.
    sqlx::query("UPDATE job SET sync_request_id = $2 WHERE org_id = $1 AND id = ANY($3)")
        .bind(db_uuid(org.0))
        .bind(db_uuid(REQUEST))
        .bind(
            [
                Some(db_uuid(CREATE_JOB.0)),
                remove.map(|job| db_uuid(job.0)),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<uuid::Uuid>>(),
        )
        .execute(&mut *tx)
        .await
        .expect("the fixture jobs name their request");
    tx.commit().await.expect("the fixture request commits");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn enqueue(engine: &PgPool, tenant: &Tenant, job: JobId, items: &[u8]) {
    let items: Vec<NewJobItem> = items
        .iter()
        .map(|seed| NewJobItem {
            item: JobItemId(Uuid([*seed; 16])),
            mapping: tenant.mapping,
            idempotency_key: IdempotencyKey(Uuid([seed.wrapping_add(0x40); 16])),
            operation: ItemOperation::Create,
            requires_bound_on: None,
        })
        .collect();
    JobRepo::new(engine.clone())
        .enqueue(
            tenant.org,
            &NewJob {
                job,
                inventory: InventoryId::Tes,
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
            &items,
        )
        .await
        .expect("the job enqueues");
}

/// Settles a job's items and runs the completion check, exactly as the ledger
/// write does, in one transaction.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn settle_job(engine: &PgPool, org: OrgId, job: JobId, outcome: &str) -> bool {
    let mut tx = engine.begin().await.expect("the settler opens");
    sqlx::query(
        "UPDATE job_item SET state = 'settled', outcome = $3, settled_at = now() \
          WHERE org_id = $1 AND job_id = $2",
    )
    .bind(db_uuid(org.0))
    .bind(db_uuid(job.0))
    .bind(outcome)
    .execute(&mut *tx)
    .await
    .expect("the items settle");
    let emitted = settle_if_complete(&mut tx, org, job, T0)
        .await
        .expect("the completion check runs");
    tx.commit().await.expect("the settle commits");
    emitted
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn topics(engine: &PgPool, org: OrgId) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT topic FROM outbox_message WHERE org_id = $1")
        .bind(db_uuid(org.0))
        .fetch_all(engine)
        .await
        .expect("the outbox is readable")
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_user(app: &PgPool, org: OrgId, seed: u8, wants_mail: bool) -> UserId {
    let user = UserId(Uuid([seed; 16]));
    sqlx::query(
        "INSERT INTO app_user (id, org_id, email, created_at, auth_subject, notify_email) \
         VALUES ($1, $2, $3, now(), $4, $5)",
    )
    .bind(db_uuid(user.0))
    .bind(db_uuid(org.0))
    .bind(format!("{seed}@subject.invalid"))
    .bind(db_uuid(Uuid([seed.wrapping_add(0x10); 16])))
    .bind(wants_mail)
    .execute(app)
    .await
    .expect("the fixture user inserts");
    user
}

/// A sync is one job under one request, so it is one inbox row keyed on the
/// request and one message.
#[sqlx::test(migrations = "./migrations")]
async fn a_settled_sync_writes_one_notification_and_one_message(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC1).await;
    enqueue(&engine, &tenant, CREATE_JOB, &[0x91, 0x92]).await;
    seed_request(&app, tenant.org, "sync", None).await;

    assert!(
        settle_job(&engine, tenant.org, CREATE_JOB, "succeeded").await,
        "settling every item finishes the job"
    );

    let rows = NotificationRepo::new(app.clone())
        .list(tenant.org, None, 10)
        .await
        .expect("the inbox is readable");
    assert_eq!(rows.len(), 1, "one run, one row");
    assert_eq!(rows[0].kind, NotificationKind::Sync);
    assert_eq!(
        rows[0].subject, REQUEST,
        "the row names the request the seller asked for, not the job that finished"
    );
    assert_eq!(rows[0].inventory, Some(InventoryId::Tpt), "the target");
    assert_eq!(
        rows[0].counts,
        NotificationCounts {
            succeeded: 2,
            ..NotificationCounts::default()
        }
    );
    assert!(rows[0].read_at.is_none(), "a new row is unread");
    assert_eq!(topics(&engine, tenant.org).await, vec!["email.job_settled"]);
}

/// A migration owns two jobs and is one run. The first leg to settle writes
/// nothing; the second writes one row whose counts are the sum of both.
#[sqlx::test(migrations = "./migrations")]
async fn a_migration_notifies_once_its_second_leg_settles(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC2).await;
    enqueue(&engine, &tenant, CREATE_JOB, &[0x91, 0x92]).await;
    enqueue(&engine, &tenant, REMOVE_JOB, &[0x93]).await;
    seed_request(&app, tenant.org, "migrate", Some(REMOVE_JOB)).await;

    settle_job(&engine, tenant.org, CREATE_JOB, "succeeded").await;
    let notifications = NotificationRepo::new(app.clone());
    assert!(
        notifications
            .list(tenant.org, None, 10)
            .await
            .expect("the inbox is readable")
            .is_empty(),
        "one leg of a migration is not a finished migration"
    );
    assert!(
        topics(&engine, tenant.org).await.is_empty(),
        "and nothing is mailed for half a run"
    );

    settle_job(&engine, tenant.org, REMOVE_JOB, "failed").await;
    let rows = notifications
        .list(tenant.org, None, 10)
        .await
        .expect("the inbox is readable");
    assert_eq!(rows.len(), 1, "two jobs, one run, one row");
    assert_eq!(rows[0].kind, NotificationKind::Migration);
    assert_eq!(rows[0].subject, REQUEST);
    assert_eq!(
        rows[0].counts,
        NotificationCounts {
            succeeded: 2,
            failed: 1,
            ..NotificationCounts::default()
        },
        "the counts sum over both legs"
    );
    assert_eq!(topics(&engine, tenant.org).await, vec!["email.job_settled"]);
}

/// The reverse order produces the identical row, because the rule is about the
/// sibling being settled and not about which leg finished first.
#[sqlx::test(migrations = "./migrations")]
async fn the_legs_may_settle_in_either_order(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC3).await;
    enqueue(&engine, &tenant, CREATE_JOB, &[0x91, 0x92]).await;
    enqueue(&engine, &tenant, REMOVE_JOB, &[0x93]).await;
    seed_request(&app, tenant.org, "migrate", Some(REMOVE_JOB)).await;

    settle_job(&engine, tenant.org, REMOVE_JOB, "failed").await;
    assert!(
        NotificationRepo::new(app.clone())
            .list(tenant.org, None, 10)
            .await
            .expect("the inbox is readable")
            .is_empty(),
        "the remove leg alone is not a finished migration either"
    );

    settle_job(&engine, tenant.org, CREATE_JOB, "succeeded").await;
    let rows = NotificationRepo::new(app.clone())
        .list(tenant.org, None, 10)
        .await
        .expect("the inbox is readable");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].counts,
        NotificationCounts {
            succeeded: 2,
            failed: 1,
            ..NotificationCounts::default()
        },
        "whichever leg finished last, the run's counts are the same"
    );
}

/// A job no request names writes the message it always wrote and no inbox row,
/// because there is no run page for a notification's button to open.
#[sqlx::test(migrations = "./migrations")]
async fn a_job_outside_a_request_keeps_the_message_it_always_had(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC4).await;
    enqueue(&engine, &tenant, CREATE_JOB, &[0x91]).await;

    assert!(settle_job(&engine, tenant.org, CREATE_JOB, "succeeded").await);
    assert_eq!(
        topics(&engine, tenant.org).await,
        vec!["email.job_settled"],
        "the seller is told, which nothing did before"
    );
    assert!(
        NotificationRepo::new(app.clone())
            .list(tenant.org, None, 10)
            .await
            .expect("the inbox is readable")
            .is_empty(),
        "and the inbox holds nothing it could link to"
    );
}

/// A run that changed nothing is a line in the console and not an email, which
/// is the whole of the fatigue rule on an hourly schedule.
///
/// Driven through the import path because that is where an all-zero run is
/// reachable: `job_item_settled_total` requires an outcome on every settled
/// item, so a job with items always carries at least one count, while a batch
/// whose rows were all refused at parse time creates nothing and fails nothing.
#[sqlx::test(migrations = "./migrations")]
async fn a_run_that_changed_nothing_fills_the_inbox_and_sends_no_mail(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC5).await;
    NotificationRepo::new(app.clone())
        .record_import(
            tenant.org,
            Uuid([0xA1; 16]),
            NotificationCounts::default(),
            T0,
        )
        .await
        .expect("the import records");

    let rows = NotificationRepo::new(app.clone())
        .list(tenant.org, None, 10)
        .await
        .expect("the inbox is readable");
    assert_eq!(rows.len(), 1, "the console still says the run finished");
    assert!(rows[0].counts.is_empty(), "and that it changed nothing");
    assert!(
        rows[0].inventory.is_none(),
        "an import writes to no marketplace"
    );
    assert!(
        topics(&engine, tenant.org).await.is_empty(),
        "no mail for a run with nothing to report"
    );
}

/// Marking read is idempotent, and it reaches everything older rather than
/// only the row named.
#[sqlx::test(migrations = "./migrations")]
async fn marking_read_is_idempotent_and_reaches_backwards(app: PgPool) {
    let tenant = seed_tenant(&app, 0xC6).await;
    let notifications = NotificationRepo::new(app.clone());
    notifications
        .record_import(
            tenant.org,
            Uuid([0xA1; 16]),
            NotificationCounts {
                succeeded: 3,
                ..NotificationCounts::default()
            },
            Timestamp(T0.0),
        )
        .await
        .expect("the first import records");
    notifications
        .record_import(
            tenant.org,
            Uuid([0xA2; 16]),
            NotificationCounts {
                succeeded: 1,
                ..NotificationCounts::default()
            },
            Timestamp(T0.0 + 1_000),
        )
        .await
        .expect("the second import records");

    let rows = notifications
        .list(tenant.org, None, 10)
        .await
        .expect("the inbox is readable");
    assert_eq!(rows.len(), 2, "two imports, two rows");
    assert_eq!(rows[0].subject, Uuid([0xA2; 16]), "newest first");

    let newest = rows[0].id;
    assert_eq!(
        notifications
            .mark_read_through(tenant.org, newest, Timestamp(T0.0 + 2_000))
            .await
            .expect("the mark runs"),
        Some(2),
        "the newest carries everything older with it"
    );
    assert_eq!(
        notifications
            .mark_read_through(tenant.org, newest, Timestamp(T0.0 + 3_000))
            .await
            .expect("the mark runs again"),
        Some(0),
        "a second mark changes nothing"
    );
    assert_eq!(
        notifications
            .mark_read_through(tenant.org, Uuid([0xEE; 16]), Timestamp(T0.0))
            .await
            .expect("the mark runs"),
        None,
        "an id this organisation does not hold is not found rather than a fault"
    );
}

/// An import records once however many chunks finish it.
#[sqlx::test(migrations = "./migrations")]
async fn an_import_records_once(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC7).await;
    let notifications = NotificationRepo::new(app.clone());
    let counts = NotificationCounts {
        succeeded: 4,
        ..NotificationCounts::default()
    };
    assert!(notifications
        .record_import(tenant.org, Uuid([0xA1; 16]), counts, T0)
        .await
        .expect("the first pass records"));
    assert!(
        !notifications
            .record_import(tenant.org, Uuid([0xA1; 16]), counts, T0)
            .await
            .expect("the second pass runs"),
        "a chunk that arrives second writes nothing"
    );
    assert_eq!(
        notifications
            .list(tenant.org, None, 10)
            .await
            .expect("the inbox is readable")
            .len(),
        1
    );
    assert_eq!(
        topics(&engine, tenant.org).await.len(),
        1,
        "and one message"
    );
}

/// The recipient read answers who wants the mail, and nothing about anyone who
/// does not.
#[sqlx::test(migrations = "./migrations")]
async fn the_recipient_read_skips_whoever_opted_out(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC8).await;
    let wants = seed_user(&app, tenant.org, 0x31, true).await;
    let declines = seed_user(&app, tenant.org, 0x32, false).await;
    let notifications = NotificationRepo::new(engine.clone());

    let recipients = notifications
        .recipients(tenant.org)
        .await
        .expect("the recipient read runs");
    assert_eq!(recipients.len(), 1, "only the one who wants it");
    assert_eq!(recipients[0].user, wants);

    assert_eq!(
        NotificationRepo::new(app.clone())
            .set_notify_email(tenant.org, wants, false)
            .await
            .expect("the preference writes"),
        Some(false)
    );
    assert!(
        notifications
            .recipients(tenant.org)
            .await
            .expect("the recipient read runs")
            .is_empty(),
        "an organisation whose only user opted out has nobody to mail, which is \
         an empty answer rather than a failure"
    );
    assert_eq!(
        NotificationRepo::new(app.clone())
            .notify_email(tenant.org, declines)
            .await
            .expect("the preference reads"),
        Some(false)
    );
}

/// One tenant's completions are invisible to another under the app role, which
/// is what the row-level policy is for.
#[sqlx::test(migrations = "./migrations")]
async fn one_tenant_cannot_read_anothers_completions(app: PgPool) {
    let first = seed_tenant(&app, 0xC9).await;
    let second = seed_tenant(&app, 0xCA).await;
    NotificationRepo::new(app.clone())
        .record_import(
            first.org,
            Uuid([0xA1; 16]),
            NotificationCounts {
                succeeded: 2,
                ..NotificationCounts::default()
            },
            T0,
        )
        .await
        .expect("the import records");

    let seen = NotificationRepo::new(app.clone())
        .list(second.org, None, 10)
        .await
        .expect("the inbox is readable");
    assert!(
        seen.is_empty(),
        "tenant B must not see tenant A's completions"
    );
    assert_eq!(
        NotificationRepo::new(app.clone())
            .list(first.org, None, 10)
            .await
            .expect("the inbox is readable")
            .len(),
        1,
        "and tenant A still sees its own"
    );
}
