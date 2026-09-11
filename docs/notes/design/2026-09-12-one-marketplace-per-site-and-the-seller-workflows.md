# One Tes, and the seller workflows: import, migration, scheduling, sync, templates, collections, backoffice, themes

- date: 2026-09-12
- status: design accepted by the founder on 2026-09-12 (six decisions recorded in `docs/design/decisions.md` under that date); phase 0 in flight
- inputs: the founder's feedback of 2026-09-12 on the 2026-09-11 release, `~/downloads/teachouse-ui-change-{1,2}.png`, the audits under `agent://ScoutTesOneSite`, `agent://ScoutFeatureState`, and three research reports copied beside this note: `research/2026-09-12-dedup-and-collections.md` (21 sources), `research/2026-09-12-pricing-and-tiers.md` (30 sources, every price read on 2026-09-12), `research/2026-09-12-ai-roadmap.md`
- continues: `brand-kit-and-teacher-ui.md` (brand and wording), `migration-file-routing.md` (D27 file model), `vendoo-for-teachers-rethink.md` (D1 to D30), `admin-backoffice.md`
- paths: `crates/**`, `web/**`, `apps/landing/**`, `apps/desktop/**`, `docs/**`

## The rules this design is built on

1. **One resource per real product.** Teachouse is the single source of truth; an import, a sync pull or a migration never mints a second copy of a product the catalogue already holds.
2. **One tile per marketplace, and Tes is one marketplace.** No regions anywhere: not in the enum, the schema, the wire, the crosswalk, the form, the board, the import page, the status page or the export.
3. **D1 and D27 stand.** Every request to a no-API marketplace originates on the seller's device; file bytes stay there; the server holds descriptions, digests and sketches.
4. **Teacher wording** per `brand-kit-and-teacher-ui.md`; a disabled control states its reason.
5. **A marketplace verb ships only when captured.** Where a capture is missing the console says so and names it; it never guesses at a live listing.

## 1. One Tes

### What a Tes region is, on the evidence

One author account, one upload flow, one JSON API, one resource list, one price integer per resource with the currency fixed by the account, and one taxonomy tree served per country under a mechanical id prefix (probe 04 lines 10 to 21, probe 10 lines 9 to 19, `endpoints.rs:159-163`).
The three inventories were a modelling choice made before probe 04 and kept for four things that are not catalogues: currency, the age field, the taxonomy prefix and the source and target of a GB-to-NZ duplication.
Two live defects argue for the collapse: the Curriculum ticks write nothing to Tes (`registry/tes.rs:82-95`, the native is read-only) and every non-GB Tes listing is created with `yearGroups: []`, no age data at all (`registry/tes.rs:160-167`).

### The cutover

- `InventoryId::TesGb` is renamed `Tes`; `TesUs` and `TesNz` are deleted. GB keeps idempotency byte tag 0 (`idempotency.rs:23`), so every historical key still derives. DB code `tes_gb` becomes `tes`; wire token `"TesGb"` becomes `"Tes"`; `vocab.ts` regenerates.
- `currency_rule(Tes) = Fixed(Gbp)`, which is what the founder's account observes on every flow. A non-GBP Tes seller is the recorded reopen trigger and gets a currency read on the connection then, not now.
- The age field is `ageRanges`, the only branch ever written; the 30-value `yearGroups` vocabulary stays read-only for import.
- The taxonomy is the GB tree. The country code becomes a field on the Tes connection defaulted to `GB`, so a second market is a data change, never a re-forked enum. The GB-to-NZ crosswalk engine and its tests are deleted.
- Migration: mint `tes`; **refuse** when any org has a product mapped, elected, halted or reconciled on two Tes inventories (a real seller with two live Tes listings for one product is a person to talk to, not a row to drop); rewrite tenant rows in place; delete and re-seed the derived projection tables; retire the three codes.
- Every surface loses the region: `PLATFORMS`, `REGION_TAG`, `TES_CURRICULA`, the Curriculum panel on the form, the three board chips, the import site select, the three export columns, the three status rows, the analytics scope union, the five hardcoded `TesGb` sites in the desktop crate.

### What this deletes

The M1 Tes GB-to-NZ duplication. The console never offered it (`MIGRATE_TARGET` is hardcoded `'Tpt'`); it was a backend capability and its tests. `decisions.md` records the supersession of the 2026-08-25 wedge entries.

## 2. Import

### The shape

Import is one page with two sources and one outcome: resources in the catalogue, with nothing drafted anywhere.

- **Spreadsheet**: unchanged machinery (`import_batch/*`), minus the region tabs.
- **Marketplace**: the device enumerates the shop under the seller's session (`list_own_resources`), reads each listing (`fetch_for_import`), streams the file through the pipeline with a discarding sink (cover kept, bytes discarded, digests and sketches recorded), and posts pages to `POST /v1/devices/{device}/import`. The seller chooses all resources or ticks a selection from a preview list the device posts first.
- The migration divert goes: an import is an import. `IMPORT_IS_A_MIGRATION`, `HANDOFF_LABEL`, `migrationHref` and D8's gate on the migration page are deleted; the TPT copyright declaration is asked once, at connect time, on the Marketplaces page.
- **Auto-label**: every imported resource gets the label of the marketplace it came from (`TPT`, `Tes`), created on first use and never capped against the twenty (the marketplace label is system-owned and sits outside the count).
- **Progress**: one import run is one `import_run` with `job_item`-shaped rows; the existing `/v1/events/stream` ledger carries per-item state to a live bar (`n of m read · k need you · j imported`), and `/imports/[run]` shows the rows. Past runs list newest first.

### Duplicates: the matcher

Scoped to one org; proposes merges only across different marketplaces. Layers and strengths, from the research:

| layer | signal | strength |
|---|---|---|
| L1 | a payload entry's blake3 and byte length equal, the digest held by at most two products in the org, file at least 50 KB | decisive |
| L1b | Jaccard over the set of entry digests at least 0.6 (a Tes multi-file bundle against a TPT single file) | strong |
| L2 | PDF text, normalised, 5-word shingles, MinHash k=128; Jaccard at least 0.9 strong, 0.6 to 0.9 moderate; 64-bit SimHash Hamming at most 3 as the blocker | strong; survives re-export, which is the real case |
| L3 | cover pHash Hamming at most 6 | moderate, never alone |
| L4 | title normalised (case, punctuation, marketplace boilerplate, store name), token Jaccard at least 0.6 or trigram at least 0.5 | weak |
| L5 | grade band overlap, same subject, equal page count, price ratio within 1.5 after conversion, created within 90 days; page count differing by more than 20% is a strong negative | weak |

Fusion is a Fellegi-Sunter log-odds sum, frequency-weighted at L1, L3 and L4 (an unweighted L1 would merge every product carrying the seller's shared `Terms of Use.pdf`).
Three outcomes: **auto-merge** only on L1, or L2 at least 0.9 with equal page count, and no strong negative; **ask the seller** on any two independent moderate signals; **distinct** otherwise.
Henzinger's best fused precision on web pages was 0.79, and one wrong merge in five is unacceptable when the next publish would overwrite a live listing.

Every fingerprint is computed on the device (blake3 entry digests already exist as `observed_hash`; `pdf-extract` for text; `image_hasher` for pHash/dHash; MinHash and SimHash hand-rolled and frozen behind `fingerprint_version`) and uploaded as a fixed-width sketch of about 600 bytes, in the assertion shape `0052_product_file_source.sql` already uses. The server blocks, scores, persists and drives the review; it never sees text or bytes. Page rendering (L2b, for scanned PDFs) is deferred: it means shipping Pdfium in the bundle.

Storage: `product_fingerprint` (versioned sketches, `title_norm` under `pg_trgm`, SimHash band columns) and `duplicate_verdict` on the unordered product pair holding `same | different` with the evidence snapshot, so a "different" answer is never asked twice.

**The review**: between parse and commit, where import already pauses. One card per pair: both listings side by side with marketplace, cover, title, price and grades; one sentence of evidence ("the same file, byte for byte: worksheet-pack.pdf, 2.4 MB"), never a score; three buttons, *Same resource, keep one* / *Different resources* / *Decide later*; on "same", a which-side-wins step per field, per-marketplace values kept on `mapping`; reversible for 30 days; parked pairs never block the import.

### A capture this depends on

TPT's own-file download is uncaptured (`uncaptured_source(Tpt) = "tpt.download_resource_bundle"`). Until the founder supervises that capture, a TPT import carries title, description, price, grades, cover and page count but no file digest or text sketch, so a TPT-to-Tes duplicate is found by L3, L4 and L5 alone and is always asked, never auto-merged.

## 3. Marketplace lifecycle

| verb | TPT | Tes |
|---|---|---|
| enumerate catalogue | wired | wired |
| read one listing | wired | wired |
| download own file | **uncaptured** | wired |
| create draft / publish live | wired | wired |
| edit published | wired | **uncaptured** (`tes.edit_published`) |
| unpublish | n/a | **uncaptured** (`tes.unpublish`) |
| delete | wired | wired |

The console shows every verb; the two uncaptured ones render disabled with the sentence "Editing a live Tes listing is not built yet" and the capture is named as owed by the founder. Nothing else in this design waits on them.

## 4. Migrations

- **Copy** is `Disposition::Sync`; **Move** is `Disposition::Migrate` (creates on the target, then removes from the source once bound). Both exist; the console gains the choice and a per-resource preview (*will create* / *already there* / *blocked: reason*) before confirm.
- Source and target are any two authorable marketplaces; TPT as a source needs the file-download capture above, so until then a TPT-to-Tes move is offered disabled with the reason.
- The selection is all resources or a tick list, or a collection (section 7).

### Plans, capabilities and the one-off purchase

The evidence (`research/2026-09-12-pricing-and-tiers.md`): the full-crosslister band is $25 to $45 a month; the teacher persona's habitual software spend is $5 to $18 (TPT Premium $59.95 a year, Boom $6.99 a month, Canva Pro $18); the typical active TPT seller earns about $27 a month; every one of eight comparables bundles import inside the subscription; AI has converged on bundled-with-a-fair-use-cap; Vendoo abandoned per-listing metering and the surviving axis is feature and automation depth.

`org.plan` is a closed set: `free`, `subscriber`, `migration_only`, plus `studio` reserved and unsold until at least a fifth of subscribers exceed 300 resources or hit the migration cap twice in a quarter.
Prices stay the founder's: Subscription $24 a month or $240 a year with a 14-day carded trial; the Catalogue Import ladder $47 up to 20, $77 up to 50, $127 up to 100, $247 up to 250, with two rungs added because the measured dual-lister holds about 764 TPT listings: $397 up to 500 and a conversation above that; Founding 100 unchanged.
The ladder counts resources committed to the catalogue after duplicate merges, and says so.
Import is included for subscribers: charging at the moment of activation taxes the one step that makes the product useful, and the 20-a-month migration cap is what keeps a one-month subscription from undercutting the ladder.

Every feature is gated by a `Capabilities` struct computed once per request from `org.plan` and the org's overrides, carried on `OrgContext`, mirrored into the device entitlement token, and rendered by the console's disabled-with-reason pattern; the same struct is what the pricing page and the Account page read, so the three can never disagree.

| capability | free | subscriber | migration_only |
|---|---|---|---|
| resources | 20 | 400 | the rung bought |
| marketplaces connected | 1 | all | all |
| import: spreadsheet, marketplace, duplicate review | spreadsheet only | all | all, within the rung |
| cross-list and publish | manual, one marketplace | unlimited | one pass over the imported set |
| edit and delete on marketplaces | yes | yes | 30 days after purchase |
| migrations (copy or move) per month | 0 | 20 resources | the rung, once |
| scheduling | no | yes | no |
| sync pulls and auto-publish rules | no | every 6 hours | no |
| templates / collections / labels | 1 / 0 / 5 | 20 / 20 / 20 | 1 / 0 / auto-labels |
| analytics | no | yes | no |
| export | yes | yes | yes |
| devices | 1 | 2 | 1 |
| AI auto-fill (coming soon) | no | bundled, 200 a month | no |
| support | guides | email, two business days | email for 30 days |

Export is never gated: a seller who cannot get a catalogue out will not put one in.
A `migration_only` org reaches Import, Migrations, Export and Account, which is what the product called Catalogue Import sells; every other section renders its reason and a link to subscribe.
Paddle gains one price per rung and the webhook maps price ids to plans; a purchase writes an `entitlement_grant` row (plan, rung, granted_by, expires_at), and the plan an org holds is the strongest unexpired grant.

Three risks were put to the founder: the subscription's real market is roughly the top decile of the dual-lister pool, so `migration_only` must be a first-class product; the ongoing Founding discount is now capped at three years by the founder's decision; and the Tes royalty bands (60/70/80 percent on a rolling twelve months) are the sharpest ROI argument available and belong in the copy.

### The backoffice can set a plan

An operator can grant or revoke any plan or rung on any organisation, with a reason, an optional expiry and an audit row (`entitlement_grant.granted_by` is the operator, never null for a manual grant). The org's Account page shows a manual grant as "Set by Teachouse until <date>".

## 5. Automations

### Scheduling (was Marketplace Sharing)

A `schedule` names a selection (a label or a collection), the marketplaces, an intent (draft or live), a time, and optionally a rule: **on update, republish** (when a resource in the selection changes, its live listings are revised on the next tick).
`tam-worker` gains a scheduler tick beside its maintenance pass: every minute it materialises due schedules into sync requests with the resolved member ids frozen in, idempotent on `(schedule, resource, marketplace, tick)`.
Device-bound marketplaces run the requests at the device's next check-in, which the page says.
The page lists schedules, their next run, and the runs they produced.

### Sync

Per marketplace, one setting: **pull new resources** with a cadence (every 6 hours, daily, weekly) and a rule per marketplace: **auto-publish new pulls to** a set of other marketplaces, using the seller's template for the marketplace-specific fields.
The device enumerates the shop on the cadence (a `catalogue_check` request the scheduler mints), the server diffs against `mapping`, unmapped listings enter import with the duplicate review, and each new resource is labelled with its source and, where the rule says so, published onward.
The page shows every resource that is live on more than one marketplace, and an activity log in seller words: *"Fractions Pack" pulled from TPT 2 hours ago · Maths, Year 5 · published to Tes*.
Existing sync-request coverage and the ledger stream carry the state.

## 6. Status

One row per marketplace (TPT, Tes, Etsy coming soon), with the halt state and the last successful device verification time.
The top-strip "Tes checking / Tpt checking" chips are removed: `checking` is linked-but-unverified, which is nothing to report, and the strip now shows a dot only for `unstable` and `disconnected`.

## 7. Templates

A template is a name, a short description, a scope (generic or one marketplace), and any subset of the whole new-resource form: title pattern, description, price fields, grades, subjects, tags, formats, standards, details, copyright, licence, tax code, localization.
It is stored as a partial `DraftInput` exactly as today (`resource_templates.rs`), widened from five fields to the full form; unanswered fields stay unanswered.
Apply: on the new-resource form ("Start from a template"), on a selection ("Apply template" fills only empty fields unless the seller ticks overwrite), and as the marketplace-specific source the sync rule uses.
The "Marketplace words" tab (projection overrides) stays as it is.

## 8. Collections

Shipped, as a plain many-to-many beside labels: `collection(org, id, name 1..80, description ..1000)` and `collection_member(org, collection, product, position)`, RLS copied from `product_label`.

Why membership in several collections is safe: a membership row is a reference, never a copy; the uniqueness rule is about identity and about `mapping_one_per_inventory`, and neither table creates a product or a mapping.
What would break the rule is refused outright: "duplicate into collection", and any per-marketplace state on a collection (overrides live on `mapping` and in templates).

Bulk verbs, each resolving the distinct union of members, previewing per resource, and enqueueing idempotently on `(org, product, marketplace, intent)` with the resolved ids frozen in: **publish to a marketplace**, **apply template / add labels**, **export to the spreadsheet**. Delist waits until the preview has earned trust.
Manual only: no rules, no nesting; the saved label filter is the smart collection.
Deleting a collection removes membership and deactivates nothing.
Collections are not bundles; a TPT or Tes bundle is a product and a separate change.

## 9. Backoffice

Extends the operator surface that exists (`admin.rs`, `/admin/*`, `tam_backoffice` role, impersonation with a separated audit trail):

- **Users**: list with organisation, plan, last sign-in and active sessions (better-auth's session table), sign-out-everywhere, and the plan grant described in section 4 with its audit row.
- **Guides**: a `guide` table (slug, title, body as Markdown, status draft/published, updated_by) with an editor that supports images through the existing upload route; rendered by `pulldown-cmark` into `/guides`, which replaces the placeholder.
- **Marketplace requests, failures, health, import drain, dead letters**: as today.
- **Product analytics**: the operational charter forbids a collector on day one and free text in any payload. A self-hosted PostHog on thunderstorm with an allow-listed event set and no autocapture would satisfy the second rule and breach the first; it is a founder decision and is not built by default.

## 10. Themes

A `[data-theme="dark"]` block in `tokens.css` and `site.css` carrying the kit's colours re-grounded on Indigo (`--ground #141C3A`, `--surface #1E2A5A`, `--text #F8FAF8`, `--muted #B4BCD0`, teal and the accents unchanged, soft states darkened); `color-scheme: light dark`; a three-way switch (system / light / dark) in Account preferences persisted per user and mirrored in `localStorage` for first paint.
The token test measures every contrast pair on both grounds.
Desktop: the Tauri window `theme` follows the console's choice through the existing command channel; Android's `values-night` already follows the system.

## 11. Landing imagery

Direction chosen from the three the art-direction pass produced: **the Resource Atelier**. Resource covers as valued creative work, one catalogue card, quiet teal routing lines to the marketplace marks, a real console screenshot as the proof.

- Hero: an SVG composition; "Your catalogue" card with the house mark, three self-drawn resource covers (fractions circles, ruled reading lines, checklist squares), four teal lines to TPT, Tes, Classful and Teach Simple tiles, "+ more" in Inter. No person, no laptop, no script font.
- The challenge: one finished cover, three staggered listing sheets repeating the same title with different marks, a peach bracket "Same resource. Repeated admin."
- The solution: a real screenshot of the console's Resources board seeded with the same three resources, in an indigo-outlined frame under the house mark.
- Phone: separate compact SVGs; display text outlined from the local Poppins.

## 12. AI, and what "coming soon" promises

The charter allows models in two places, listing-copy generation and selector rediscovery, and forbids agent-driven sync; every AI feature is therefore a proposal the seller approves, never a write.
`research/2026-09-12-ai-roadmap.md` is the full analysis; the shape it fixes:

**AI auto-fill** is the first feature and ships as "coming soon" on the pricing page and beside Files on the form until it is built.
A seller chooses a PDF; the device extracts a bounded fact sheet (page count from the page tree, explicit grade phrases with page references, heading candidates, standards codes printed in the file, section boundaries) and the PDF never leaves the device.
Deterministic facts pre-fill with a verification mark; explicit evidence pre-fills as "Suggested" once a field's measured precision on the founder's own evaluation set passes 98 percent; everything else is a suggestion with its reason ("Grade 3, printed on page 1") or is left blank.
Closed vocabularies are hit by retrieval and ranking over the versioned taxonomy, never by a model inventing a label; a description is composed by a hosted model from the approved fact sheet alone, with numbers bound to fact ids, and a Rust validator blocks any numeric claim the fact sheet cannot support, any invalid vocabulary id and any over-length field, exactly as D-M4 required.
Price, tax code, copyright and licence are never inferred.
Cost is about $0.003 a resource on a small hosted model; no GPU and no on-device model in the first release.

After auto-fill, in the order value divided by build cost: listing health audit (rules, free), per-marketplace title and description rewriting (subscriber), duplicate-review explanations in plain sentences (free), tag and category suggestions (subscriber), British and American description adaptation (subscriber), preview page recommendations, analytics narratives, pricing ranges from the seller's own sales, a standards evidence finder, bundle candidates, help search over the guides, and thumbnail layouts.
Not built: a catalogue chatbot, server-side OCR or PDF upload, AI inside scheduler ticks, generated cover art as the headline, revenue forecasts, and anything that asserts rights or curriculum alignment.

Packaging: bundled in the subscription with a fair-use cap of 200 fills a month and a $5 add-on for 100 more; deterministic checks and failed generations never count.
The "coming soon" copy promises "fills the form from your file, for you to check" and promises no accuracy figure, no marketplace write and no date.

## 13. Every surface is desktop and phone

The console reaches Windows and Android through the webview, so every new page ships in both layouts before it ships at all: the tile grids wrap, the review cards and the migration preview stack, the activity log becomes a list, the scheduler and sync settings become one column, and the backoffice tables become cards under 720 pixels.
Each phase's verification includes a 1280 and a 390 capture of every new page, as the 2026-09-11 wave did.

## Phasing

| phase | delivers | unblocks |
|---|---|---|
| 0 | One Tes (enum, schema, migration, every surface), status chips, dark mode, landing imagery, "coming soon" AI placements | everything below is written against one Tes |
| 1 | Plans and capabilities: `org.plan`, `entitlement_grant`, the `Capabilities` struct, Paddle prices per rung, the backoffice grant, the pricing page and Account on the same table | every later feature ships gated |
| 2 | Import: catalogue-only, selection preview, auto-label, live progress, fingerprints on the device, the duplicate review | sync pulls reuse it |
| 3 | Migrations: copy/move with preview, the monthly cap, the `migration_only` account end to end | scheduling reuses the request shape |
| 4 | Scheduling and Sync: the scheduler tick, schedules, cadenced pulls, per-marketplace rules, the activity log | |
| 5 | Templates v2 and Collections with the three verbs | |
| 6 | Backoffice: users and sessions, guides | |
| 7 | AI auto-fill: the device fact sheet, the evaluation set, the validator, the proposal UI | the ranked list after it |

Founder-supervised captures owed, none blocking phase 0 to 2: TPT own-file download, Tes edit-published, Tes unpublish, TPT preview slot (from 2026-09-11).
