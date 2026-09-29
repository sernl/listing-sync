//! The operators' campaigns against a real database: a campaign queues one
//! outbox row per recipient, the drainer claims each once, deleting frees the
//! body and the rows and keeps the log line, the unsubscribe token opts a
//! seller out, and erasing a seller takes their rows with them.

#![cfg(feature = "pg-tests")]

use serde_json::json;
use sqlx::PgPool;
use tam_storage::{
    CampaignCounts, ErasureRepo, MailCampaignRepo, MailOutcome, NewCampaign, NewRecipient,
    StorageError,
};
use tam_types::{Timestamp, UserId, Uuid};

const OPERATOR: UserId = UserId(Uuid([0x0A; 16]));
const CAMPAIGN: Uuid = Uuid([0xC1; 16]);
const T0: Timestamp = Timestamp(1_790_000_000_000);
const T1: Timestamp = Timestamp(1_790_000_060_000);

fn user(n: u8) -> UserId {
    UserId(Uuid([n; 16]))
}

fn subject(n: u8) -> Uuid {
    Uuid([n ^ 0xFF; 16])
}

/// One organisation and user per seller, the operator included.
async fn provision(pool: &PgPool, sellers: &[u8]) -> Result<(), sqlx::Error> {
    for &n in sellers.iter().chain([0x0A].iter()) {
        let org = uuid::Uuid::from_bytes([n.wrapping_add(0x40); 16]);
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(org)
            .bind(format!("org {n}"))
            .execute(pool)
            .await?;
        sqlx::query(
            "INSERT INTO app_user (id, org_id, email, created_at, auth_subject) \
             VALUES ($1, $2, $3, now(), $4)",
        )
        .bind(uuid::Uuid::from_bytes([n; 16]))
        .bind(org)
        .bind(format!("u{n}@subject.invalid"))
        .bind(uuid::Uuid::from_bytes(subject(n).0))
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn queue(repo: &MailCampaignRepo, sellers: &[u8]) -> Result<(), StorageError> {
    let audience = json!({"segment": "all", "exclude_operators": true, "verified_only": true});
    let recipients: Vec<NewRecipient> = sellers
        .iter()
        .map(|&n| NewRecipient {
            user: user(n),
            auth_subject: subject(n),
            org_name: format!("org {n}"),
            plan: "Look".to_owned(),
        })
        .collect();
    repo.create(
        &NewCampaign {
            id: CAMPAIGN,
            subject: "News",
            body_html: "<p>Hi @first_name</p>",
            link_url: None,
            link_label: None,
            audience: &audience,
            test: false,
            created_by: OPERATOR,
            created_by_label: "ops@example.test",
            at: T0,
        },
        &recipients,
    )
    .await
}

#[sqlx::test(migrations = "./migrations")]
async fn a_campaign_queues_one_outbox_row_per_recipient(pool: PgPool) -> Result<(), StorageError> {
    provision(&pool, &[1, 2, 3]).await?;
    let repo = MailCampaignRepo::new(pool.clone());
    queue(&repo, &[1, 2, 3]).await?;

    let rows = repo.recipients(CAMPAIGN).await?;
    assert_eq!(rows.len(), 3, "one row per recipient");
    assert!(rows
        .iter()
        .all(|row| row.status == "queued" && row.attempts == 0));
    let listed = repo.list().await?;
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].counts,
        CampaignCounts {
            total: 3,
            queued: 3,
            sent: 0,
            failed: 0,
            skipped: 0
        }
    );

    // Each row is claimed once while its lease holds.
    let lease = Timestamp(T0.0 + 120_000);
    let mut claimed = Vec::new();
    while let Some(mail) = repo.claim(T0, lease).await? {
        assert_eq!(mail.attempts, 1);
        assert_eq!(mail.body_html, "<p>Hi @first_name</p>");
        claimed.push(mail);
    }
    assert_eq!(claimed.len(), 3, "nothing is claimed twice under a lease");

    repo.settle(
        CAMPAIGN,
        claimed[0].user,
        &MailOutcome::Sent {
            provider_id: "re_1".to_owned(),
        },
        T1,
    )
    .await?;
    repo.settle(
        CAMPAIGN,
        claimed[1].user,
        &MailOutcome::Failed {
            error: "refused".to_owned(),
        },
        T1,
    )
    .await?;
    repo.settle(
        CAMPAIGN,
        claimed[2].user,
        &MailOutcome::Retry {
            error: "429".to_owned(),
            at: Timestamp(T1.0 + 60_000),
        },
        T1,
    )
    .await?;
    assert!(
        repo.claim(T1, lease).await?.is_none(),
        "the retry is not due yet"
    );
    let again = repo
        .claim(Timestamp(T1.0 + 60_000), Timestamp(T1.0 + 180_000))
        .await?
        .expect("due again after its wait");
    assert_eq!(again.attempts, 2);

    let counts = repo.get(CAMPAIGN).await?.expect("the campaign").counts;
    assert_eq!((counts.sent, counts.failed, counts.queued), (1, 1, 1));
    let sent = repo
        .recipients(CAMPAIGN)
        .await?
        .into_iter()
        .find(|row| row.status == "sent")
        .expect("the sent row");
    assert_eq!(sent.provider_id.as_deref(), Some("re_1"));

    assert_eq!(repo.retry_failed(CAMPAIGN, T1).await?, 1);
    let counts = repo.get(CAMPAIGN).await?.expect("the campaign").counts;
    assert_eq!((counts.failed, counts.queued), (0, 2));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn deleting_frees_the_body_and_rows_and_keeps_the_log_line(
    pool: PgPool,
) -> Result<(), StorageError> {
    provision(&pool, &[1, 2]).await?;
    let repo = MailCampaignRepo::new(pool.clone());
    queue(&repo, &[1, 2]).await?;
    let first = repo.claim(T0, T1).await?.expect("a row");
    repo.settle(
        CAMPAIGN,
        first.user,
        &MailOutcome::Sent {
            provider_id: "re_1".to_owned(),
        },
        T1,
    )
    .await?;

    assert!(repo.delete(CAMPAIGN, OPERATOR, T1).await?);
    assert!(
        !repo.delete(CAMPAIGN, OPERATOR, T1).await?,
        "a second delete finds nothing live"
    );

    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mail_campaign_recipient WHERE campaign_id = $1")
            .bind(uuid::Uuid::from_bytes(CAMPAIGN.0))
            .fetch_one(&pool)
            .await?;
    assert_eq!(rows, 0, "every recipient row is gone");
    let kept = repo.get(CAMPAIGN).await?.expect("the log line stays");
    assert_eq!(kept.body_html, None, "the body is gone");
    assert_eq!(kept.subject, "News");
    assert_eq!(kept.deleted_at, Some(T1));
    assert_eq!(
        kept.counts,
        CampaignCounts {
            total: 2,
            queued: 1,
            sent: 1,
            failed: 0,
            skipped: 0
        },
        "the counts are frozen as they stood"
    );
    assert!(
        repo.claim(T1, T1).await?.is_none(),
        "nothing deleted is sent"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_unsubscribe_token_opts_its_seller_out(pool: PgPool) -> Result<(), StorageError> {
    provision(&pool, &[1, 2]).await?;
    let repo = MailCampaignRepo::new(pool.clone());
    let token: uuid::Uuid =
        sqlx::query_scalar("SELECT marketing_token FROM app_user WHERE id = $1")
            .bind(uuid::Uuid::from_bytes(user(1).0 .0))
            .fetch_one(&pool)
            .await?;
    assert!(
        !repo.opt_out_by_token(Uuid([0x77; 16])).await?,
        "an unknown token changes nothing"
    );
    assert!(repo.opt_out_by_token(Uuid(*token.as_bytes())).await?);

    let members = repo.audience_members().await?;
    let opted: Vec<(UserId, bool)> = members.iter().map(|m| (m.user, m.opted_out)).collect();
    assert!(opted.contains(&(user(1), true)));
    assert!(opted.contains(&(user(2), false)), "only the token's holder");

    // The drainer sees it on a row queued before the click.
    queue(&repo, &[1]).await?;
    let claimed = repo.claim(T0, T1).await?.expect("the row");
    assert!(claimed.opted_out);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn erasing_a_seller_takes_their_campaign_rows(pool: PgPool) -> Result<(), StorageError> {
    provision(&pool, &[1, 2]).await?;
    let repo = MailCampaignRepo::new(pool.clone());
    queue(&repo, &[1, 2]).await?;
    ErasureRepo::new(pool.clone())
        .erase_account(subject(1))
        .await?
        .expect("a one-person tenant nobody pays for is erased");
    let left: Vec<UserId> = repo
        .recipients(CAMPAIGN)
        .await?
        .into_iter()
        .map(|row| row.user)
        .collect();
    assert_eq!(left, vec![user(2)]);
    Ok(())
}
