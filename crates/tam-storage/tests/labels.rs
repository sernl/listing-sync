//! Seller-defined labels: what a write leaves behind, how a label is resolved
//! by name, and the tenant fence around both tables.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::CanonicalProduct;
use tam_storage::{Colour, LabelRecord, LabelRename, LabelRepo, ProductRepo};
use tam_types::{
    ContentHash, FileBytes, FileId, FileKind, FileRole, OrgId, PayloadSet, ProductFile, ProductId,
    ScanOutcome, Timestamp, Uuid,
};

mod common;
use common::{minimal_product, seed_org_a, ORG_A, PRODUCT_1};

const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const PRODUCT_2: ProductId = ProductId(Uuid([0x02; 16]));
const NOW: Timestamp = Timestamp(5_000);

/// A second product in the same organisation, with its own payload file:
/// `product_file` is keyed `(org_id, id)`, so two products in one tenant
/// cannot share a file identifier.
fn second_product() -> CanonicalProduct {
    let mut product = minimal_product();
    product.id = PRODUCT_2;
    product.payload = Some(PayloadSet::new(
        ProductFile {
            id: FileId(Uuid([0x22; 16])),
            role: FileRole::Payload,
            kind: FileKind::Pdf,
            bytes: FileBytes::Held {
                hash: ContentHash([0x52; 32]),
                byte_len: 4,
                scan: ScanOutcome::Pending,
            },
        },
        vec![],
    ));
    product
}

fn names(records: &[tam_storage::LabelRecord]) -> Vec<String> {
    records.iter().map(|record| record.name.clone()).collect()
}

#[sqlx::test(migrations = "./migrations")]
async fn a_write_replaces_the_whole_set_and_mints_labels_by_name(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("the product inserts");
    let repo = LabelRepo::new(pool.clone());

    let written = repo
        .set_for_product(
            ORG_A,
            PRODUCT_1,
            &["Autumn term".to_owned(), "Bundle".to_owned()],
            NOW,
        )
        .await
        .expect("the labels write");
    assert_eq!(names(&written), vec!["Autumn term", "Bundle"]);

    // Replace rather than merge: the set sent is the set held, so a label left
    // out of the write is off the item.
    let replaced = repo
        .set_for_product(ORG_A, PRODUCT_1, &["Bundle".to_owned()], NOW)
        .await
        .expect("the second write lands");
    assert_eq!(names(&replaced), vec!["Bundle"]);
    assert_eq!(
        names(
            &repo
                .for_product(ORG_A, PRODUCT_1)
                .await
                .expect("the read-back reads")
        ),
        vec!["Bundle"]
    );

    // "Autumn term" is on no item now, so it has left the organisation's
    // vocabulary and the board's filter will not offer it.
    assert_eq!(
        names(&repo.list(ORG_A).await.expect("the vocabulary reads")),
        vec!["Bundle"]
    );

    let emptied = repo
        .set_for_product(ORG_A, PRODUCT_1, &[], NOW)
        .await
        .expect("an empty write lands");
    assert!(
        emptied.is_empty(),
        "an empty set is expressible, which is what makes removing the last label possible"
    );
}

/// The same word is the same label across the catalogue, whatever case it
/// arrives in; that is what makes filtering by it mean anything.
#[sqlx::test(migrations = "./migrations")]
async fn one_word_is_one_label_across_the_catalogue(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let products = ProductRepo::new(pool.clone());
    products
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("the first product inserts");
    products
        .insert(ORG_A, &second_product(), NOW)
        .await
        .expect("the second product inserts");
    let repo = LabelRepo::new(pool.clone());

    repo.set_for_product(ORG_A, PRODUCT_1, &["Autumn term".to_owned()], NOW)
        .await
        .expect("the first item is labelled");
    let second = repo
        .set_for_product(ORG_A, PRODUCT_2, &["autumn TERM".to_owned()], NOW)
        .await
        .expect("the second item is labelled");

    assert_eq!(
        names(&second),
        vec!["Autumn term"],
        "the second write finds the label the first minted rather than making a rival spelling"
    );
    assert_eq!(
        repo.list(ORG_A).await.expect("the vocabulary reads").len(),
        1,
        "one word, one label"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_labels_colour_is_derived_from_its_name_and_is_stable(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("the product inserts");
    let repo = LabelRepo::new(pool.clone());

    let first = repo
        .set_for_product(ORG_A, PRODUCT_1, &["Autumn term".to_owned()], NOW)
        .await
        .expect("the labels write");
    assert_eq!(
        first[0].colour,
        Colour::of_name("Autumn term"),
        "the stored colour is the one the name earns, not one the client chose"
    );
    assert_eq!(
        Colour::of_name("Autumn term"),
        Colour::of_name("autumn TERM"),
        "case folds before the colour is derived, so one label cannot have two colours"
    );
}

/// The fence, probed as the tenancy suite probes every other one: a second
/// tenant sees none of the first's labels, and the same word in two tenants is
/// two independent labels.
#[sqlx::test(migrations = "./migrations")]
async fn labels_do_not_cross_the_tenant_fence(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-b', now())")
        .bind(uuid::Uuid::from_bytes(ORG_B.0 .0))
        .execute(&pool)
        .await
        .expect("org b seeds");
    let products = ProductRepo::new(pool.clone());
    products
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("tenant a's product inserts");
    let mut theirs = minimal_product();
    theirs.id = PRODUCT_2;
    theirs.org = ORG_B;
    products
        .insert(ORG_B, &theirs, NOW)
        .await
        .expect("tenant b's product inserts");

    let repo = LabelRepo::new(pool.clone());
    repo.set_for_product(ORG_A, PRODUCT_1, &["Autumn term".to_owned()], NOW)
        .await
        .expect("tenant a labels its item");

    assert!(
        repo.list(ORG_B)
            .await
            .expect("tenant b's vocabulary reads")
            .is_empty(),
        "tenant b sees none of tenant a's labels"
    );
    assert!(
        repo.for_product(ORG_B, PRODUCT_1)
            .await
            .expect("the read is fenced rather than refused")
            .is_empty(),
        "naming tenant a's product from tenant b answers nothing"
    );

    let theirs = repo
        .set_for_product(ORG_B, PRODUCT_2, &["Autumn term".to_owned()], NOW)
        .await
        .expect("tenant b labels its own item");
    assert_eq!(
        names(&theirs),
        vec!["Autumn term"],
        "the one-per-name index is per tenant, not global"
    );
    assert_eq!(
        repo.list(ORG_A)
            .await
            .expect("tenant a's vocabulary reads")
            .len(),
        1,
        "tenant b's write did not touch tenant a's vocabulary"
    );
}

/// The filter is a clause in the page query, so it must narrow the page rather
/// than shorten it after the fact.
#[sqlx::test(migrations = "./migrations")]
async fn the_catalogue_page_narrows_to_one_label(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let products = ProductRepo::new(pool.clone());
    products
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("the first product inserts");
    products
        .insert(ORG_A, &second_product(), NOW)
        .await
        .expect("the second product inserts");
    LabelRepo::new(pool.clone())
        .set_for_product(ORG_A, PRODUCT_1, &["Autumn term".to_owned()], NOW)
        .await
        .expect("only the first item is labelled");

    let all = products
        .list_page(ORG_A, None, 50, None)
        .await
        .expect("the unfiltered page reads");
    assert_eq!(all.len(), 2);

    let filtered = products
        .list_page(ORG_A, None, 50, Some("Autumn term"))
        .await
        .expect("the filtered page reads");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, PRODUCT_1);

    let folded = products
        .list_page(ORG_A, None, 50, Some("autumn TERM"))
        .await
        .expect("the folded filter reads");
    assert_eq!(
        folded.len(),
        1,
        "the filter folds case, matching the index that decides label identity"
    );

    let unknown = products
        .list_page(ORG_A, None, 50, Some("no such label"))
        .await
        .expect("an unknown label reads");
    assert!(
        unknown.is_empty(),
        "a label nothing carries is an empty page rather than an error"
    );
}

/// The filter must be a clause in the page query, not a sieve over the page it
/// returns. Limit one with the label on the *second* product is what tells the
/// two apart: a post-filter would fetch the first product, discard it, and
/// answer nothing.
#[sqlx::test(migrations = "./migrations")]
async fn the_label_filter_narrows_the_query_rather_than_the_page(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let products = ProductRepo::new(pool.clone());
    products
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("the first product inserts");
    products
        .insert(ORG_A, &second_product(), NOW)
        .await
        .expect("the second product inserts");
    LabelRepo::new(pool.clone())
        .set_for_product(ORG_A, PRODUCT_2, &["Bundle".to_owned()], NOW)
        .await
        .expect("only the second item is labelled");

    let unfiltered = products
        .list_page(ORG_A, None, 1, None)
        .await
        .expect("the first page reads");
    assert_eq!(
        unfiltered.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![PRODUCT_1],
        "the first page of the whole catalogue is the first product"
    );

    let filtered = products
        .list_page(ORG_A, None, 1, Some("Bundle"))
        .await
        .expect("the filtered first page reads");
    assert_eq!(
        filtered.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![PRODUCT_2],
        "a one-row page of the filtered catalogue is the labelled item, which only a clause in \
         the query can answer"
    );
}

/// A deleted item carries no labels, and a label only it carried leaves the
/// vocabulary with it: otherwise the board's filter offers a word whose page is
/// always empty.
#[sqlx::test(migrations = "./migrations")]
async fn deleting_an_item_takes_its_labels_with_it(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let products = ProductRepo::new(pool.clone());
    products
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("the first product inserts");
    products
        .insert(ORG_A, &second_product(), NOW)
        .await
        .expect("the second product inserts");
    let repo = LabelRepo::new(pool.clone());
    repo.set_for_product(
        ORG_A,
        PRODUCT_1,
        &["Only mine".to_owned(), "Shared".to_owned()],
        NOW,
    )
    .await
    .expect("the first item is labelled");
    repo.set_for_product(ORG_A, PRODUCT_2, &["Shared".to_owned()], NOW)
        .await
        .expect("the second item is labelled");

    assert!(
        products
            .soft_delete(ORG_A, PRODUCT_1, NOW)
            .await
            .expect("the delete lands"),
        "the product was there to delete"
    );

    assert!(
        repo.for_product(ORG_A, PRODUCT_1)
            .await
            .expect("the deleted item's labels read")
            .is_empty(),
        "a deleted item carries nothing"
    );
    assert_eq!(
        names(&repo.list(ORG_A).await.expect("the vocabulary reads")),
        vec!["Shared"],
        "the label only the deleted item carried is gone; the one another item still carries stays"
    );
    assert!(
        products
            .list_page(ORG_A, None, 50, Some("Only mine"))
            .await
            .expect("the filtered page reads")
            .is_empty(),
        "the filter cannot find the deleted item by the label it used to carry"
    );
}

/// A rename is a write on the one label row, so the items carrying it are
/// carried across without being visited, and the colour follows the new name
/// because it is derived from it.
#[sqlx::test(migrations = "./migrations")]
async fn a_rename_keeps_every_carrier_and_takes_the_new_names_colour(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let products = ProductRepo::new(pool.clone());
    products
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("the first product inserts");
    products
        .insert(ORG_A, &second_product(), NOW)
        .await
        .expect("the second product inserts");
    let repo = LabelRepo::new(pool.clone());
    for product in [PRODUCT_1, PRODUCT_2] {
        repo.set_for_product(ORG_A, product, &["Autumn term".to_owned()], NOW)
            .await
            .expect("the item is labelled");
    }

    let renamed = repo
        .rename(ORG_A, "autumn TERM", "Term one")
        .await
        .expect("the rename runs");
    assert_eq!(
        renamed,
        LabelRename::Renamed(LabelRecord {
            name: "Term one".to_owned(),
            colour: Colour::of_name("Term one"),
            system: false,
        }),
        "the label is addressed by name the way the unique index folds it, and \
         the stored colour is the one the new name earns"
    );

    assert_eq!(
        names(&repo.list(ORG_A).await.expect("the vocabulary reads")),
        vec!["Term one"],
        "one label, under its new name"
    );
    for product in [PRODUCT_1, PRODUCT_2] {
        assert_eq!(
            names(
                &repo
                    .for_product(ORG_A, product)
                    .await
                    .expect("the item's labels read")
            ),
            vec!["Term one"],
            "every item that carried the old name carries the new one"
        );
    }
    assert_eq!(
        products
            .list_page(ORG_A, None, 50, Some("Term one"))
            .await
            .expect("the filtered page reads")
            .len(),
        2,
        "the filter finds both items under the new name"
    );
}

/// The two answers a rename gives that are not a rename, both of which move
/// zero rows and mean different things to the seller.
#[sqlx::test(migrations = "./migrations")]
async fn a_rename_onto_a_name_in_use_is_refused_and_an_unknown_one_is_reported(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("the product inserts");
    let repo = LabelRepo::new(pool.clone());
    repo.set_for_product(
        ORG_A,
        PRODUCT_1,
        &["Autumn term".to_owned(), "Bundle".to_owned()],
        NOW,
    )
    .await
    .expect("the item is labelled");

    assert_eq!(
        repo.rename(ORG_A, "Autumn term", "bundle")
            .await
            .expect("the refused rename is an answer rather than a fault"),
        LabelRename::Taken,
        "the unique index folds case, so a name differing only in case is in use"
    );
    assert_eq!(
        names(&repo.list(ORG_A).await.expect("the vocabulary reads")),
        vec!["Autumn term", "Bundle"],
        "the refusal left both labels as they were"
    );
    assert_eq!(
        repo.rename(ORG_A, "Spring term", "Term two")
            .await
            .expect("the rename runs"),
        LabelRename::Missing,
        "a label this organisation does not have is missing rather than taken"
    );
    assert_eq!(
        repo.rename(ORG_A, "Autumn term", "AUTUMN TERM")
            .await
            .expect("the rename runs"),
        LabelRename::Renamed(LabelRecord {
            name: "AUTUMN TERM".to_owned(),
            colour: Colour::of_name("AUTUMN TERM"),
            system: false,
        }),
        "a label may be renamed to its own name in another case: the row it \
         would collide with is itself"
    );
}

/// Deleting a label takes it off every item carrying it, by the cascade
/// migration 0046 argues for, and leaves the rest of the vocabulary standing.
#[sqlx::test(migrations = "./migrations")]
async fn deleting_a_label_takes_it_off_every_item(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let products = ProductRepo::new(pool.clone());
    products
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("the first product inserts");
    products
        .insert(ORG_A, &second_product(), NOW)
        .await
        .expect("the second product inserts");
    let repo = LabelRepo::new(pool.clone());
    for product in [PRODUCT_1, PRODUCT_2] {
        repo.set_for_product(
            ORG_A,
            product,
            &["Autumn term".to_owned(), "Bundle".to_owned()],
            NOW,
        )
        .await
        .expect("the item is labelled");
    }

    assert!(
        repo.delete(ORG_A, "autumn term")
            .await
            .expect("the delete runs"),
        "the label was there to remove, whatever case it was asked for in"
    );
    for product in [PRODUCT_1, PRODUCT_2] {
        assert_eq!(
            names(
                &repo
                    .for_product(ORG_A, product)
                    .await
                    .expect("the item's labels read")
            ),
            vec!["Bundle"],
            "the removed label is off every item, and the other one is untouched"
        );
    }
    assert!(
        products
            .list_page(ORG_A, None, 50, Some("Autumn term"))
            .await
            .expect("the filtered page reads")
            .is_empty(),
        "nothing can be found by a label that no longer exists"
    );
    assert!(
        !repo
            .delete(ORG_A, "Autumn term")
            .await
            .expect("the second delete runs"),
        "asking again is answered rather than reported as a success that did nothing"
    );
}

/// The fence around both writes: neither reaches a label of another tenant
/// that happens to share a name.
#[sqlx::test(migrations = "./migrations")]
async fn a_rename_and_a_delete_stop_at_the_tenant_fence(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-b', now())")
        .bind(uuid::Uuid::from_bytes(ORG_B.0 .0))
        .execute(&pool)
        .await
        .expect("org b seeds");
    let products = ProductRepo::new(pool.clone());
    products
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("org a's product inserts");
    let repo = LabelRepo::new(pool.clone());
    repo.set_for_product(ORG_A, PRODUCT_1, &["Autumn term".to_owned()], NOW)
        .await
        .expect("org a's item is labelled");

    assert_eq!(
        repo.rename(ORG_B, "Autumn term", "Term one")
            .await
            .expect("the rename runs"),
        LabelRename::Missing,
        "org b cannot rename a label it does not have, however org a spells its own"
    );
    assert!(
        !repo
            .delete(ORG_B, "Autumn term")
            .await
            .expect("the delete runs"),
        "org b cannot delete a label it does not have"
    );
    assert_eq!(
        names(&repo.list(ORG_A).await.expect("org a's vocabulary reads")),
        vec!["Autumn term"],
        "org a's label is untouched by either call"
    );
}
