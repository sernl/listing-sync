# The seller dashboard

What a teacher-seller sees: one catalogue of their own items, the standing of each on every marketplace they sell through, and the actions those standings admit.

- date: 2026-09-03
- status: built; the screens below are what the console renders today, and the gap list at the foot is what the API stream still owes them
- naming: the seller-facing noun is item, the board is Inventory, and the console's screens carry Vendoo's own words where Vendoo has one (founder question Q1, and `docs/research/rethink/vendoo-console-cross-reference.md` under "Rename map")
- decisions it implements: D1 (the two-branch automation rule, shown to the seller as the device-driven reality rather than hidden), D3 (a phone starts work for a no-API marketplace but never schedules it), D14 and D30 (where a marketplace login lives, and the words for it)
- sources: `docs/research/rethink/vendoo-console-cross-reference.md`, which is the map this console was aligned to; `docs/research/rethink/vendoo-workflows-and-ux.md` sections 1, 3, 9, 10 and 11; `docs/research/rethink/vendoo-architecture-and-market.md` sections 5 and 8; `docs/notes/design/creation-flow.md`; `docs/notes/design/device-registry.md`

## What this adopts from Vendoo, and what it drops

One item that fans out to many marketplaces, and every screen a view over that fan-out, is the whole product shape and it transfers intact.
The per-marketplace glyph strip on an inventory row transfers, with its state set widened, and it is the single densest thing on the screen.
Advanced filtering, search over the title, and the split editor with a destination rail transfer.
The connections page shape transfers: one row per marketplace carrying its state and the control that changes it, merged here with the device registry so one screen answers whether a marketplace can be written to right now.
Vendoo's 2026 sidebar grouping transfers as far as it has counterparts: a Crosslist group, an Automations group, and a Help group at the foot for the destinations Vendoo keeps in its profile and help menus.

Three things Vendoo does are deliberately not built.
Sold is not an inventory column, because selling never removes a digital item from sale.
Delist-and-relist is not offered as a refresh, because it discards the reviews, ratings, sales history and URL that are the seller's accumulated asset; the reconciliation verb is revise-in-place.
Auto-delist does not exist at all, and neither does the sold-but-still-listed warning that only means something for one physical unit.

The two failures that generate the most anger in Vendoo's own reviews are a missed sale detection and a silent connection drop, and both are observability problems.
That is the argument for stating the device-driven reality on the dashboard rather than burying it in settings.

## The screens

`/` is the workspace overview: the figures the catalogue supports, the attention list, the recent strip, and a band saying whether the seller's own machine is on and which marketplaces need a sign-in there.

`/inventory` is the board, and it is the centre of the product.
One row per item, carrying its title, price, the per-marketplace chip strip, its captured views and sales, and when it was last touched.
Search over the title, filter by marketplace and by standing, and five bulk verbs over the selected rows.
Two of the five run: cross-list, and delete.
Three are rendered disabled with the reason on the control and again under the table: bulk edit, which needs a screen rather than an endpoint; bulk labels, which needs G6; and bulk mark-as-listed, which needs G3 and a bind verb on top of it.
Bulk delete asks which way the listings already on a marketplace should go and defaults to neither, because removing everywhere fires one write per listing from one click and leaving them standing abandons listings nothing here tracks; that is the seller's decision at the moment they take it, not one the console makes for them.
There is no bulk delist-and-relist under any name, and a test refuses one.

`/inventory/{id}` is one item: its canonical fields, one row per marketplace with that marketplace's standing and the action it admits, the runs this listing has started, and the destructive actions behind their own dialogs.
The run timeline is not rebuilt here; a run links to `/sync/{job}`, which already renders items, gates and per-item events off the ledger.

`/marketplaces` is one row per marketplace, carrying the branch its automation runs on, the sign-in or connection standing that branch decides, the machine holding a device-branch login, and the machines list itself below the rows.
It is the one place a machine is signed out, and the dashboard band and the item screen link into it rather than duplicating it.
Browser sign-ins are sign-ins to us rather than to any marketplace and sit on `/settings` beside the account.

The dashboard band answers three questions in the order a seller asks them.
Is anything scheduled running at all, which is true only where a machine is checking in and holds at least one marketplace login.
Where does each marketplace's login live, which the transport branch decides: for TPT and Tes the device registry is the only place one can be, because no server has ever held one, and for Etsy the connection record is, because no device does.
Which machines exist, when each was last heard from, and whether a machine signed out still has a wipe outstanding.
A device checks in hourly, so the band calls a machine current within two cadences and quiet after that: one missed check-in is a laptop that slept, and two is the first silence that carries information.

## The small viewport

The console is one SvelteKit app on the web, the Windows desktop and Android, so the Android instruction is this app below 620px rather than a second route tree.
The sidebar is replaced there by a bottom bar of five tabs, Inventory, Marketplaces, Sync, Analytics and Account; that set is derived rather than sourced, because no public Vendoo page names its app's tab bar, and it will be corrected against the founder's own view of the app when they supply it.
The inventory board becomes one block per item carrying its title, price, marketplace chip strip and the line saying what it needs, and importing and the many-item bulk verbs are hidden rather than disabled, because Vendoo's own availability article puts them on the desktop only.
The desktop layout above the breakpoint is unchanged.

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
Not listed, with no mapping: cross-list here, which needs gap G1 and is disabled with that stated until it lands.

Draft: publish here, the same endpoint with intent `live`.

Listed: revise, which is an edit through `PATCH /v1/products/{id}` followed by a send; and remove from this marketplace, through `DELETE /v1/products/{id}` with `remove_from`.
Open the live listing is not offered, because no endpoint serves the listing's URL; that is gap G3.

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
Which branch a marketplace is on is not served; it is a total map over the generated `Marketplace` union mirroring `tam_types`, in the same style as `MARKETPLACE_OF` and `PLATFORMS`, so a marketplace added in Rust stops the web lane rather than rendering under the wrong branch.

## Endpoint gaps

G1. Adding a marketplace to an existing product.
Mappings are chosen when the draft is created and there is no way to add one afterwards, so the central Vendoo action — cross-list this to somewhere it is not yet — cannot be performed at all.
Shape needed: `POST /{version}/products/{product}/mappings` taking `{ inventory }` and answering `{ mapping, inventory }`, refusing an inventory the product already carries.

G2. A mapping's own work.
Job items are reachable only through the job that holds them, so answering "what is happening to this listing on this marketplace" costs a read of every recent run and is bounded rather than complete.
Shape needed: `latest_item` on `MappingHead`, carrying `{ job, item, state, outcome, blocked_on, settled_at }` or null.

G3. The live listing's URL.
Vendoo's glyph strip clicks through to the listing on the marketplace, which is the single most-used affordance on its inventory board, and nothing here serves the URL.
Shape needed: `listing_url: string | null` on `MappingHead`.

G4. Elections have no client binding.
`GET /{version}/elections/items` is served and its `DecisionView` carries `product` and `inventory`, which is exactly the per-item per-marketplace "waiting on your answer" signal, but `web/src/lib/api.ts` has no method for it.
This is a client gap rather than a server one, and `api.ts` belongs to another stream.

G5. File versions.
The research names file versions as the central new entity, driving the out-of-date state and the update-everywhere action, and no endpoint records that a payload changed after a publish.
Shape needed: a version on the product and the version last published on each mapping, so the two can be compared.

G6. Custom labels.
Coloured seller-defined labels transfer from Vendoo as-is and are the dimension a teacher would filter units, seasons and sale participation by, and no endpoint holds them.
Filtering is therefore title search, marketplace and standing only.

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

G12. Binding a listing the console did not create.
Vendoo's "Mark as Listed" takes a pasted listing URL and binds an existing marketplace listing to the item, which is how an imported or pre-existing catalogue is adopted; nothing here binds a mapping to a listing the engine did not create, so the verb is disabled on the item screen and in bulk.
It sits on top of G3 rather than beside it: G3 serves the URL a bound mapping already has, and this one accepts a URL for a mapping that has none.
Shape needed: `POST /{version}/mappings/{mapping}/bind` taking `{ listing_url }`, refusing a mapping that is already bound.
