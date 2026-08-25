//! The idempotency key: ours, because no marketplace in scope offers one, and
//! keyed on the INVENTORY rather than the marketplace — both ends of a Tes
//! GB-to-US duplication share tenant, marketplace, product, intent version
//! and content hash, so a marketplace-keyed tuple would collide on
//! `UNIQUE (org_id, idempotency_key)` and the second half of the first
//! chargeable product would silently never run.
//!
//! UUIDv5 over a fixed-width canonical encoding, per
//! `docs/design/sync-machine.md`: every field is fixed-width, so no separator
//! is needed and no length-extension ambiguity exists. A requeued item
//! recomputes the same key, which is what lets a connection transitioning to
//! `NeedsReauth` requeue every item behind a gate without any losing identity.

use tam_types::{ContentHash, InventoryId, OrgId, ProductId, NAMESPACE_TAM_INTENT};

use crate::IdempotencyKey;

/// The ordinal is part of the canonical encoding and therefore forever:
/// variants append, existing ordinals never renumber. The test below pins
/// each one.
const fn inventory_ordinal(inventory: InventoryId) -> u8 {
    match inventory {
        InventoryId::TesGb => 0,
        InventoryId::TesUs => 1,
        InventoryId::Etsy => 2,
        InventoryId::Tpt => 3,
    }
}

#[must_use]
pub fn derive_idempotency_key(
    org: OrgId,
    inventory: InventoryId,
    product: ProductId,
    intent_version: u32,
    intent_hash: ContentHash,
) -> IdempotencyKey {
    let mut canonical = [0u8; 69];
    canonical[0..16].copy_from_slice(&org.0 .0);
    canonical[16] = inventory_ordinal(inventory);
    canonical[17..33].copy_from_slice(&product.0 .0);
    canonical[33..37].copy_from_slice(&intent_version.to_be_bytes());
    canonical[37..69].copy_from_slice(&intent_hash.0);
    let namespace = uuid::Uuid::from_bytes(NAMESPACE_TAM_INTENT.0);
    let derived = uuid::Uuid::new_v5(&namespace, &canonical);
    IdempotencyKey(tam_types::Uuid(derived.into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::{derive_idempotency_key, inventory_ordinal};
    use tam_types::{ContentHash, InventoryId, OrgId, ProductId, Uuid};

    const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
    const PRODUCT: ProductId = ProductId(Uuid([0x01; 16]));
    const HASH: ContentHash = ContentHash([0x51; 32]);

    #[test]
    fn the_ordinals_are_forever() {
        for (inventory, ordinal) in [
            (InventoryId::TesGb, 0u8),
            (InventoryId::TesUs, 1),
            (InventoryId::Etsy, 2),
            (InventoryId::Tpt, 3),
        ] {
            assert_eq!(
                inventory_ordinal(inventory),
                ordinal,
                "renumbering an ordinal re-keys every item in flight"
            );
        }
    }

    #[test]
    fn the_derivation_is_deterministic() {
        let first = derive_idempotency_key(ORG, InventoryId::TesGb, PRODUCT, 1, HASH);
        let again = derive_idempotency_key(ORG, InventoryId::TesGb, PRODUCT, 1, HASH);
        assert_eq!(
            first, again,
            "a requeued item must recompute the same key or lose its identity"
        );
    }

    #[test]
    fn the_two_tes_inventories_do_not_collide() {
        let gb = derive_idempotency_key(ORG, InventoryId::TesGb, PRODUCT, 1, HASH);
        let us = derive_idempotency_key(ORG, InventoryId::TesUs, PRODUCT, 1, HASH);
        assert_ne!(
            gb, us,
            "a marketplace-keyed tuple would collide here and the second half \
             of the first chargeable product would silently never run"
        );
    }

    #[test]
    fn version_and_hash_both_move_the_key() {
        let base = derive_idempotency_key(ORG, InventoryId::TesGb, PRODUCT, 1, HASH);
        let bumped = derive_idempotency_key(ORG, InventoryId::TesGb, PRODUCT, 2, HASH);
        let rehashed =
            derive_idempotency_key(ORG, InventoryId::TesGb, PRODUCT, 1, ContentHash([0x52; 32]));
        assert_ne!(base, bumped, "a new intent version is a new write");
        assert_ne!(base, rehashed, "new content is a new write");
    }
}
