//! The lowering table: a seller's stated intent plus the mapping's own
//! binding and lifecycle, into the items that realise it.
//!
//! Every row is decided from two stored columns and the target's captured
//! capabilities. Nothing here reads a marketplace, and nothing guesses: a
//! lifecycle the bind never wrote is refused rather than assumed, because the
//! caller would otherwise have to assert a `from` it does not know.
//!
//! It lives beside `MappingSeed` rather than at the API because there are two
//! enqueue paths and only one lowering. `POST /{v}/jobs` lowers the seller's
//! intent directly; `POST /{v}/sync` stores it and the drain lowers it a poll
//! later, on a mapping the same drain has just minted. A second copy of this
//! table is a second answer to "what does live mean", and the one that
//! drifted would be the one nobody enqueues by hand.

use tam_domain::ItemOperation;
use tam_marketplace::{LifecycleTransition, ListingState, RemoteListingId};
use tam_types::InventoryId;

use crate::job_reads::MappingSeed;

/// Why a stated intent cannot be lowered against this mapping. Each is a
/// seller-facing refusal rather than a fault: the remedy is named because the
/// seller is the one who can apply it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoweringRefusal {
    #[error("this listing's state is unknown; verify it first")]
    LifecycleUnknown,
    #[error("{inventory:?} has no captured {capability}, so this edit cannot be attempted yet")]
    UncapturedTransition {
        inventory: InventoryId,
        capability: &'static str,
    },
    #[error("this mapping reads bound but names no listing; verify it first")]
    SubjectUnnamed,
    #[error("this mapping has a create in flight; wait for it to settle")]
    CreateInFlight,
}

/// Why a removal cannot be lowered against this mapping.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RemovalRefusal {
    #[error("this listing is not on that marketplace, so there is nothing to remove")]
    NotThere,
    #[error("this listing's state is unknown; verify it first")]
    LifecycleUnknown,
    #[error("this mapping reads bound but names no listing; verify it first")]
    SubjectUnnamed,
    #[error("this mapping has a create in flight; wait for it to settle")]
    CreateInFlight,
}

/// Lowers one mapping against the state the seller asked it to end up in.
///
/// # Errors
///
/// Every arm the table refuses rather than lowers against a guess.
pub fn lower(
    to: ListingState,
    inventory: InventoryId,
    seed: &MappingSeed,
) -> Result<Vec<ItemOperation>, LoweringRefusal> {
    match seed.binding_state.as_str() {
        // Nothing exists yet, so live is two writes: both adapters create a
        // draft, and the publish names whatever the create bound.
        "unbound" | "severed" => Ok(match to {
            ListingState::Draft => vec![ItemOperation::Create],
            ListingState::Live => vec![ItemOperation::Create, ItemOperation::Publish { to }],
        }),
        "bound" => {
            let from = match seed.lifecycle_state.as_str() {
                "draft" => ListingState::Draft,
                "live" => ListingState::Live,
                // Every mapping written before the bind recorded a lifecycle
                // reads 'absent', and the moderation states no write
                // addresses say nothing either. Refused with the remedy
                // named rather than lowered against a guess.
                _ => return Err(LoweringRefusal::LifecycleUnknown),
            };
            // Tes implements only the two transitions out of draft. Without
            // this the seller gets a 202, then an item that settles skipped
            // with a code that says nothing about why -- on three of five
            // inventories. TPT serves all four.
            if let Some(capability) = uncaptured_transition(inventory, from, to) {
                return Err(LoweringRefusal::UncapturedTransition {
                    inventory,
                    capability,
                });
            }
            Ok(vec![ItemOperation::Revise {
                subject: subject_of(seed)?,
                transition: LifecycleTransition { from, to },
            }])
        }
        // A create in flight, or one whose outcome nobody knows. Either way
        // there is no binding to lower against and enqueuing a second write
        // would be a write on the strength of a guess.
        _ => Err(LoweringRefusal::CreateInFlight),
    }
}

/// Lowers one mapping into the removal of the listing it names, in the state
/// the bind recorded. The state travels with the operation because every
/// adapter's delete is asymmetric between draft and live and discovering it
/// from a probe is unsound (see `MarketplaceAdapter::remove`); the driver's
/// absence poll is what settles the item, never the delete's own status.
///
/// # Errors
///
/// Every arm the table refuses rather than lowers against a guess.
pub fn lower_removal(seed: &MappingSeed) -> Result<ItemOperation, RemovalRefusal> {
    match seed.binding_state.as_str() {
        "unbound" | "severed" => Err(RemovalRefusal::NotThere),
        "bound" => {
            let state = match seed.lifecycle_state.as_str() {
                "draft" => ListingState::Draft,
                "live" => ListingState::Live,
                _ => return Err(RemovalRefusal::LifecycleUnknown),
            };
            let subject = seed.subject.clone().ok_or(RemovalRefusal::SubjectUnnamed)?;
            Ok(ItemOperation::Remove { subject, state })
        }
        _ => Err(RemovalRefusal::CreateInFlight),
    }
}

/// The same table asked of a [`MappingHead`] instead of a [`MappingSeed`].
///
/// A migration preview answers "would this create, or is it refused" before
/// any job exists, and it reads heads because a product with no mapping on the
/// target has no seed to read. The table itself is [`lower`]: this adapts the
/// input rather than restating the rows, because two answers to "what does
/// live mean" is exactly what the module header refuses.
///
/// The three fields lowering reads are the binding, the lifecycle and the
/// subject, and a head carries all three. The payload hashes and the sever
/// generation belong to the idempotency key rather than to the table, so they
/// are empty here and no caller of this may mint an item from the result.
///
/// # Errors
///
/// Whatever [`lower`] refuses this mapping.
pub fn lower_head(
    to: ListingState,
    inventory: InventoryId,
    head: &crate::mapping::MappingHead,
) -> Result<Vec<ItemOperation>, LoweringRefusal> {
    lower(
        to,
        inventory,
        &MappingSeed {
            mapping: head.id,
            product: head.product,
            payload_hashes: Vec::new(),
            sever_generation: 0,
            binding_state: head.binding_state.clone(),
            lifecycle_state: head.lifecycle_state.clone(),
            subject: head.remote.clone(),
        },
    )
}

/// Whether a publish must wait for the create that binds the listing it
/// publishes, stated where the lowering is rather than at each caller.
///
/// FIFO within the job usually gets the order right and does not when the
/// two items share a `created_at` and the tie-break is a fresh uuid, so the
/// gate is the item's own rather than the queue's.
#[must_use]
pub const fn requires_bound_on(
    operation: &ItemOperation,
    inventory: InventoryId,
) -> Option<InventoryId> {
    match operation {
        ItemOperation::Publish { .. } => Some(inventory),
        ItemOperation::Create | ItemOperation::Revise { .. } | ItemOperation::Remove { .. } => None,
    }
}

/// The subject a bound mapping's revise addresses. `mapping_seeds` carries
/// the binding's spelling and not its id, so the id is read where the row
/// already has to be hydrated -- and a bound row without one is a corrupt
/// row rather than a seller error.
fn subject_of(seed: &MappingSeed) -> Result<RemoteListingId, LoweringRefusal> {
    seed.subject.clone().ok_or(LoweringRefusal::SubjectUnnamed)
}

/// Which transition this inventory has no capture for, if any. A registry
/// lookup rather than a new concept: the adapters already refuse these with
/// the capability named, and refusing at the API means the seller is told
/// before an item is enqueued rather than after it settles.
#[must_use]
pub const fn uncaptured_transition(
    inventory: InventoryId,
    from: ListingState,
    to: ListingState,
) -> Option<&'static str> {
    match (inventory, from, to) {
        (InventoryId::Tes, ListingState::Live, ListingState::Live) => Some("tes.edit_published"),
        (InventoryId::Tes, ListingState::Live, ListingState::Draft) => Some("tes.unpublish"),
        _ => None,
    }
}

/// Which capture a sync's *source* read is waiting on, if any.
///
/// The same registry as the transition table above and the reason the sync
/// endpoint can refuse before the drain leases anything: a request naming a
/// source whose download the drain cannot perform is one it could pick up
/// every poll and never serve.
///
/// TPT was listed here until 2026-09-13, because the download route met a
/// sign-in gate under a server-side cookie jar. That was the transport and
/// not the route: under the seller's own session, on the seller's own
/// device, an owned resource's `Download` hop answers 302 to a signed asset
/// URL that serves the archive, and the founder-supervised capture
/// witnessed one. The row is gone because the capability exists, and the
/// device is where it exists -- which is D1's rule rather than an exception
/// to it.
///
/// Etsy stays listed: no download of a seller's own files has been captured
/// there at all.
#[must_use]
pub const fn uncaptured_source(inventory: InventoryId) -> Option<&'static str> {
    match inventory {
        InventoryId::Tes | InventoryId::Tpt => None,
        InventoryId::Etsy => Some("etsy.download_resource_bundle"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        lower, lower_removal, requires_bound_on, uncaptured_source, LoweringRefusal, RemovalRefusal,
    };
    use crate::job_reads::MappingSeed;
    use tam_domain::ItemOperation;
    use tam_marketplace::{ListingState, RemoteListingId};
    use tam_types::{InventoryId, MappingId, ProductId, Uuid};

    fn seed(binding_state: &str, lifecycle_state: &str) -> MappingSeed {
        MappingSeed {
            mapping: MappingId(Uuid([0x11; 16])),
            product: ProductId(Uuid([0x22; 16])),
            payload_hashes: vec![],
            sever_generation: 0,
            binding_state: binding_state.to_owned(),
            lifecycle_state: lifecycle_state.to_owned(),
            subject: None,
        }
    }

    #[test]
    fn a_live_intent_on_nothing_lowers_to_a_create_and_a_gated_publish() {
        let operations = lower(
            ListingState::Live,
            InventoryId::Tes,
            &seed("unbound", "absent"),
        )
        .expect("an unbound mapping lowers");
        assert_eq!(
            operations,
            vec![
                ItemOperation::Create,
                ItemOperation::Publish {
                    to: ListingState::Live
                }
            ],
            "both adapters create a draft, so live is two writes"
        );
        assert_eq!(
            requires_bound_on(&operations[1], InventoryId::Tes),
            Some(InventoryId::Tes),
            "the publish names the listing the create bound, so it waits for it"
        );
        assert_eq!(
            requires_bound_on(&operations[0], InventoryId::Tes),
            None,
            "the create waits for nothing"
        );
    }

    #[test]
    fn a_draft_intent_on_nothing_lowers_to_a_create_alone() {
        assert_eq!(
            lower(
                ListingState::Draft,
                InventoryId::Tes,
                &seed("unbound", "absent")
            )
            .expect("an unbound mapping lowers"),
            vec![ItemOperation::Create],
        );
    }

    #[test]
    fn a_bound_mapping_with_no_observed_lifecycle_is_refused_rather_than_guessed() {
        assert_eq!(
            lower(
                ListingState::Live,
                InventoryId::Tes,
                &seed("bound", "absent")
            ),
            Err(LoweringRefusal::LifecycleUnknown),
        );
    }

    #[test]
    fn a_create_in_flight_refuses_a_second_write() {
        assert_eq!(
            lower(
                ListingState::Draft,
                InventoryId::Tes,
                &seed("creating", "absent")
            ),
            Err(LoweringRefusal::CreateInFlight),
        );
    }

    #[test]
    fn a_removal_carries_the_state_the_bind_recorded_and_waits_for_nothing() {
        let subject = RemoteListingId::Tes {
            url: "https://www.tes.com/api/v2/resources/9001".to_owned(),
        };
        let bound = MappingSeed {
            subject: Some(subject.clone()),
            ..seed("bound", "live")
        };
        let operation = lower_removal(&bound).expect("a bound live listing is removable");
        assert_eq!(
            operation,
            ItemOperation::Remove {
                subject,
                state: ListingState::Live
            },
            "the delete route is chosen from the recorded state, never from a probe"
        );
        assert_eq!(requires_bound_on(&operation, InventoryId::Tes), None);
    }

    #[test]
    fn a_removal_of_nothing_is_refused_before_anything_is_enqueued() {
        assert_eq!(
            lower_removal(&seed("unbound", "absent")),
            Err(RemovalRefusal::NotThere)
        );
        assert_eq!(
            lower_removal(&seed("bound", "absent")),
            Err(RemovalRefusal::LifecycleUnknown)
        );
        assert_eq!(
            lower_removal(&seed("bound", "draft")),
            Err(RemovalRefusal::SubjectUnnamed),
            "a bound row naming no listing is corrupt, not removable"
        );
        assert_eq!(
            lower_removal(&seed("creating", "absent")),
            Err(RemovalRefusal::CreateInFlight)
        );
    }

    #[test]
    fn etsy_alone_has_no_captured_seller_download() {
        assert_eq!(uncaptured_source(InventoryId::Tes), None);
        assert_eq!(
            uncaptured_source(InventoryId::Tpt),
            None,
            "the own-file download the supervised capture witnessed on the seller's device"
        );
        assert_eq!(
            uncaptured_source(InventoryId::Etsy),
            Some("etsy.download_resource_bundle"),
            "the capability the adapter itself refuses with, so both name one thing"
        );
    }
}
