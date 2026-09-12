//! The OpenAPI document, hand-built as data: the route table below is one
//! constant, the generator folds it into the document, and the parity test
//! probes every documented route against the mounted router — so the
//! document cannot drift silently toward fiction. Publishing, metering,
//! per-tier rate limiting and SDK generation stay deferred until a real
//! third-party consumer exists, exactly as the specification defers them.

use axum::extract::State;
use axum::Json;

use crate::version::APIVersion;
use crate::AppState;

/// One documented operation. `path` is the mounted axum path with the
/// version parameter still abstract; the document renders it under the
/// concrete version it is served for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    pub method: &'static str,
    pub path: &'static str,
    pub summary: &'static str,
}

/// Every operation this build serves. Mounting happens in `router()`;
/// documenting happens here; the parity test holds the two together.
pub const ROUTES: [Route; 114] = [
    Route {
        method: "get",
        path: "/healthz",
        summary: "Unversioned liveness probe",
    },
    Route {
        method: "get",
        path: "/{version}/healthz",
        summary: "Versioned health probe echoing the negotiated version",
    },
    Route {
        method: "get",
        path: "/{version}/whoami",
        summary: "The organisation and user the session speaks for",
    },
    Route {
        method: "post",
        path: "/{version}/session",
        summary: "Exchange a minted token for the HttpOnly session cookie",
    },
    Route {
        method: "delete",
        path: "/{version}/session",
        summary: "Expire the session and clear the cookie",
    },
    Route {
        method: "get",
        path: "/{version}/org",
        summary: "The calling organisation's own settings",
    },
    Route {
        method: "patch",
        path: "/{version}/org",
        summary: "Rename the calling organisation, claim its slug, or both",
    },
    Route {
        method: "get",
        path: "/{version}/org/slug/{slug}",
        summary: "Whether a slug is free, advisory; the write's 409 is authoritative",
    },
    Route {
        method: "get",
        path: "/{version}/billing",
        summary: "The calling organisation's subscription state, or none",
    },
    Route {
        method: "post",
        path: "/{version}/billing/webhook",
        summary: "Paddle's notification endpoint; the signature is the authentication",
    },
    Route {
        method: "get",
        path: "/{version}/mappings",
        summary: "The flat mapping listing the product-by-inventory table joins",
    },
    Route {
        method: "post",
        path: "/{version}/mappings/overrides",
        summary: "Record one seller's own answer for how a term of theirs projects",
    },
    Route {
        method: "get",
        path: "/{version}/mappings/overrides",
        summary: "The calling organisation's own projection overrides",
    },
    Route {
        method: "delete",
        path: "/{version}/mappings/overrides",
        summary: "Withdraw one override, leaving the global relation to answer again",
    },
    Route {
        method: "post",
        path: "/{version}/mappings/{mapping}/bind",
        summary: "Bind a mapping to a listing by its URL, contacting no marketplace",
    },
    Route {
        method: "get",
        path: "/{version}/devices/{device}/reads",
        summary: "What this device should capture from the marketplace, or nothing",
    },
    Route {
        method: "post",
        path: "/{version}/devices/{device}/reads",
        summary: "The capture this device made, recorded as snapshots",
    },
    Route {
        method: "get",
        path: "/{version}/analytics/summary",
        summary: "The newest captured figure for every listing that has one",
    },
    Route {
        method: "get",
        path: "/{version}/status",
        summary: "Public per-marketplace status from the fleet kill switch",
    },
    Route {
        method: "get",
        path: "/{version}/notifications",
        summary: "Finished runs, newest first, with their settled counts",
    },
    Route {
        method: "post",
        path: "/{version}/notifications/read",
        summary: "Mark every notification up to a given one read",
    },
    Route {
        method: "get",
        path: "/{version}/notifications/preferences",
        summary: "Whether the requesting user takes completion mail",
    },
    Route {
        method: "patch",
        path: "/{version}/notifications/preferences",
        summary: "Set whether the requesting user takes completion mail",
    },
    Route {
        method: "get",
        path: "/{version}/profile",
        summary: "The requesting user's own profile: which picture they set",
    },
    Route {
        method: "get",
        path: "/{version}/profile/avatar",
        summary: "The requesting user's own picture, as bytes",
    },
    Route {
        method: "put",
        path: "/{version}/profile/avatar",
        summary: "Set the requesting user's picture to an uploaded handle",
    },
    Route {
        method: "delete",
        path: "/{version}/profile/avatar",
        summary: "Clear the requesting user's picture",
    },
    Route {
        method: "post",
        path: "/{version}/jobs",
        summary: "Start a sync as an operation resource; Idempotency-Key required",
    },
    Route {
        method: "get",
        path: "/{version}/jobs",
        summary: "List jobs, newest first, by opaque keyset cursor",
    },
    Route {
        method: "post",
        path: "/{version}/sync",
        summary: "Enqueue a sync, a migration or a bulk; one endpoint, one per resource",
    },
    Route {
        method: "get",
        path: "/{version}/sync/{request}",
        summary: "A sync request's state, its per-resource states, and the jobs it produced",
    },
    Route {
        method: "get",
        path: "/{version}/jobs/{job}",
        summary: "The job roll-up: raw item counts, never a scalar verdict",
    },
    Route {
        method: "get",
        path: "/{version}/jobs/{job}/items",
        summary: "Page the job's items by opaque keyset cursor, filtered by outcome",
    },
    Route {
        method: "get",
        path: "/{version}/jobs/{job}/items/{item}",
        summary: "One item with its event timeline, the downloadable result",
    },
    Route {
        method: "get",
        path: "/{version}/events/stream",
        summary: "SSE ledger projection; Last-Event-ID resumes, 204 stops",
    },
    Route {
        method: "post",
        path: "/{version}/uploads",
        summary: "Ingest one upload's bytes and return the handles a create names",
    },
    Route {
        method: "get",
        path: "/{version}/uploads/{handle}",
        summary: "One sealed image of this organisation's, by the handle an upload returned",
    },
    Route {
        method: "get",
        path: "/{version}/products",
        summary: "Page the catalogue by opaque keyset cursor",
    },
    Route {
        method: "post",
        path: "/{version}/products",
        summary: "Author a draft product and one unbound mapping per selected platform",
    },
    Route {
        method: "get",
        path: "/{version}/products/export",
        summary: "The whole catalogue as CSV, with its per-marketplace state",
    },
    Route {
        method: "get",
        path: "/{version}/imports/template",
        summary: "The import workbook, generated from the registry",
    },
    Route {
        method: "post",
        path: "/{version}/imports",
        summary: "Parse and hold a filled workbook; creates nothing",
    },
    Route {
        method: "get",
        path: "/{version}/imports",
        summary: "This organisation's spreadsheet imports, newest first",
    },
    Route {
        method: "get",
        path: "/{version}/imports/{batch}",
        summary: "One import with its row-by-row report and its warnings",
    },
    Route {
        method: "delete",
        path: "/{version}/imports/{batch}",
        summary: "Abandon an open import, releasing its hold on the next one",
    },
    Route {
        method: "post",
        path: "/{version}/imports/{batch}/rows/{sheet}/{ordinal}/file",
        summary: "Bind one row to the payload and cover an upload already sealed",
    },
    Route {
        method: "delete",
        path: "/{version}/imports/{batch}/rows/{sheet}/{ordinal}/file",
        summary: "Release one row's hold on the bytes bound to it",
    },
    Route {
        method: "post",
        path: "/{version}/imports/{batch}/commit",
        summary: "Create the next chunk of an import's resources; enqueues nothing",
    },
    Route {
        method: "post",
        path: "/{version}/imports/runs",
        summary: "Open a marketplace import run for the seller's device to read",
    },
    Route {
        method: "get",
        path: "/{version}/imports/runs",
        summary: "This organisation's import runs, newest first",
    },
    Route {
        method: "get",
        path: "/{version}/imports/runs/{run}",
        summary: "One run with its items, its counts and its open duplicate questions",
    },
    Route {
        method: "post",
        path: "/{version}/imports/runs/{run}/select",
        summary: "Choose every listed resource or a named few; the rest are skipped",
    },
    Route {
        method: "post",
        path: "/{version}/imports/runs/{run}/commit",
        summary: "Create the next chunk of a run's matched resources; drafts nothing",
    },
    Route {
        method: "post",
        path: "/{version}/imports/runs/{run}/abandon",
        summary: "Settle an open run the seller has given up on",
    },
    Route {
        method: "get",
        path: "/{version}/imports/runs/{run}/items/{ordinal}/cover",
        summary: "The cover one read produced, before any product exists to hold it",
    },
    Route {
        method: "get",
        path: "/{version}/duplicates",
        summary: "The duplicate questions still waiting on the seller",
    },
    Route {
        method: "post",
        path: "/{version}/duplicates/{lo}/{hi}",
        summary: "Answer one pair: the same resource, different resources, or later",
    },
    Route {
        method: "post",
        path: "/{version}/duplicates/{lo}/{hi}/undo",
        summary: "Reverse a merge, inside the thirty days the seller was told",
    },
    Route {
        method: "get",
        path: "/{version}/products/{product}",
        summary: "The product aggregate: files, subjects, verbatim grades",
    },
    Route {
        method: "patch",
        path: "/{version}/products/{product}",
        summary: "Edit the canonical fields, refusing an uncaptured live transition",
    },
    Route {
        method: "delete",
        path: "/{version}/products/{product}",
        summary: "Soft-delete locally and enqueue a removal per elected platform",
    },
    Route {
        method: "post",
        path: "/{version}/products/{product}/mappings",
        summary: "Add a marketplace to an existing product, as one unbound mapping",
    },
    Route {
        method: "post",
        path: "/{version}/products/{product}/files",
        summary: "Add one file to an existing product, naming a handle the upload returned",
    },
    Route {
        method: "put",
        path: "/{version}/products/{product}/files/{file}",
        summary: "Swap one file's bytes, keeping its role and redrawing the thumbnail here",
    },
    Route {
        method: "delete",
        path: "/{version}/products/{product}/files/{file}",
        summary: "Retire one file, refusing the removal that would leave no payload",
    },
    Route {
        method: "get",
        path: "/{version}/products/{product}/cover",
        summary: "The bytes of one resource's cover, for a browser to draw",
    },
    Route {
        method: "get",
        path: "/{version}/products/{product}/labels",
        summary: "The seller's own labels on one item",
    },
    Route {
        method: "put",
        path: "/{version}/products/{product}/labels",
        summary: "Replace the labels on one item, minting any the org has not used",
    },
    Route {
        method: "get",
        path: "/{version}/templates",
        summary: "The organisation's saved starting points for a new resource",
    },
    Route {
        method: "post",
        path: "/{version}/templates",
        summary: "Save a named partial draft of the create form",
    },
    Route {
        method: "get",
        path: "/{version}/templates/{template}",
        summary: "One template whole, with the draft a new resource is prefilled from",
    },
    Route {
        method: "patch",
        path: "/{version}/templates/{template}",
        summary: "Rename a template, replace its draft, or both",
    },
    Route {
        method: "delete",
        path: "/{version}/templates/{template}",
        summary: "Remove a template",
    },
    Route {
        method: "get",
        path: "/{version}/labels",
        summary: "Every label this organisation uses, which the board's filter lists",
    },
    Route {
        method: "patch",
        path: "/{version}/labels/{name}",
        summary: "Rename one label, keeping every item that carries it",
    },
    Route {
        method: "delete",
        path: "/{version}/labels/{name}",
        summary: "Remove one label from the organisation and from every item",
    },
    Route {
        method: "get",
        path: "/{version}/vocabulary/{inventory}",
        summary: "One marketplace's authoring vocabulary, so the form is data-driven",
    },
    Route {
        method: "get",
        path: "/{version}/authoring/vocabulary",
        summary: "Every controlled list the canonical create form renders",
    },
    Route {
        method: "post",
        path: "/{version}/authoring/check",
        summary: "What the create form refuses, decided by the server rather than the client",
    },
    Route {
        method: "get",
        path: "/{version}/standards/search",
        summary: "Education standards for one jurisdiction, or the not-ingested state",
    },
    Route {
        method: "get",
        path: "/{version}/taxonomy/terms",
        summary: "The canonical terms a create body's subjects field names, filterable by kind",
    },
    Route {
        method: "get",
        path: "/{version}/devices",
        summary: "The seller's own machines and the marketplaces each one holds",
    },
    Route {
        method: "post",
        path: "/{version}/devices",
        summary: "Register this device, or refresh what a known one says about itself",
    },
    Route {
        method: "post",
        path: "/{version}/devices/{device}/heartbeat",
        summary: "A device's check-in; the answer tells it whether it has been signed out",
    },
    Route {
        method: "post",
        path: "/{version}/devices/{device}/revoke",
        summary: "Sign one device out; it wipes its marketplace sessions on next contact",
    },
    Route {
        method: "post",
        path: "/{version}/connections/{marketplace}/authorship",
        summary: "The seller declares who authored what this connection publishes",
    },
    Route {
        method: "get",
        path: "/{version}/connections",
        summary: "The marketplace connections and their link states",
    },
    Route {
        method: "post",
        path: "/{version}/connections/{connection}/revoke",
        summary: "Revoke a connection through the credential broker",
    },
    Route {
        method: "post",
        path: "/{version}/connections/{connection}/disconnect",
        summary: "Disconnect a marketplace; connecting again on a device restores it",
    },
    Route {
        method: "post",
        path: "/{version}/marketplace-requests",
        summary: "Ask for a marketplace this platform does not sell on yet",
    },
    Route {
        method: "get",
        path: "/{version}/reconciliation/items",
        summary: "The open reconciliation queue",
    },
    Route {
        method: "post",
        path: "/{version}/reconciliation/items/{item}/resolve",
        summary: "Resolve a queue item by authoring a durable Exact edge",
    },
    Route {
        method: "post",
        path: "/{version}/reconciliation/items/{item}/no-counterpart",
        summary: "Record that the term has no counterpart; the mapping omits it",
    },
    Route {
        method: "get",
        path: "/{version}/reconciliation/stats",
        summary: "The drain counters the kill gate reads",
    },
    Route {
        method: "get",
        path: "/{version}/elections/items",
        summary: "The seller's open decisions, with the candidates read live",
    },
    Route {
        method: "post",
        path: "/{version}/elections/items/{item}/answer",
        summary: "Answer one decision; the items it blocked requeue in the same transaction",
    },
    Route {
        method: "post",
        path: "/{version}/elections/items/{item}/withdraw",
        summary: "Withdraw a decision nobody needs answered, without recording an answer",
    },
    Route {
        method: "post",
        path: "/{version}/elections/rules",
        summary: "Record a standing answer, so the same question is never asked twice",
    },
    Route {
        method: "get",
        path: "/{version}/elections/delegation",
        summary: "Which axes of which marketplaces the seller has handed to best fit",
    },
    Route {
        method: "put",
        path: "/{version}/elections/delegation",
        summary: "Tick or untick best fit for one marketplace, as durable revocable rules",
    },
    Route {
        method: "get",
        path: "/{version}/admin/signups",
        summary: "Operator: signups per day from the identity and app planes",
    },
    Route {
        method: "get",
        path: "/{version}/plans",
        summary: "The price list, its capabilities, the import ladder and the founding offer",
    },
    Route {
        method: "get",
        path: "/{version}/entitlement",
        summary: "What this organisation holds, what it grants and what it has used",
    },
    Route {
        method: "get",
        path: "/{version}/admin/orgs",
        summary: "Operator: every organisation with its per-tenant counts",
    },
    Route {
        method: "get",
        path: "/{version}/admin/orgs/{org}",
        summary: "Operator: one organisation, its connections, halts, subscription state, \
                  plan and grant history",
    },
    Route {
        method: "post",
        path: "/{version}/admin/orgs/{org}/plan",
        summary: "Operator: grant a plan or rung, with a reason and an optional expiry",
    },
    Route {
        method: "post",
        path: "/{version}/admin/orgs/{org}/plan/{grant}/revoke",
        summary: "Operator: withdraw a grant, keeping its audit row",
    },
    Route {
        method: "get",
        path: "/{version}/admin/sync-health",
        summary: "Operator: the ledger by item state across every tenant",
    },
    Route {
        method: "get",
        path: "/{version}/admin/failed-writes",
        summary: "Operator: write attempts that failed or are stranded in flight",
    },
    Route {
        method: "get",
        path: "/{version}/admin/import-drain",
        summary: "Operator: the import-drain measurement series per tenant",
    },
    Route {
        method: "get",
        path: "/{version}/admin/dead-letters",
        summary: "Operator: outbox messages the drainer gave up on, counted by topic",
    },
    Route {
        method: "get",
        path: "/{version}/admin/impersonations",
        summary: "Operator: the identity plane's impersonation record, newest first",
    },
    Route {
        method: "get",
        path: "/{version}/admin/marketplace-requests",
        summary: "Operator: the marketplaces sellers have asked for, newest first",
    },
    Route {
        method: "get",
        path: "/{version}/openapi.json",
        summary: "This document",
    },
];

/// Builds the document for one concrete version. Paths keep their remaining
/// parameters in OpenAPI's own brace form, which axum's happens to match.
#[must_use]
pub fn document(version: APIVersion) -> serde_json::Value {
    let concrete = version.as_str();
    let mut paths = serde_json::Map::new();
    for route in ROUTES {
        let path = route.path.replace("{version}", concrete);
        let entry = paths
            .entry(path)
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let serde_json::Value::Object(operations) = entry {
            operations.insert(
                route.method.to_owned(),
                serde_json::json!({
                    "summary": route.summary,
                    "responses": {
                        "default": {
                            "description": "Failures carry the structured APIError body \
                                            with its closed code and kind vocabulary."
                        }
                    }
                }),
            );
        }
    }
    serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": "listing-sync internal API",
            "version": concrete,
            "description": "Internal until a third-party consumer exists; \
                            cookie-session authenticated; idempotency keys on \
                            every sync-starting request."
        },
        "paths": serde_json::Value::Object(paths),
    })
}

pub(crate) async fn serve_document(
    version: APIVersion,
    State(_state): State<AppState>,
) -> Json<serde_json::Value> {
    Json(document(version))
}

#[cfg(test)]
mod tests {
    use super::{document, ROUTES};
    use crate::version::APIVersion;

    #[test]
    fn every_route_documents_under_the_concrete_version() {
        let doc = document(APIVersion::V1);
        let paths = doc
            .get("paths")
            .and_then(|paths| paths.as_object())
            .map(serde_json::Map::len);
        // Derived from the table rather than restated as a number. A path
        // carrying several methods is one entry in the document and several
        // rows in the table -- `/{version}/products` takes a get and a post,
        // `/{version}/mappings/overrides` three -- so the two counts differ by
        // however many methods share a path. That is a fact of the table, and
        // a hand-maintained offset beside it is a second copy of it that every
        // route addition has to remember to update.
        let mut distinct: Vec<&str> = ROUTES.iter().map(|route| route.path).collect();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            paths,
            Some(distinct.len()),
            "every path the table names reaches the document"
        );

        // The half a count cannot see on its own: two rows with the same
        // method on the same path collapse into one document entry, and the
        // assertion above would still agree with itself while an operation
        // had silently vanished.
        let declared = ROUTES.len();
        let mut operations: Vec<(&str, &str)> = ROUTES
            .iter()
            .map(|route| (route.path, route.method))
            .collect();
        operations.sort_unstable();
        operations.dedup();
        assert_eq!(
            operations.len(),
            declared,
            "each operation lands in the document exactly once"
        );
    }
}
