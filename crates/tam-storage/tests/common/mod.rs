//! Fixture values shared by the integration-test binaries that hang rows off
//! one minimal catalogue product.

use sqlx::PgPool;
use tam_domain::{CanonicalProduct, DeclarationSource, GradeDeclaration};
use tam_types::{
    ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, ListingCopy, OrgId, PayloadSet,
    PriceIntent, ProductFile, ProductId, ScanOutcome, Title, Uuid,
};

pub(crate) const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
// Each integration binary compiles this module separately and uses a
// different subset, so unuse is per-binary and expected here.
#[allow(dead_code)]
pub(crate) const PRODUCT_1: ProductId = ProductId(Uuid([0x01; 16]));

#[allow(dead_code)]
pub(crate) fn minimal_product() -> CanonicalProduct {
    CanonicalProduct {
        id: PRODUCT_1,
        org: ORG_A,
        title: Title("Fixture product".to_owned()),
        body: ListingCopy {
            body: "Fixture body.".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload: Some(PayloadSet::new(
            ProductFile {
                id: FileId(Uuid([0x21; 16])),
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
        grades: GradeDeclaration {
            source: DeclarationSource::Seller,
            raw: vec![],
            derived: None,
        },
        price: PriceIntent::Free,
        rights: tam_domain::RightsDeclaration::Unstated,
        native_residue: vec![],
    }
}

#[allow(dead_code)]
pub(crate) async fn seed_org_a(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .execute(pool)
        .await?;
    Ok(())
}
