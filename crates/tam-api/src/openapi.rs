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
pub const ROUTES: [Route; 44] = [
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
        summary: "Rename the calling organisation",
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
        method: "get",
        path: "/{version}/vocabulary/{inventory}",
        summary: "One marketplace's authoring vocabulary, so the form is data-driven",
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
        path: "/{version}/admin/signups",
        summary: "Operator: signups per day from the identity and app planes",
    },
    Route {
        method: "get",
        path: "/{version}/admin/orgs",
        summary: "Operator: every organisation with its per-tenant counts",
    },
    Route {
        method: "get",
        path: "/{version}/admin/orgs/{org}",
        summary: "Operator: one organisation, its connections, halts and subscription state",
    },
    Route {
        method: "get",
        path: "/{version}/admin/sync-health",
        summary: "Operator: the ledger by item state across every tenant",
    },
    Route {
        method: "get",
        path: "/{version}/admin/failed-writes",
        summary: "Operator: write attempts carrying a failure code, newest first",
    },
    Route {
        method: "get",
        path: "/{version}/admin/impersonations",
        summary: "Operator: the identity plane's impersonation record, newest first",
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
        // /v1/jobs, /v1/session and /v1/org each carry two operations,
        // /v1/products carries two and /v1/products/{product} three, so
        // distinct paths are six fewer than the operations in the table.
        assert_eq!(
            paths,
            Some(ROUTES.len() - 6),
            "each operation lands in the document exactly once"
        );
    }
}
