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

### Task 3: the write path (unblocked by the two write captures, 2026-08-28)

The two founder-supervised captures settled the write path, and the verdict is the TES shape: plain-HTTP, no browser, no bot-management token on any write.
The connector implements `MarketplaceAdapter::submit` and `project_fields` for TPT against the recipe below.

The create flow is an eleven-hop chain, all authenticated by the broker-held cookie jar:
GET the form page and scrape the SecurityComponent triple (`data[_Token][key]` session-scoped, `data[_Token][fields]` a per-form-URL HMAC, `data[_Token][unlocked]` the static field-name list) and the CSRF pair (`data[_Csrf][csrfKey]`/`[csrfToken]`, both equal to the `csrfToken` cookie);
`POST /uploads/upload_file` to reserve an S3 key, returning the object key, the `live.digital.upload` bucket, and the server-chosen path;
`GET /uploads/time` for the server clock that fills `x-amz-date`;
`GET /uploads/sign_auth?to_sign=<StringToSign>` — a server-side AWS Signature V2 oracle that returns the HMAC so the connector holds no AWS secret, called once per S3 request;
the S3 multipart upload against `https://s3.amazonaws.com/live.digital.upload/<path>` (path-style, `Authorization: AWS <keyid>:<sig>`): initiate, PUT each part, complete;
`POST /uploads/process_file`, whose `x-queue-tracking-id` response header — not its body — is the job token;
poll `POST /queue/results` (~1.2s cadence) until `status:2`, whose `data.key` is the file handle posted as `data[ItemDigital][product]`;
`POST /converter/generate_thumbs` and poll again for `thumbnails` and `collection_key`;
`POST /My-Products/New/Digital-Next` with the 43-field multipart form, expecting a `302` to `/Product/<slug>-<id>` as the success signal.
The edit flow is a single `POST /itemsDigital/editNext/{id}` that re-uploads nothing: existing asset handles are echoed back verbatim and `*_uploaded=1` means keep-unchanged.

The x-csrf-token header rides only the XHR endpoints (`/uploads/*`, `/queue/*`, `/converter/*`); the two form POSTs are native navigations carrying CSRF in the form fields alone.
The enum-to-id mapping the form demands is captured (`statusUser "ACTIVE"` posts `status_user=1`, `answerKey "INCLUDED"` posts `answer_key=1`, `copyrightDeclaration "ORIGINAL_WORK"` posts `copyright_declaration=1`, `teachingDuration "HOURS_1"` posts `duration=6`), and `status_user=0` is a draft — the connector creates drafts, never publishing without intent, exactly as the TES driver does.

Three residual questions are settled by live-fire, not by more captures, and none blocks the build: the true required-field minimum is unproven from a success-only capture, so the connector posts the full field set verbatim; the multipart part-size threshold was not crossed (a 224 KB file went single-part), so the multi-part loop is implemented defensively; and cf_clearance cold-start plus login-reCAPTCHA are session-establishment concerns for the broker link step, handled as the TES session is.

## Deferred, with owners

- The session-establishment capture: a cold-start capture from a fresh profile and the intended server egress IP with an empty jar hitting the form page and login, recording whether Cloudflare issues cf_clearance without a managed challenge and whether login demands a reCAPTCHA token. If login needs a browser-minted token, the pragmatic design is a browser-assisted link once, then plain-HTTP writes on the harvested jar — the TES broker pattern. Owner: founder link step + a live probe.
- Live-fire first contact: create one real draft (`status_user=0`) on the founder's own store through the full eleven-hop chain, deletable, to prove the recipe end to end and to measure the required-field minimum, the multi-part threshold on a large file, and the preview/video upload slots (only the product slot and a single-part upload were captured). Owner: a founder-supervised live run, as the TES first charge was.
- The token-lifetime questions: whether `data[_Token][fields]` is deterministic per form URL or nonce-per-render (re-scrape every submit is the safe default the build takes), and the lifetime of the session-scoped `data[_Token][key]`. Owner: settled incidentally by the live run.
- The TPT taxonomy seed: TPT categories are flat tags, not a tree; whether they seed new canonical terms or map onto existing ones via the reconciliation queue follows the M6 taxonomy routes. Owner: M7 Task 2 follow-up.
- The `tax_code_id` id-vs-code axis and the 14-value grade vocabulary: the create form posts a numeric `tax_code_id` and a numeric grade; the registry entry records the tax-code strings and leaves grades unrecorded, both needing a registry-axis decision. Owner: M7 write follow-up.
- Bundle creation: `bundle_max_allowed_files=500` implies a distinct form at `/My-Products/New/Bundle-Next` not covered here. Owner: a later M7 slice.
- Security note, not a task: `/uploads/sign_auth` signs an arbitrary caller-supplied StringToSign for a live AWS key with only a session cookie, and `SellerPayoutPreferences` returns `hyperwalletAuthToken`; the connector must never select the payout fields and should expect the signing oracle to be a fragility TPT may harden. Owner: standing note wherever TPT request text is authored.
- The M2 analytics read: `storeResourceStatsNext` gives TPT per-resource views/sales/earnings; the M2 dashboard consumes it. Owner: M2.
