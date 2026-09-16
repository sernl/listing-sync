# The seller dashboard

What a teacher-seller sees: one catalogue of their own items, the standing of each on every marketplace they sell through, and the actions those standings admit.

- date: 2026-09-03
- status: built; the screens below are what the console renders today, and the gap list at the foot is what the API stream still owes them
- amended: 2026-09-16 — machine sign-ins moved to Preferences; Files moved under Resources; import and cleanup layouts verified at desktop and phone widths
- naming: the seller-facing noun is item, the board is Inventory, and the console's screens carry Vendoo's own words where Vendoo has one (founder question Q1, and `docs/research/rethink/vendoo-console-cross-reference.md` under "Rename map")
- decisions it implements: D1 (the two-branch automation rule, shown to the seller as the device-driven reality rather than hidden), D3 (a phone starts work for a no-API marketplace but never schedules it), D14 and D30 (where a marketplace login lives, and the words for it)
- sources: `docs/research/rethink/vendoo-console-cross-reference.md`, which is the map this console was aligned to; `docs/research/rethink/vendoo-workflows-and-ux.md` sections 1, 3, 9, 10 and 11; `docs/research/rethink/vendoo-architecture-and-market.md` sections 5 and 8; `docs/notes/design/creation-flow.md`; `docs/notes/design/device-registry.md`

## What this adopts from Vendoo, and what it drops

One item that fans out to many marketplaces, and every screen a view over that fan-out, is the whole product shape and it transfers intact.
The per-marketplace glyph strip on an inventory row transfers, with its state set widened, and it is the single densest thing on the screen.
Advanced filtering, search over the title, and the split editor with a destination rail transfer.
The connections page keeps one row per marketplace with its standing and connection controls; installation management lives in Settings rather than below those rows.
Vendoo's 2026 sidebar grouping transfers as far as it has counterparts: a Crosslist group, an Automations group, and a Help group at the foot for the destinations Vendoo keeps in its profile and help menus.

Three things Vendoo does are deliberately not built.
Sold is not an inventory column, because selling never removes a digital item from sale.
Delist-and-relist is not offered as a refresh, because it discards the reviews, ratings, sales history and URL that are the seller's accumulated asset; the reconciliation verb is revise-in-place.
Auto-delist does not exist at all, and neither does the sold-but-still-listed warning that only means something for one physical unit.

The two failures that generate the most anger in Vendoo's own reviews are a missed sale detection and a silent connection drop, and both are observability problems.
That is the argument for stating the device-driven reality on the dashboard rather than burying it in settings.

## The screens

`/` is the workspace overview: the figures the catalogue supports, the attention list, the recent strip, and a band saying whether the seller's own machine is on and which marketplaces need a sign-in there.

`/resources` is the board, and it is the centre of the product.
One row per item, carrying its title, price, the per-marketplace chip strip, its captured views and sales, and when it was last touched.
Search over the title, filter by marketplace, by standing and by label, and five bulk verbs over the selected rows.
Four of the five run: cross-list, mark-as-listed, labels, and delete.
One is rendered disabled with the reason on the control and again under the table: bulk edit, whose gap is a screen choosing which fields change across a selection rather than an endpoint, because the per-item edit is already served.
The label filter narrows the catalogue's own page query rather than the rows already loaded, so paging a filtered catalogue is the same walk as paging the whole one.
Single and bulk resource deletion select no marketplace automatically.
Removing a marketplace listing requires selecting it explicitly; leaving an existing listing unchanged requires a separate acknowledgment.
If a bulk deletion cannot confirm a result, it stops, refreshes Resources, reports the confirmed count and leaves the dialog open without a retry action.
There is no bulk delist-and-relist under any name, and a test refuses one.

`/resources/{id}` is one item: its canonical fields, one row per marketplace with that marketplace's standing and the action it admits, the runs this listing has started, and the destructive actions behind their own dialogs.
The run timeline is not rebuilt here; a run links to `/sync/{job}`, which already renders items, gates and per-item events off the ledger.

`/resources/files` lists files with links to the resources that use them.
Search covers filenames, digests and linked resource titles; availability, resource-link and machine filters apply before pagination.
The native application's local-file list is separate from the shared index, so a remote filter cannot hide a file kept on this machine.

`/marketplaces` shows each marketplace's connection standing and where its session is held.
Machine sign-ins are managed in `/settings`, under Preferences, immediately after Browser sign-ins.
The former `/marketplaces#machines` destination redirects there; marketplace connection controls remain on Marketplaces.

The dashboard band answers three questions in the order a seller asks them.
Is anything scheduled running at all, which is true only where a machine is checking in and holds at least one marketplace login.
Where does each marketplace's login live, which the transport branch decides: for TPT and Tes the device registry is the only place one can be, because no server has ever held one, and for Etsy the connection record is, because no device does.
Which machines exist, when each was last heard from, and whether a machine signed out still has a wipe outstanding.
A device checks in hourly, so the band calls a machine current within two cadences and quiet after that: one missed check-in is a laptop that slept, and two is the first silence that carries information.

## The small viewport

The console is one SvelteKit app on the web, the Windows desktop and Android, so the Android instruction is this app below 620px rather than a second route tree.
At phone widths the bottom bar offers Import, Catalogue, New, Automate and Markets.
Import and cleanup controls remain available on small screens.
History rows separate selection, marketplace labels, status and actions rather than squeezing them into one line; desktop rows keep these controls aligned without overlap.

## The states, and the words for them

The state set is derived from what the API serves and from nothing else.
Vendoo's three glyph states widen to five in the research; the five below are those, plus the three this tree's own data model distinguishes and a seller acts on differently.

Not listed: no mapping for that marketplace, or a mapping whose binding is `unbound` or `severed`.
The item exists here and the marketplace has never seen it.

Draft: bound, lifecycle `draft`.
The marketplace holds the listing and is not showing it to buyers, which is a real state on both TPT and Tes and a different action from not listed.

Listed: bound, lifecycle `live`.

In flight: a job item for this mapping is `queued`, `leased`, `running` or `verifying`, or the binding state is one this client does not recognise, which means a create is out and its outcome is unknown.

Blocked: a job item is `blocked`, and the gate names why.
The words come from `gateLabel` in `web/src/lib/gates.ts`, which is already an exhaustive map over the generated `BlockedGate` union, so a gate added in Rust fails the web lane rather than reaching a seller as an identifier.

Needs sign-in: the gate is `ReauthRequired` or `awaiting_seller_signin`, or the marketplace's connection status is `disconnected`.
For a marketplace on the seller-device branch this reads as sign in on your own device, because no session for one has ever been on our servers.

Stranded: a job item is `parked_live` or `parked_cold`.
A write was in flight when the gate closed and it is being held rather than retried blind, which is a different fact from blocked before anything was attempted, and the seller needs to know a half-finished write exists.

Failed: a job item settled with outcome `failed`.

Paused overlays any of the above rather than replacing it: `GET /v1/status` reports the inventory halted, and the reason it carries is displayed verbatim.

Published but out of date, which the research names as the state the whole file-version entity exists to drive, is deliberately absent.
Nothing in the API records that a payload changed after a publish, so rendering it would be an invention; it is gap G5 below.

## The actions each state admits

Not listed, with a mapping: send it, which enqueues through `POST /v1/jobs` with intent `draft` or `live`.
Not listed, with no mapping: cross-list here, which adds the marketplace through `POST /v1/products/{product}/mappings` and then opens the send.
The add writes the catalogue and contacts nobody, and the intent stays the seller's choice on the send rather than one the control makes for them.

Draft: publish here, the same endpoint with intent `live`.

Listed: revise, which is an edit through `PATCH /v1/products/{id}` followed by a send; and remove from this marketplace, through `DELETE /v1/products/{id}` with `remove_from`.
Open the live listing, which the chip itself carries: `listing_url` on the mapping is the page, derived by the server from the identifier the binding already holds.
A marketplace whose page shape the server has not observed serves null there, and the chip falls back to opening the item rather than guessing a URL.

In flight: watch the run, which links to `/sync/{job}`.

Blocked: see why, which states the gate in the seller's words and links to `/reconciliation` for a reconciliation or election gate and to the run for every other.

Needs sign-in: sign in, which links to `/marketplaces`, where the marketplace's own row and the machine that would hold the session are on one screen.

Stranded: the same sign-in action, with the held write named so the seller knows a retry is waiting rather than lost.

Failed: see the failure, which links to the run, where the failure code and detail already render.

Paused: open status.

## Which endpoint feeds what

The inventory reads `GET /v1/products` through every page, `GET /v1/mappings`, `GET /v1/analytics/summary` for the two captured columns, `GET /v1/connections`, `GET /v1/status`, and the newest few runs through `GET /v1/jobs` and `GET /v1/jobs/{job}` and `GET /v1/jobs/{job}/items`.
The last of those is what supplies the in-flight, blocked, stranded and failed states, and it is bounded to the newest runs rather than paginated, exactly as the dashboard's activity panel already is.

The item screen reads `GET /v1/products/{id}`, `GET /v1/mappings` filtered to the product, `GET /v1/vocabulary/{inventory}` per marketplace, `GET /v1/connections` and `GET /v1/status`, and it writes through `POST /v1/jobs`, `PATCH /v1/products/{id}` and `DELETE /v1/products/{id}`.

The device band reads `GET /v1/devices` and `GET /v1/connections`.
The Files view reads `GET /v1/library`; its result and total come from the same database snapshot.
Which branch a marketplace is on is not served; it is a total map over the generated `Marketplace` union mirroring `tam_types`, in the same style as `MARKETPLACE_OF` and `PLATFORMS`, so a marketplace added in Rust stops the web lane rather than rendering under the wrong branch.

## Endpoint gaps

G1. Adding a marketplace to an existing product. Served.
`POST /{version}/products/{product}/mappings` takes `{ inventory }` and answers the whole `MappingHead`, which is what the chip strip renders, rather than the `{ mapping, inventory }` pair this gap first asked for.
The mapping it mints is the create's own unbound mapping, so the send that follows is the ordinary job path; an inventory the product already carries is refused with `mapping_already_exists`.
The item screen's cross-list control and the bulk dialog's unmapped targets both run through it.

G2. A mapping's own work.
Job items are reachable only through the job that holds them, so answering "what is happening to this listing on this marketplace" costs a read of every recent run and is bounded rather than complete.
Shape needed: `latest_item` on `MappingHead`, carrying `{ job, item, state, outcome, blocked_on, settled_at }` or null.

G3. The live listing's URL. Served.
`listing_url: string | null` on `MappingHead`, derived rather than stored: `listing_url` in `crates/tam-domain/src/registry/listing_url.rs` renders the page from the identifier the binding holds, and no request is made to produce it.
It answers only while the binding is `bound`, because a severed mapping still carries remote-id columns and the console reads severed as not listed, so serving one would link a seller to a listing this tree no longer claims.
Tes normalises both stored shapes to `https://www.tes.com/teaching-resource/-{id}`: the create path stores the canonical `/api/v2/resources/{id}`, which is an API route a seller opening it would read as JSON, and the import path stores a page.
TPT renders `/Product/listing-{id}` under a constant slug, because `tam-marketplace-tpt`'s `classify` module establishes that any slug serves the correct product; a title-derived slug was rejected because it would put a join onto `product` on every mapping read to decorate a path the marketplace discards.
Etsy answers null until an adapter exists, and a stored value matching no known shape answers null rather than a guess.

G4. Elections have no client binding.
`GET /{version}/elections/items` is served and its `DecisionView` carries `product` and `inventory`, which is exactly the per-item per-marketplace "waiting on your answer" signal, but `web/src/lib/api.ts` has no method for it.
This is a client gap rather than a server one, and `api.ts` belongs to another stream.

G5. File versions.
The research names file versions as the central new entity, driving the out-of-date state and the update-everywhere action, and no endpoint records that a payload changed after a publish.
Shape needed: a version on the product and the version last published on each mapping, so the two can be compared.

G6. Custom labels. Served.
`GET` and `PUT /{version}/products/{product}/labels` hold one item's labels, `GET /{version}/labels` is the organisation's whole vocabulary, and `label` on `GET /{version}/products` narrows the page.
Labels are named rather than identified: a seller types a word, and the same word is the same label across the catalogue, which is what makes filtering by it mean anything.
`label_one_per_name` folds case, so "autumn term" after "Autumn term" is the label they already have rather than a rival spelling that would split a filter in half.
A `PUT` replaces the whole set rather than merging, because a merge would leave removing the last label with no spelling; the bulk dialog therefore reads each item's set and adds to it, so relabelling twelve items does not flatten them all to one set.
A label nothing carries any more is deleted with the write that abandons it, so the filter never offers a word no item can be found by, and deleting an item clears its attachments in the same transaction for the same reason.
The sweep is narrowed to the labels that write abandoned rather than run across the organisation, because an organisation-wide sweep can strip a label another request attached between that request's insert and this delete.
A residual race remains and is accepted: two writes abandoning and re-attaching one label at the same instant can leave it deleted with a carrier, whose attachment the foreign key then cascades away.
The cost is a label the seller retypes; the cost of preventing it is every label write serialising behind every other.
The colour is derived from the name rather than chosen, from the closed set in migration 0046: seller-chosen colour would need a label-management surface this gap does not include, and a derived colour still gives the visual scanning coloured labels exist for while staying stable everywhere the label appears.

G7. Transport class is not served.
`TransportClass` is generated into the client's vocabulary but appears on no view, so the console mirrors the Rust mapping rather than reading it.
Serving it on `VocabularyView` would remove the mirror.

G8. Removing a listing from one marketplace without deleting the item.
`DELETE /{version}/products/{id}` with `remove_from` deletes the product and removes it from the marketplaces named, and `ItemOperation::Remove` is enqueued only by the migrate drain, so a teacher retiring an item from Tes while keeping it on TPT has no path.
The research asks for exactly this verb and asks that it not be called a refresh.
Shape needed: a per-mapping removal, either its own endpoint or an intent on `POST /{version}/jobs`.

G9. Linking a marketplace connection.
`GET /{version}/connections` and `POST /{version}/connections/{connection}/revoke` are the whole surface (`crates/tam-api/src/lib.rs:222`, `crates/tam-api/src/openapi.rs:204` and `:209`); no endpoint establishes a connection, so the Marketplaces screen renders Link and Re-link disabled with that stated.
Only the official-API branch needs it: a marketplace on the seller-device branch is signed into on the machine itself, which is the only place its session can exist.
Shape needed: whatever the broker's link handshake requires, reached from `POST /{version}/connections` taking `{ marketplace }`.

G10. A last-sync time per marketplace.
The cross-reference asks the Marketplaces screen to show when each marketplace was last synced, against Vendoo's own "Last sync" label.
`ConnectionView.updated_at` and `DeviceSessionView.last_used_at` are the nearest served facts and neither is a sync time, so the screen shows neither rather than mislabelling one.
Shape needed: the time of the last completed run per marketplace, on the connection view and on the device session.

G11. A thumbnail on the product list.
`ProductHead` carries id, title, price and the two timestamps, and no image; the cover and previews are reachable only through the single-product endpoint.
The phone card therefore carries four of the five fields the cross-reference names for an inventory card, and renders no placeholder tile for an image that is not served.
Shape needed: a cover image URL on `ProductHead`.

G12. Binding a listing the console did not create. Served.
`POST /{version}/mappings/{mapping}/bind` takes `{ listing_url }` and answers the bound `MappingHead`; the item screen offers it per marketplace and the board offers it over a selection.
The URL is parsed by host and path rather than by its trailing digits, in `parse_listing_url` beside G3's renderer, and a round-trip test holds the two directions together.
Host and path are both checked because every marketplace's page ends in digits: the adapters' own parsers read a `Location` header from a redirect they had just caused, where the marketplace was never in doubt, and a pasted URL has no such provenance.
A Tes paste is stored as the canonical `/api/v2/resources/{id}` identity rather than the page that was pasted, because `DraftId::canonical_url` requires one spelling per resource: a second one makes a later write report a divergent landing against the mapping it just wrote, and defeats `mapping_one_bound_url`.
The binding starts `Verification::Stale` at the bind instant, which is the state migration 0004 gives a bound mapping nothing has read back, and the engine's read-back is what confirms it; nothing on this path contacts a marketplace, so a seller can attach a listing that is gone or is not theirs and the read-back is what catches it.
Four refusals are named rather than collapsed: a link for another marketplace, a link that is not a listing page, a mapping that already binds one or has a create out, and a listing another of the seller's own items already claims.
Re-binding a `severed` mapping is deliberately not offered here: it carries a sever generation and the content keys hanging off it, which is reconciliation's path rather than a paste.

G13. Seller-chosen label colour.
G6 serves labels with a colour derived from the name, from the closed set in migration 0046, which gives the visual scanning coloured labels exist for and is stable everywhere a label appears.
What it does not give is the seller's own choice, and choosing one needs more than a colour field: a label is shared across the catalogue, so recolouring means a surface that renames, recolours and deletes a label everywhere at once, and deleting one has to say what happens to the items carrying it.
Shape needed: a label-management screen and the endpoints under it, at which point the colour stops being derived and becomes a stored choice.
