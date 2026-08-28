# M7: the TPT connector

Goal: a TeachersPayTeachers connector on the M6 seam — the first-party read that enumerates a seller's own TPT catalogue, the registry entry that names TPT's fields and vocabularies, and the write path that creates a listing.
This plan is written against six HAR captures of the founder's own TPT seller account (`~/downloads/ebmc/TPT/`), analysed 2026-08-28.
It differs from the TES connector in three ways the captures make unavoidable, and it draws a hard line between what those captures let us build now and what a further capture must settle first.

## What the captures settled

TPT is not one API.
The read surface is two GraphQL services on `www.teacherspayteachers.com`: `/graph/graphql` owns the seller and product domain, `/gateway/graphql` owns store analytics and account.
Both authenticate identically — a cookie session (`sessionKey`/`TPT`) plus a double-submit CSRF token where the `csrfToken` cookie value is mirrored verbatim into an `x-csrf-token` header — and the gateway adds one header, `x-gateway-auth-version: 2`.
No bearer token, no OAuth, no persisted-query hashes: every request carries its full query text.
`MyProductListings` on `/graph/graphql` is the seller-owns-these enumeration query, fully specified with its `MyResourceFields` fragment; it is the reconciliation read the connector needs, and it needs no further capture.
The statistics surface is a bonus for M2: `storeResourceStatsNext` and `storeResourceTotalsAllTimeStats` on the gateway expose per-resource EARNINGS, RESOURCE_VIEWS, DOWNLOADS, SALES_COUNT, WISHLISTED and more — so, contrary to the earlier platform-API finding that no marketplace exposes views, TPT does, for the seller's own resources.

## The three differences from TES

The write is a legacy CakePHP multipart form, not GraphQL.
The create page server-renders `<form id="ItemAddForm" action="/My-Products/New/Digital-Next" method="post" enctype="multipart/form-data">` whose fields are `data[Model][field]` tuples, and it is protected by a CakePHP SecurityComponent token triple plus a separate CSRF pair.
The SecurityComponent hashes the posted field set, so the connector cannot synthesise a submit: it must GET the form immediately before each write, lift all six token values verbatim, and post exactly the whitelisted field set the hash was computed over, tokens presumed single-use.

Files upload out of band, not in the form.
The form's file slots are hidden text inputs that carry a handle, not `<input type=file>`; the binaries go to Evaporate-to-S3 and/or Filestack (a `marketplaceWorkflowId` is configured, implying an async virus-scan/transform workflow with a status to poll), and the returned handle is written back into the hidden field before the form posts.
None of that upload sequence is in the captures.

A three-layer bot-management stack fronts the origin.
Cloudflare Bot Management mints a `cf_clearance` cookie from an obfuscated-JS fingerprint (`cloudflare-captcha: true` is enabled for this account); reCAPTCHA Enterprise v3 warms a score token on every page load; Sift device-fingerprinting is deployed.
In every capture all three ran passively and nothing blocked, but every capture rode a `cf_clearance` cookie obtained before recording began, and no capture exercised a gated write.

## The line this plan draws

The read slice is buildable now and is not rework-prone: the auth envelope, `MyProductListings`, and TPT's vocabularies are all fully captured and stable.
It is the M6 "a new platform is an adapter plus data" proof on a real second platform, and it feeds M2.

The write slice is capture-gated.
A connector cannot be built to a write no capture contains, and the four milestone measurements plus the three risks above all resolve on a single artefact: a capture of one product created and published, from a browser with tracking protection disabled so the bot-management layers behave as they do for a real user.
That capture settles, at once: whether the submit carries a reCAPTCHA Enterprise token (and under what action string); the full Evaporate/Filestack upload sequence and how the handle rejoins the form; whether a product write needs a Sift beacon; the real mandatory-field set, the minimum price and its currency, and the title/description/tag bounds; and whether the write is really this CakePHP form or a newer GraphQL mutation the React surface is migrating to.

The architecture fork is downstream of that capture, not settled now.
If reads and the write pass from a server egress IP carrying only a broker-established cookie jar, the connector is the TES shape — a sans-io session via the broker.
If Cloudflare rejects a non-browser client, or the write demands a freshly executed reCAPTCHA token, the connector needs a headless browser at the egress IP, which reopens the deliberately-closed `tam-browser` decision and is a founder call.
The design removed `tam-browser` at M-1; re-admitting one is not a thing this plan does on its own authority.

## Tasks

### Task 1: the TPT read adapter (buildable now)

A `tam-marketplace-tpt` crate with a `TptAdapter<T: Transport>` implementing the `FirstPartyExport` seam's `list_own_resources` against `MyProductListings`, reusing the sans-io `Transport` seam so the live-session question stays a broker concern.
The auth envelope is constructor state: the cookie jar and the mirrored CSRF header, never request data, exactly as the TES adapter keeps its session.
`fetch_for_import` is deferred with the M6 import-run generalisation: `ImportedListing` is Tes-shaped (licence tokens, GBP, age ranges) and TPT's model is flat taxonomy tags with no licence, so canonicalising TPT waits until the import run is generalised.
Tests are cassette-driven from the captured `MyProductListings` response.

### Task 2: the TPT registry entry

Populate `InventoryId::Tpt` in the field registry from the captured facts: the flat taxonomy-tag namespace, the tax-code vocabulary (five values), the education-standards jurisdictions, custom categories as a seller-owned shelf taxonomy, and the observed bounds recorded as observed-not-proven (title to 80, description cap 45000 client-side, minimum price 0.95 in an unconfirmed currency, the per-slot upload caps).
Vocabularies we hold get `Closed`; documented-but-unmeasured bounds get their honest state, never a guess.

### Task 3: the write path (capture-gated — not started without the create capture)

The CakePHP form writer: GET the form, lift the SecurityComponent and CSRF tokens, project the canonical listing onto the `data[Model][field]` whitelist, drive the out-of-band upload, write the handle back, and post.
`project_fields` on the seam renders the TPT field set the way the TES adapter renders its own.
This task's field set, upload sequence, and bot-management handling come from the create capture; it is specified only in outline until then.

## Deferred, with owners

- The create-and-publish capture: tracking protection disabled, one product through to published, plus ideally one edit of an existing listing. Owner: founder. This gates Task 3 entirely.
- The cold-start probe: call `MyProductListings` from the intended server egress IP with a broker-established jar and no browser-minted `cf_clearance`, to learn whether reads need a headless browser. Owner: a live probe once the read adapter exists.
- The browser-vs-session architecture decision: downstream of the capture and the probe; re-admitting `tam-browser` is founder-gated. Owner: founder.
- The TPT taxonomy seed: TPT categories are flat tags, not a tree; whether they seed new canonical terms or map onto existing ones via the reconciliation queue follows the M6 taxonomy routes. Owner: M7 Task 2 follow-up.
- The M2 analytics read: `storeResourceStatsNext` gives TPT per-resource views/sales/earnings; the M2 dashboard consumes it. Owner: M2.
