//! The help corpus in the database: what a save writes, what a publish
//! copies, and what a reader can reach of either.
//!
//! One property carries most of this file. A guide is two copies — the working
//! one an operator edits and the snapshot sellers read — and the whole point
//! of migration 0075 is that writing the first cannot change the second. Every
//! test here that saves a published guide then asks the reader's own methods
//! what they answer, because "the draft did not leak" is a statement about
//! what those methods return and not about what the writer intended.
//!
//! The second property is that every write is conditional. Two operators in
//! one guide, and a publisher who read a draft somebody has since replaced,
//! both have to fail loudly and change nothing: the alternative is one
//! operator's paragraph disappearing silently, or a seller reading a paragraph
//! nobody approved.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_storage::{
    escape_like, GuideDelete, GuideEdit, GuidePublishedHead, GuideRecord, GuideRepo,
    GuideRevisionWrite, GuideSearch, GuideStatus, GuideTaxon, GuideTaxonKind, GuideTaxonWrite,
    GuideWrite, NewGuide,
};
use tam_types::{Timestamp, UserId, Uuid};

const OPERATOR: UserId = UserId(Uuid([0x0A; 16]));
const NOW: Timestamp = Timestamp(5_000);
const LATER: Timestamp = Timestamp(9_000);

/// The record a successful conditional write answered with.
///
/// The acknowledgement is a value the write produced, not a read taken
/// afterwards, so a test that wants to know what a write did asks the write.
#[expect(
    clippy::panic,
    reason = "allow-panic-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn landed(outcome: GuideRevisionWrite) -> GuideRecord {
    match outcome {
        GuideRevisionWrite::Written(stored) => *stored,
        GuideRevisionWrite::Stale { id, revision } => panic!(
            "the write was expected to land; the guide at that slug is {} at revision {revision}",
            id.to_hyphenated()
        ),
        GuideRevisionWrite::Missing => panic!("the write was expected to land; there is no guide"),
    }
}

async fn provision_operator(pool: &PgPool) -> Result<(), sqlx::Error> {
    let org = uuid::Uuid::from_bytes([0xA0; 16]);
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
        .bind(org)
        .bind("guide test operator")
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO app_user (id, org_id, email, created_at) VALUES ($1, $2, $3, now())")
        .bind(uuid::Uuid::from_bytes(OPERATOR.0 .0))
        .bind(org)
        .bind("guide-operator@example.test")
        .execute(pool)
        .await?;
    Ok(())
}

#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "allow-expect-in-tests and allow-panic-in-tests reach #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn taxon(repo: &GuideRepo, kind: GuideTaxonKind, slug: &str, name: &str) -> GuideTaxon {
    match repo
        .create_taxon(kind, slug, name, NOW)
        .await
        .expect("the word stores")
    {
        GuideTaxonWrite::Stored(taxon) => taxon,
        GuideTaxonWrite::SlugTaken => panic!("the fixture named a slug nothing holds"),
    }
}

#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "allow-expect-in-tests and allow-panic-in-tests reach #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn draft(
    repo: &GuideRepo,
    slug: &str,
    title: &str,
    body: &str,
    filing: (Uuid, Uuid),
) -> Uuid {
    let (topic, tag) = filing;
    let written = repo
        .create(&NewGuide {
            slug,
            edit: GuideEdit {
                title,
                body,
                topic: Some(topic),
                tags: &[tag],
                updated_by: OPERATOR,
                at: NOW,
            },
        })
        .await
        .expect("the guide stores");
    match written {
        // The identifier travels back to the test, because a conditional
        // write names the guide and not only the address it lives at.
        GuideWrite::Stored(stored) => stored.id,
        GuideWrite::SlugTaken => panic!("the fixture named a slug another guide holds"),
    }
}

fn names(taxa: &[GuideTaxon]) -> Vec<&str> {
    taxa.iter().map(|taxon| taxon.name.as_str()).collect()
}

fn listed(heads: &[GuidePublishedHead]) -> Vec<&str> {
    heads.iter().map(|head| head.slug.as_str()).collect()
}

#[sqlx::test(migrations = "./migrations")]
async fn a_save_on_a_published_guide_changes_nothing_a_reader_can_see(pool: PgPool) {
    provision_operator(&pool)
        .await
        .expect("the operator provisions");
    let repo = GuideRepo::new(pool.clone());
    let selling = taxon(&repo, GuideTaxonKind::Topic, "selling", "Selling").await;
    let pricing = taxon(&repo, GuideTaxonKind::Topic, "pricing", "Pricing").await;
    let tpt = taxon(&repo, GuideTaxonKind::Tag, "tpt", "TPT").await;
    let etsy = taxon(&repo, GuideTaxonKind::Tag, "etsy", "Etsy").await;

    let guide = draft(
        &repo,
        "getting-started",
        "Getting started",
        "Connect a marketplace first.",
        (selling.id, tpt.id),
    )
    .await;
    assert_eq!(
        landed(
            repo.publish("getting-started", guide, 1, OPERATOR, NOW)
                .await
                .expect("the publish lands")
        )
        .revision,
        2,
        "a publish is a write, so it moves the revision on"
    );

    // The operator now edits every field the reader can see, and files the
    // guide somewhere else entirely.
    let saved = repo
        .save(
            "getting-started",
            &GuideEdit {
                title: "Getting started, rewritten",
                body: "An unfinished sentence about quarantined dragonfruit.",
                topic: Some(pricing.id),
                tags: &[etsy.id],
                updated_by: OPERATOR,
                at: LATER,
            },
            guide,
            2,
        )
        .await
        .expect("the save lands");
    assert_eq!(
        landed(saved).revision,
        3,
        "the save is accepted; what it must not do is change the page"
    );

    let page = repo
        .published_page("getting-started")
        .await
        .expect("the page reads")
        .expect("the guide is published");
    assert_eq!(
        page.title, "Getting started",
        "the reader's title is the published one, not the one being typed"
    );
    assert_eq!(
        page.body, "Connect a marketplace first.",
        "and so is the prose"
    );
    assert_eq!(
        page.topic.as_ref().map(|topic| topic.slug.as_str()),
        Some("selling"),
        "and the filing, which the save also changed"
    );
    assert_eq!(names(&page.tags), vec!["TPT"], "and the tags with it");
    assert_eq!(
        page.published_at, NOW,
        "the reader's update time is the publication, not the autosave"
    );

    let live = repo
        .published(&GuideSearch::default())
        .await
        .expect("the listing reads");
    assert_eq!(
        live.len(),
        1,
        "one published guide, and the save did not add a second"
    );
    assert_eq!(live[0].title, "Getting started", "listed as published");
    assert_eq!(names(&live[0].tags), vec!["TPT"], "with published tags");
    assert_eq!(live[0].published_at, NOW, "at the publication time");
    assert_eq!(
        live[0].source_revision, 1,
        "the snapshot names the draft it was taken from, which is the revision \
         the publisher had read"
    );

    // The phrase that exists only in the working copy.
    let hidden = repo
        .published(&GuideSearch {
            text: Some(&escape_like("dragonfruit")),
            ..GuideSearch::default()
        })
        .await
        .expect("the search reads");
    assert!(
        hidden.is_empty(),
        "a word an operator has typed but not published is not searchable: {:?}",
        listed(&hidden)
    );

    // The reader's vocabulary follows the snapshot too.
    let vocabulary = repo
        .published_taxonomy()
        .await
        .expect("the reader's taxonomy reads");
    assert_eq!(
        names(&vocabulary.topics),
        vec!["Selling"],
        "a topic only a draft names is not a filter a seller is offered"
    );
    assert_eq!(
        names(&vocabulary.tags),
        vec!["TPT"],
        "and nor is a tag only a draft carries"
    );

    // And the operator's own read shows both copies at once, which is what an
    // editor that lost an acknowledgement reconciles against.
    let record = repo
        .get("getting-started")
        .await
        .expect("the guide reads")
        .expect("the guide is there");
    assert_eq!(record.revision, 3, "three writes have happened");
    assert_eq!(
        record.status,
        GuideStatus::Published,
        "and the guide is published, on the strength of the snapshot"
    );
    assert_eq!(
        record.title, "Getting started, rewritten",
        "the operator sees their own working title"
    );
    let published = record.published.expect("the snapshot is there");
    assert_eq!(
        published.title, "Getting started",
        "beside the title sellers are reading"
    );
    assert_eq!(
        published.source_revision, 1,
        "and the revision it came from, which is how a lost acknowledgement is \
         reconciled"
    );
    assert_eq!(published.published_at, NOW, "and when it went out");
}

/// An acknowledgement is what the write did, not what the guide happens to be
/// afterwards.
///
/// This is the overtaken-acknowledgement regression. A write that answered
/// from a read taken after its own commit can be handed a *later* operator's
/// revision, and an editor that believes it advances its base revision past a
/// write it never saw: its next autosave then overwrites that write and is
/// never offered the conflict, because the revision it is naming is the one
/// the overtaking writer produced. The assertion below is on the value the
/// save returned, deliberately held across a second save, so nothing in this
/// test can be satisfied by re-reading the row.
#[sqlx::test(migrations = "./migrations")]
async fn an_acknowledgement_is_the_writes_own_state_and_not_a_later_read(pool: PgPool) {
    provision_operator(&pool)
        .await
        .expect("the operator provisions");
    let repo = GuideRepo::new(pool.clone());
    let selling = taxon(&repo, GuideTaxonKind::Topic, "selling", "Selling").await;
    let tpt = taxon(&repo, GuideTaxonKind::Tag, "tpt", "TPT").await;
    let etsy = taxon(&repo, GuideTaxonKind::Tag, "etsy", "Etsy").await;
    let fees = draft(
        &repo,
        "fees",
        "Fees",
        "The first paragraph.",
        (selling.id, tpt.id),
    )
    .await;

    let first = landed(
        repo.save(
            "fees",
            &GuideEdit {
                title: "Fees",
                body: "The first operator's paragraph.",
                topic: None,
                tags: &[tpt.id],
                updated_by: OPERATOR,
                at: NOW,
            },
            fees,
            1,
        )
        .await
        .expect("the first save lands"),
    );

    // A second operator, working from the revision the first one just
    // produced, overtakes it.
    let second = landed(
        repo.save(
            "fees",
            &GuideEdit {
                title: "Fees, again",
                body: "The second operator's paragraph.",
                topic: None,
                tags: &[etsy.id],
                updated_by: OPERATOR,
                at: LATER,
            },
            fees,
            2,
        )
        .await
        .expect("the second save lands"),
    );

    assert_eq!(
        first.revision, 2,
        "the first save's acknowledgement names the revision that save wrote, \
         not the revision the guide reached afterwards"
    );
    assert_eq!(
        first.body, "The first operator's paragraph.",
        "and carries the prose that save stored"
    );
    assert_eq!(
        names(&first.tags),
        vec!["TPT"],
        "and the filing that save stored"
    );
    assert_eq!(
        second.revision, 3,
        "while the second save's acknowledgement is its own"
    );
    assert_eq!(second.body, "The second operator's paragraph.");

    let now = repo
        .get("fees")
        .await
        .expect("the guide reads")
        .expect("the guide is there");
    assert_eq!(
        now.revision, 3,
        "the stored guide has moved on, which is exactly what an \
         acknowledgement must not silently report as the caller's own"
    );
}

/// Two operators saving the same revision at the same moment: one write, one
/// conflict, and no lost paragraph.
#[sqlx::test(migrations = "./migrations")]
async fn two_writers_at_one_revision_produce_one_winner_and_one_conflict(pool: PgPool) {
    provision_operator(&pool)
        .await
        .expect("the operator provisions");
    let repo = GuideRepo::new(pool.clone());
    let topic = taxon(&repo, GuideTaxonKind::Topic, "selling", "Selling").await;
    let tag = taxon(&repo, GuideTaxonKind::Tag, "tpt", "TPT").await;
    let fees = draft(&repo, "fees", "Fees", "What was there.", (topic.id, tag.id)).await;

    let one = GuideRepo::new(pool.clone());
    let other = GuideRepo::new(pool.clone());
    let tags = [tag.id];
    let first_edit = GuideEdit {
        title: "Fees",
        body: "One operator's paragraph.",
        topic: None,
        tags: &tags,
        updated_by: OPERATOR,
        at: NOW,
    };
    let second_edit = GuideEdit {
        title: "Fees",
        body: "The other operator's paragraph.",
        topic: None,
        tags: &tags,
        updated_by: OPERATOR,
        at: LATER,
    };
    let (first, second) = tokio::join!(
        one.save("fees", &first_edit, fees, 1,),
        other.save("fees", &second_edit, fees, 1,),
    );
    let outcomes = [
        first.expect("the first save answers"),
        second.expect("the second save answers"),
    ];
    let landed_writes: Vec<&GuideRecord> = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            GuideRevisionWrite::Written(record) => Some(record.as_ref()),
            GuideRevisionWrite::Stale { .. } | GuideRevisionWrite::Missing => None,
        })
        .collect();
    assert_eq!(
        landed_writes.len(),
        1,
        "exactly one of two writes at one revision lands: {outcomes:?}"
    );
    assert!(
        outcomes.iter().any(|outcome| matches!(
            outcome,
            GuideRevisionWrite::Stale { id, revision: 2 } if *id == fees
        )),
        "and the other is told the revision that is stored, rather than \
         overwriting it: {outcomes:?}"
    );

    let stored = repo
        .get("fees")
        .await
        .expect("the guide reads")
        .expect("the guide is there");
    assert_eq!(
        stored.revision, 2,
        "one write happened, so the guide moved one revision"
    );
    let acknowledged = landed_writes
        .first()
        .expect("exactly one write landed, as asserted above");
    assert_eq!(
        (acknowledged.revision, acknowledged.body.as_str()),
        (stored.revision, stored.body.as_str()),
        "and the winner's acknowledgement is the guide that is stored, field \
         for field"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_stale_write_changes_nothing_and_names_the_revision_that_is_stored(pool: PgPool) {
    provision_operator(&pool)
        .await
        .expect("the operator provisions");
    let repo = GuideRepo::new(pool.clone());
    let topic = taxon(&repo, GuideTaxonKind::Topic, "selling", "Selling").await;
    let tag = taxon(&repo, GuideTaxonKind::Tag, "tpt", "TPT").await;
    let fees = draft(
        &repo,
        "fees",
        "Fees",
        "The first operator's paragraph.",
        (topic.id, tag.id),
    )
    .await;

    let first = GuideEdit {
        title: "Fees",
        body: "The first operator's paragraph.",
        topic: Some(topic.id),
        tags: &[tag.id],
        updated_by: OPERATOR,
        at: LATER,
    };
    assert_eq!(
        landed(
            repo.save("fees", &first, fees, 1)
                .await
                .expect("the save lands")
        )
        .revision,
        2,
        "the first operator writes at the revision they read"
    );

    // The second operator read revision 1 and is still holding it.
    let second = GuideEdit {
        body: "The second operator's paragraph.",
        ..first
    };
    assert_eq!(
        repo.save("fees", &second, fees, 1)
            .await
            .expect("the stale save answers"),
        GuideRevisionWrite::Stale {
            id: fees,
            revision: 2
        },
        "a stale save is refused with the revision that is stored, so the \
         editor can reconcile rather than guess"
    );
    let record = repo
        .get("fees")
        .await
        .expect("the guide reads")
        .expect("the guide is there");
    assert_eq!(
        record.body, "The first operator's paragraph.",
        "and the refused save wrote nothing: the paragraph that is there is the \
         one that was written"
    );
    assert_eq!(record.revision, 2, "and the revision did not move");

    // A publisher holding the same stale revision cannot ship what they read,
    // and cannot ship what they did not read either.
    assert_eq!(
        repo.publish("fees", fees, 1, OPERATOR, LATER)
            .await
            .expect("the stale publish answers"),
        GuideRevisionWrite::Stale {
            id: fees,
            revision: 2
        },
        "a publish names the draft it agreed to"
    );
    assert!(
        repo.published_page("fees")
            .await
            .expect("the page reads")
            .is_none(),
        "a refused publish publishes nothing at all"
    );

    // A stale delete is refused for the same reason: the operator agreed to
    // delete a particular revision.
    assert_eq!(
        repo.delete("fees", fees, 1)
            .await
            .expect("the stale delete answers"),
        GuideDelete::Stale {
            id: fees,
            revision: 2
        },
        "the destructive write is the last place to accept whatever is there"
    );
    assert!(
        repo.get("fees").await.expect("the guide reads").is_some(),
        "so the guide is still there"
    );

    // And a slug nothing holds is Missing rather than Stale, because the
    // answers differ: one is a collision, the other is a 404.
    assert_eq!(
        repo.save("nothing-here", &first, fees, 1)
            .await
            .expect("the absent save answers"),
        GuideRevisionWrite::Missing,
        "no guide to collide with"
    );
    assert_eq!(
        repo.publish("nothing-here", fees, 1, OPERATOR, LATER)
            .await
            .expect("the absent publish answers"),
        GuideRevisionWrite::Missing,
        "and nothing to publish"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn retiring_a_word_keeps_every_guide_filed_under_it(pool: PgPool) {
    provision_operator(&pool)
        .await
        .expect("the operator provisions");
    let repo = GuideRepo::new(pool.clone());
    let topic = taxon(&repo, GuideTaxonKind::Topic, "selling", "Selling").await;
    let tag = taxon(&repo, GuideTaxonKind::Tag, "tpt", "TPT").await;
    let fees = draft(
        &repo,
        "fees",
        "Fees",
        "What a marketplace keeps.",
        (topic.id, tag.id),
    )
    .await;
    repo.publish("fees", fees, 1, OPERATOR, NOW)
        .await
        .expect("the publish lands");

    let retired = repo
        .update_taxon(GuideTaxonKind::Topic, topic.id, "Selling", true)
        .await
        .expect("the retirement lands")
        .expect("the topic is there");
    assert!(retired.retired, "the topic is withdrawn from the pickers");
    repo.update_taxon(GuideTaxonKind::Tag, tag.id, "TPT", true)
        .await
        .expect("the retirement lands")
        .expect("the tag is there");

    let page = repo
        .published_page("fees")
        .await
        .expect("the page reads")
        .expect("the guide is still published");
    assert_eq!(
        page.topic.as_ref().map(|topic| topic.slug.as_str()),
        Some("selling"),
        "retirement withdraws a word from the pickers; it does not unfile the \
         guides that carry it"
    );
    assert!(
        page.topic.is_some_and(|topic| topic.retired),
        "and the reader is told it is retired, so the console can render it as \
         withdrawn rather than as current"
    );
    assert_eq!(
        names(&page.tags),
        vec!["TPT"],
        "the tags survive retirement too"
    );

    // A retired word a published guide still names is still a filter that
    // answers, which is what makes a shared link keep working.
    let filtered = repo
        .published(&GuideSearch {
            topic: Some(topic.id),
            tags: &[tag.id],
            ..GuideSearch::default()
        })
        .await
        .expect("the filtered listing reads");
    assert_eq!(
        listed(&filtered),
        vec!["fees"],
        "a retired filter still answers the guides published under it"
    );

    let vocabulary = repo
        .published_taxonomy()
        .await
        .expect("the reader's taxonomy reads");
    assert_eq!(
        names(&vocabulary.topics),
        vec!["Selling"],
        "and the word stays in the reader's vocabulary while content names it"
    );
    assert_eq!(names(&vocabulary.tags), vec!["TPT"], "both kinds alike");

    // A retired word still resolves for a write, so an operator saving a guide
    // that already carries it is not trapped.
    let resolved = repo
        .taxa(GuideTaxonKind::Tag, &[tag.id])
        .await
        .expect("the word resolves");
    assert_eq!(
        resolved.len(),
        1,
        "a retired tag can still be written by a guide that already carries it"
    );

    // A topic identifier is not a tag identifier, which no CHECK constraint
    // can say across tables.
    let crossed = repo
        .taxa(GuideTaxonKind::Tag, &[topic.id])
        .await
        .expect("the resolution answers");
    assert!(
        crossed.is_empty(),
        "a topic cannot be filed as a tag by sending its identifier to the \
         wrong field"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn unpublishing_takes_the_page_and_keeps_the_working_copy(pool: PgPool) {
    provision_operator(&pool)
        .await
        .expect("the operator provisions");
    let repo = GuideRepo::new(pool.clone());
    let topic = taxon(&repo, GuideTaxonKind::Topic, "selling", "Selling").await;
    let tag = taxon(&repo, GuideTaxonKind::Tag, "tpt", "TPT").await;
    let fees = draft(
        &repo,
        "fees",
        "Fees",
        "What a marketplace keeps.",
        (topic.id, tag.id),
    )
    .await;
    repo.publish("fees", fees, 1, OPERATOR, NOW)
        .await
        .expect("the publish lands");

    assert_eq!(
        landed(
            repo.unpublish("fees", fees, 2, OPERATOR, LATER)
                .await
                .expect("the withdrawal lands")
        )
        .revision,
        3,
        "withdrawal is a write like any other"
    );
    assert!(
        repo.published_page("fees")
            .await
            .expect("the page reads")
            .is_none(),
        "an unpublished guide reads to a seller exactly as one that was never \
         published"
    );
    assert!(
        repo.published(&GuideSearch::default())
            .await
            .expect("the listing reads")
            .is_empty(),
        "and it leaves the listing"
    );
    assert!(
        repo.published_taxonomy()
            .await
            .expect("the reader's taxonomy reads")
            .tags
            .is_empty(),
        "and its words leave the reader's vocabulary with it"
    );

    let record = repo
        .get("fees")
        .await
        .expect("the guide reads")
        .expect("the guide is there");
    assert_eq!(
        record.status,
        GuideStatus::Draft,
        "the guide is a draft again, which is the only other state there is"
    );
    assert_eq!(
        record.body, "What a marketplace keeps.",
        "the operator keeps every word they had: withdrawal is about the page, \
         not about the draft"
    );
    assert_eq!(
        names(&record.tags),
        vec!["TPT"],
        "and the working filing is untouched"
    );
    assert!(
        record.published.is_none(),
        "with nothing published beside it"
    );

    // And the delete is conditional on what the operator just read.
    assert_eq!(
        repo.delete("fees", fees, 3)
            .await
            .expect("the delete lands"),
        GuideDelete::Deleted,
        "the guide the operator read is the guide that goes"
    );
    assert!(
        repo.get("fees").await.expect("the guide reads").is_none(),
        "and it is gone"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_search_is_literal_and_the_three_filters_narrow_together(pool: PgPool) {
    provision_operator(&pool)
        .await
        .expect("the operator provisions");
    let repo = GuideRepo::new(pool.clone());
    let selling = taxon(&repo, GuideTaxonKind::Topic, "selling", "Selling").await;
    let pricing = taxon(&repo, GuideTaxonKind::Topic, "pricing", "Pricing").await;
    let tpt = taxon(&repo, GuideTaxonKind::Tag, "tpt", "TPT").await;
    let etsy = taxon(&repo, GuideTaxonKind::Tag, "etsy", "Etsy").await;

    let discounts = draft(
        &repo,
        "discounts",
        "Discounts",
        "Take 50% off a bundle.",
        (pricing.id, tpt.id),
    )
    .await;
    repo.publish("discounts", discounts, 1, OPERATOR, NOW)
        .await
        .expect("the publish lands");
    let connecting = draft(
        &repo,
        "connecting",
        "Connecting",
        "Take the token from your shop.",
        (selling.id, etsy.id),
    )
    .await;
    repo.publish("connecting", connecting, 1, OPERATOR, LATER)
        .await
        .expect("the publish lands");

    let literal = repo
        .published(&GuideSearch {
            text: Some(&escape_like("50%")),
            ..GuideSearch::default()
        })
        .await
        .expect("the search reads");
    assert_eq!(
        listed(&literal),
        vec!["discounts"],
        "a per-cent sign is a per-cent sign"
    );

    let wildcarded = repo
        .published(&GuideSearch {
            text: Some(&escape_like("50_")),
            ..GuideSearch::default()
        })
        .await
        .expect("the search reads");
    assert!(
        wildcarded.is_empty(),
        "and an underscore matches an underscore rather than any character: {:?}",
        listed(&wildcarded)
    );

    let everything = repo
        .published(&GuideSearch {
            text: Some(&escape_like("%")),
            ..GuideSearch::default()
        })
        .await
        .expect("the search reads");
    assert_eq!(
        listed(&everything),
        vec!["discounts"],
        "a bare wildcard is a search for that character, not for every guide"
    );

    let cased = repo
        .published(&GuideSearch {
            text: Some(&escape_like("TAKE")),
            ..GuideSearch::default()
        })
        .await
        .expect("the search reads");
    assert_eq!(
        listed(&cased),
        vec!["connecting", "discounts"],
        "newest publication first, and case is not part of the question"
    );

    let by_word = repo
        .published(&GuideSearch {
            text: Some(&escape_like("etsy")),
            ..GuideSearch::default()
        })
        .await
        .expect("the search reads");
    assert_eq!(
        listed(&by_word),
        vec!["connecting"],
        "a guide is found by the name of a word it is filed under"
    );

    // Text AND topic AND any-of-tags: adding a condition never widens.
    let crossed = repo
        .published(&GuideSearch {
            text: Some(&escape_like("Take")),
            topic: Some(pricing.id),
            tags: &[etsy.id],
        })
        .await
        .expect("the search reads");
    assert!(
        crossed.is_empty(),
        "the text matches both, the topic matches one and the tag matches the \
         other, so the conjunction matches neither: {:?}",
        listed(&crossed)
    );

    let narrowed = repo
        .published(&GuideSearch {
            text: Some(&escape_like("Take")),
            topic: Some(pricing.id),
            tags: &[tpt.id, etsy.id],
        })
        .await
        .expect("the search reads");
    assert_eq!(
        listed(&narrowed),
        vec!["discounts"],
        "any of the named tags, and all of the other conditions"
    );
}

/// The migration's own property: what was published under migration 0073
/// keeps reading exactly as it did, and what was a draft stays invisible.
///
/// Applied by hand up to the migration before this one, so there is genuinely
/// old data in the table when 0075 runs. `sqlx::test` would otherwise apply
/// every migration to an empty database, where a data-preserving `UPDATE` has
/// nothing to preserve and passes by doing nothing.
#[sqlx::test(migrations = false)]
async fn a_published_guide_survives_the_snapshot_migration(pool: PgPool) {
    let migrator = sqlx::migrate!("./migrations");
    let mut pending = Vec::new();
    for migration in migrator.iter() {
        if migration.version >= 75 {
            pending.push(migration);
        } else {
            sqlx::raw_sql(migration.sql.as_ref())
                .execute(&pool)
                .await
                .unwrap_or_else(|error| panic!("migration {} applies: {error}", migration.version));
        }
    }

    // Two guides as migration 0073 stored them: one published, one not.
    let at = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(NOW.0)
        .expect("the fixture's instant is an instant");
    for (slug, title, body, status) in [
        (
            "getting-started",
            "Getting started",
            "Connect a marketplace first.",
            "published",
        ),
        ("unfinished", "Unfinished", "Half a sentence.", "draft"),
    ] {
        sqlx::query(
            "INSERT INTO guide (id, slug, title, body, status, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $6)",
        )
        .bind(uuid::Uuid::new_v4())
        .bind(slug)
        .bind(title)
        .bind(body)
        .bind(status)
        .bind(at)
        .execute(&pool)
        .await
        .expect("the legacy guide seeds");
    }

    for migration in pending {
        sqlx::raw_sql(migration.sql.as_ref())
            .execute(&pool)
            .await
            .unwrap_or_else(|error| panic!("migration {} applies: {error}", migration.version));
    }

    let repo = GuideRepo::new(pool.clone());
    let page = repo
        .published_page("getting-started")
        .await
        .expect("the page reads")
        .expect("what was published is still published");
    assert_eq!(page.title, "Getting started", "with the title it had");
    assert_eq!(
        page.body, "Connect a marketplace first.",
        "and the prose it had"
    );
    assert_eq!(
        page.published_at, NOW,
        "the publication time is the time the reader was already being shown"
    );
    assert!(
        page.topic.is_none() && page.tags.is_empty(),
        "and the migration invented no filing: an existing guide starts \
         unfiled rather than under a topic nobody chose"
    );

    assert!(
        repo.published_page("unfinished")
            .await
            .expect("the page reads")
            .is_none(),
        "a guide that was never published is unavailable to readers, which is \
         what it was before"
    );

    let record = repo
        .get("getting-started")
        .await
        .expect("the guide reads")
        .expect("the guide is there");
    assert_eq!(
        record.revision, 1,
        "every existing guide starts at revision 1"
    );
    assert_eq!(
        record.status,
        GuideStatus::Published,
        "on the strength of the snapshot the migration wrote"
    );
    assert_eq!(
        record
            .published
            .expect("the snapshot is there")
            .source_revision,
        1,
        "and its snapshot names the only revision there has ever been"
    );

    let live = repo
        .published(&GuideSearch::default())
        .await
        .expect("the listing reads");
    assert_eq!(
        listed(&live),
        vec!["getting-started"],
        "and the reader's listing is what it was"
    );
}

/// A guide deleted and written again at the same slug is a different guide,
/// and every write the first guide's editor still holds is refused.
///
/// This is the delete-and-recreate regression, and the revisions below are
/// chosen to be the ones a fence made of the revision alone would have
/// accepted. A recreated guide starts at revision 1, which is where the
/// deleted guide was; and an editor that "resynchronises" on the revision a
/// conflict reported would then be holding the replacement's own revision.
/// Both are named here, and both must be refused on the identifier.
///
/// What each assertion checks is not that an answer is a refusal but that the
/// replacement is untouched afterwards: its author's prose, its author's
/// filing, and its author's decision about whether any of it is published.
#[sqlx::test(migrations = "./migrations")]
async fn a_guide_written_again_at_a_freed_slug_refuses_the_old_guides_writes(pool: PgPool) {
    provision_operator(&pool)
        .await
        .expect("the operator provisions");
    let repo = GuideRepo::new(pool.clone());
    let selling = taxon(&repo, GuideTaxonKind::Topic, "selling", "Selling").await;
    let pricing = taxon(&repo, GuideTaxonKind::Topic, "pricing", "Pricing").await;
    let tpt = taxon(&repo, GuideTaxonKind::Tag, "tpt", "TPT").await;
    let etsy = taxon(&repo, GuideTaxonKind::Tag, "etsy", "Etsy").await;

    let first = draft(
        &repo,
        "fees",
        "Fees",
        "The first guide's paragraph.",
        (selling.id, tpt.id),
    )
    .await;
    assert_eq!(
        repo.delete("fees", first, 1)
            .await
            .expect("the delete lands"),
        GuideDelete::Deleted,
        "one operator deletes the guide they read, and the slug is free again"
    );

    // Somebody else writes a new guide at that address. It is a different
    // document by a different author, and it starts where every guide starts.
    let second = draft(
        &repo,
        "fees",
        "Fees, rewritten",
        "The second guide's paragraph.",
        (pricing.id, etsy.id),
    )
    .await;
    assert_ne!(
        second, first,
        "a guide written at a freed slug is a new guide with its own \
         identifier, which is the only thing that tells the two apart"
    );

    let held = GuideEdit {
        title: "Fees",
        body: "The first guide's paragraph, still being typed.",
        topic: Some(selling.id),
        tags: &[tpt.id],
        updated_by: OPERATOR,
        at: LATER,
    };

    // The first guide's editor still has revision 1 open, and the replacement
    // is at revision 1 too. Every one of these four would have matched a fence
    // made of the slug and the revision.
    assert_eq!(
        repo.save("fees", &held, first, 1)
            .await
            .expect("the stale save answers"),
        GuideRevisionWrite::Stale {
            id: second,
            revision: 1
        },
        "a save aimed at a deleted guide does not land on the guide that \
         replaced it, and names what is stored so the editor can see it is a \
         different guide"
    );
    assert_eq!(
        repo.publish("fees", first, 1, OPERATOR, LATER)
            .await
            .expect("the stale publish answers"),
        GuideRevisionWrite::Stale {
            id: second,
            revision: 1
        },
        "and a publish cannot ship somebody else's unfinished draft"
    );
    assert_eq!(
        repo.unpublish("fees", first, 1, OPERATOR, LATER)
            .await
            .expect("the stale withdrawal answers"),
        GuideRevisionWrite::Stale {
            id: second,
            revision: 1
        },
        "nor can a withdrawal reach it"
    );
    assert_eq!(
        repo.delete("fees", first, 1)
            .await
            .expect("the stale delete answers"),
        GuideDelete::Stale {
            id: second,
            revision: 1
        },
        "and the destructive write least of all: deleting a guide twice must \
         not delete the one that took its place"
    );

    let record = repo
        .get("fees")
        .await
        .expect("the guide reads")
        .expect("the replacement is still there");
    assert_eq!(
        record.id, second,
        "the guide at the slug is the replacement"
    );
    assert_eq!(
        record.revision, 1,
        "which four refused writes left exactly where its author put it"
    );
    assert_eq!(
        record.title, "Fees, rewritten",
        "with its author's title rather than the deleted guide's"
    );
    assert_eq!(
        record.body, "The second guide's paragraph.",
        "and its author's prose, word for word"
    );
    assert_eq!(
        record.topic.as_ref().map(|topic| topic.slug.as_str()),
        Some("pricing"),
        "and its author's filing"
    );
    assert_eq!(names(&record.tags), vec!["Etsy"], "tags included");
    assert!(
        record.published.is_none(),
        "and the refused publish published nothing: whether this guide is out \
         is its author's decision, not a stranger's stale click"
    );
    assert!(
        repo.published_page("fees")
            .await
            .expect("the page reads")
            .is_none(),
        "so no seller is reading a draft nobody approved"
    );

    // Its own author publishes it, and the replacement moves to revision 2 —
    // which is the revision the refusals above reported.
    assert_eq!(
        landed(
            repo.publish("fees", second, 1, OPERATOR, NOW)
                .await
                .expect("the publish lands")
        )
        .revision,
        2,
        "the guide's own author publishes it at the revision they read"
    );

    // An editor that treated the conflict as an ordinary one would now write
    // again at the revision it was told. The identifier is what refuses it.
    assert_eq!(
        repo.save("fees", &held, first, 2)
            .await
            .expect("the resynchronised save answers"),
        GuideRevisionWrite::Stale {
            id: second,
            revision: 2
        },
        "naming the reported revision is not enough: the guide is a different \
         guide, and no revision makes the deleted guide's text belong here"
    );
    assert_eq!(
        repo.publish("fees", first, 2, OPERATOR, LATER)
            .await
            .expect("the resynchronised publish answers"),
        GuideRevisionWrite::Stale {
            id: second,
            revision: 2
        },
        "the same for a publish"
    );
    assert_eq!(
        repo.unpublish("fees", first, 2, OPERATOR, LATER)
            .await
            .expect("the resynchronised withdrawal answers"),
        GuideRevisionWrite::Stale {
            id: second,
            revision: 2
        },
        "and for a withdrawal, which would otherwise take down a page this \
         caller has never read"
    );
    assert_eq!(
        repo.delete("fees", first, 2)
            .await
            .expect("the resynchronised delete answers"),
        GuideDelete::Stale {
            id: second,
            revision: 2
        },
        "and for the delete"
    );

    let page = repo
        .published_page("fees")
        .await
        .expect("the page reads")
        .expect("the replacement is still published");
    assert_eq!(
        page.title, "Fees, rewritten",
        "the page a seller reads is the one its author published"
    );
    assert_eq!(page.body, "The second guide's paragraph.", "prose included");
    assert_eq!(names(&page.tags), vec!["Etsy"], "and filing included");

    let after = repo
        .get("fees")
        .await
        .expect("the guide reads")
        .expect("the replacement is still there");
    assert_eq!(after.id, second, "the replacement is still the guide here");
    assert_eq!(
        after.revision, 2,
        "and the only write that moved it is the one its own author made"
    );
    assert_eq!(
        after.body, "The second guide's paragraph.",
        "with the prose untouched by any of the eight refused writes"
    );

    // Both listings name the guide they are rows of, which is what lets a
    // write be made straight from one.
    let heads = repo.list().await.expect("the operator's listing reads");
    let head = heads
        .iter()
        .find(|head| head.slug == "fees")
        .expect("the replacement is listed");
    assert_eq!(
        (head.id, head.revision),
        (second, 2),
        "the operator's row names the guide that is there and the revision a \
         write against it has to name"
    );
    let live = repo
        .published(&GuideSearch::default())
        .await
        .expect("the reader's listing reads");
    assert_eq!(
        live.iter().map(|head| head.id).collect::<Vec<_>>(),
        vec![second],
        "and the reader's row names the same guide"
    );
}
