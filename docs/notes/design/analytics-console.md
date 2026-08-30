# Analytics console groundwork

What marketplace analytics this repository can already read, what it does not keep, and what a persisted read path would cost.

This is a design note, not a decision.
Nothing described under "A minimal persistence design" or later is built, and the investigation behind it was read-only.
No marketplace was contacted; every claim below rests on the tree.
Paths are this repository, cited with line numbers.

## Verdict

Marketplace analytics is fetch-only.
One adapter can read per-listing totals from Teachers Pay Teachers on demand, and no code anywhere writes a row when it does.
There is therefore nothing for a console endpoint to read, and no read endpoint is proposed here.

## What exists today

The entire analytics surface is three files in `crates/tam-marketplace-tpt`, landed with the M7 connector.

`endpoints.rs` builds the two GraphQL requests.
`all_time_stats_request` asks the gateway for all-time per-resource totals for one metric over a batch of resource ids (`crates/tam-marketplace-tpt/src/endpoints.rs:449`).
`resolved_stats_request` asks for the same shape resolved over a time window, at day or month granularity (`:484`, `:398`, `:415`).
The batch is capped at one hundred ids, which is the size proven on the wire; a larger batch is refused rather than attempted, because the true ceiling is untested (`:424`).

`read_model.rs` parses the answer into `ResourceStat`, which is a TPT resource id and one `f64` (`crates/tam-marketplace-tpt/src/read_model.rs:189-195`, `:401`).
The value is kept as read rather than reinterpreted, because the wire carries an untyped number that means a count for sales, an amount for earnings and a ratio for the Easel assign rate (`:191-194`).
The resource id is TPT's own `u64`, which converts to `RemoteListingId::Tpt` (`:17-24`).

`flows.rs` holds the two adapter methods, `all_time_stats` and `resolved_stats` (`crates/tam-marketplace-tpt/src/flows.rs:293`, `:310`).
Both are inherent on `TptAdapter` rather than on the `MarketplaceAdapter` seam, deliberately: TPT is the only platform whose analytics can be read, and a capability with one implementor is not yet a seam (`:289-292`).
Both refuse any `FetchReason` other than `FirstPartyExport`, which is the tier-one permission for the marketplace's own export of the seller's data (`:299-301`, `:315-317`, `crates/tam-marketplace/src/lib.rs:113-115`).
Both run the batch cap before issuing anything (`crates/tam-marketplace-tpt/src/flows.rs:259`).

Twelve all-time metrics are named, including sales count, earnings, downloads, resource views, wishlisted and rating (`crates/tam-marketplace-tpt/src/endpoints.rs:331-344`).
Seven time-resolved metrics are named, of which the members beyond earnings and resource views were read out of the statistics page bundle and have not been seen executed (`:367-380`).

Nothing outside that crate calls any of it.
A search of `crates/` for `all_time_stats`, `resolved_stats` and `ResourceStat` matches only inside `crates/tam-marketplace-tpt`.
No migration under `crates/tam-storage/migrations` declares a table, column or view for views, sales, earnings or any other metric.
`tam-storage`, `tam-engine`, `tam-sync-worker`, `tam-worker` and `tam-api` contain no analytics code at all.
`DrainStats` and the `/{version}/reconciliation/stats` route are taxonomy-queue counters and are unrelated to marketplace analytics.
The Tes adapter has no statistics read of any kind.

## A minimal persistence design

One table, appended to and never updated in place.

    listing_metric_snapshot
        org_id       uuid        NOT NULL REFERENCES organisation (id)
        mapping_id   uuid        NOT NULL
        metric       text        NOT NULL
        observed_at  timestamptz NOT NULL
        total_value  double precision NOT NULL
        PRIMARY KEY (org_id, mapping_id, metric, observed_at)

It is keyed on the mapping rather than the product because a metric belongs to one listing on one marketplace, and one product may carry several mappings.
The mapping already names its inventory, so no inventory column is needed.
The value is stored as the adapter read it, for the reason the adapter keeps it that way: the wire number means different things per metric and this table is not the place to decide which.
A snapshot row rather than a latest-value row, because the all-time read gives a running total and a rate over any period is the difference between two of them; a table that overwrote would destroy the only way to compute one.
It would carry the same row-level-security policy every tenant table carries, keyed on `app.current_org` (`crates/tam-storage/migrations/0001_product.sql:32-44`).
Growth is bounded by listings times metrics times captures, so a retention cut would be needed before the first year; `PruneRepo` is the existing shape for that (`crates/tam-storage/src/pruning.rs`).

The capture seam should be a new one-run-per-invocation binary, modelled on `tam-canary`.
The canary is exactly that shape already: a systemd timer owns the schedule, the process does one pass, and its exit status is the alert (`crates/tam-canary/src/main.rs:1-6`).
Such a binary would read each tenant's bound TPT mappings, batch their remote ids at one hundred per request, and write one row per listing per metric per pass.
Folding the read into `tam-worker`'s poll loop instead would be worse, because the pump is item-driven while an analytics pass is tenant-driven, and a poll interval is not a schedule a seller can be told.
The specification already names `tam-scheduler.service` as the fleet's cron owner (`docs/design/2026-08-25-listing-sync-design.md:66`), and no such crate exists yet.
Either way the schedule is a timer this project operates, so nothing here makes any part of sync agent-driven (`docs/design/decisions.md:34`).

Two pieces are missing before that binary could be written.
The remote id lives only in the mapping's binding, as `Binding::Bound { id: RemoteListingId::Tpt { .. } }` (`crates/tam-domain/src/lib.rs:200-212`), and the existing flat read projects the binding as a state label rather than as an identifier (`crates/tam-storage/src/mapping.rs:929-940`), so a new repo read is needed.
The read also needs a live TPT session, which comes from the credential broker over its unix socket under a lease, and `LeasePurpose` currently admits only `Pump` and `Drain` (`crates/tam-engine/src/broker_client.rs:23-26`).

## The endpoint the console would consume

`GET /{version}/analytics/summary`, org-scoped through the `OrgContext` extractor like every other route, carrying no organisation identifier of its own.

    {
      "listings": [
        {
          "mapping": "uuid",
          "inventory": "tpt",
          "observed_at": 1756512000000,
          "metrics": { "resource_views": 1234.0, "sales_count": 17.0 }
        }
      ]
    }

Each row is the newest snapshot per mapping per metric.
`observed_at` travels per row rather than once per response, so the console states how stale a figure is instead of presenting a captured number as a live one.
A tenant whose first capture has not run answers an empty list, which is honest and is what a first render needs.
Paging would use the same opaque keyset cursor the catalogue page already mints, added when a tenant's listing count justifies it rather than before.

## Open questions for the founder

1. Which metrics are captured? One request reads one metric for one batch, so twelve all-time metrics is twelve times the request volume against a marketplace we would rather touch less, and a shortlist is cheaper than the full set.
2. How often is a capture worth taking? Daily is the cheapest schedule a seller would still call current; hourly multiplies both request volume and table growth.
3. Does the console ship TPT-only? Tes has no statistics read, so a seller with Tes listings would see an analytics page that is empty for reasons unrelated to their sales.
4. How long are snapshots kept? Retention decides whether the table needs a prune pass in the first release or later.
5. Does an analytics pass get its own `LeasePurpose`, or reuse `Drain`? A third purpose keeps the broker audit legible; reuse avoids touching the broker protocol.
6. Is earnings stored as a number or as money? `total_value` is an untyped `f64`, while this codebase's `Money` carries a currency and refuses a non-positive amount, so storing earnings as a bare float is a deliberate exception rather than an oversight.
