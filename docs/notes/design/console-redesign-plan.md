# The console redesign

Redrawing the console, its emails and its landing page in one wave.

- date: 2026-09-05
- status: in progress; the shell and the backend wave are under way, four slices are delivered and awaiting review, the rest are planned; amended 2026-09-11 when the founder's brand kit replaced the palette, mark, sidebar words and pricing this plan carried, and `brand-kit-and-teacher-ui.md` became the design of record for every visual and wording decision below
- sources: the console design specification and its build order, the console capability inventory, the founder's reference screenshots of a comparable product, the pricing and landing research, the marketplace catalogue, and `docs/notes/design/vendoo-for-teachers-rethink.md`
- paths: `web/`, `crates/tam-api/`, `crates/tam-types/`, `crates/tam-server/`, `auth/`, `apps/landing/`

## Purpose and scope

The console is redrawn on a four-band shell — icon rail, secondary navigation card, gap, rounded content region — with our own palette, our own words and Lucide icons.
Layout and interaction are modelled on the founder's reference screenshots of a comparable product; palette, iconography, wording and data are ours.
The wave covers the console, the two identity emails and the landing page, and it ends with the whole surface green under `nix flake check` and deployed.
No marketplace capability is added: every page renders a verb the backend already has, or states plainly that the verb is not built.

## Decisions of 2026-09-05

The palette was Kauri (ground `#F7F2E9`, primary `#6B4423`, accent `#C2543A`, additive `#3E5A8C`), then Pounamu on 2026-09-06, and since 2026-09-11 it is the founder's brand kit recorded in `brand-kit-and-teacher-ui.md`; the token names this plan introduced are unchanged and only the values moved.
The house-and-book mark was the product icon until 2026-09-11; the brand kit's house on an indigo tile is now.
Main navigation was three sections — Crosslist, Automations, Marketplaces — drawn with Lucide icons, and the Lucide dependency is approved; on 2026-09-11 Import became its own first section in the founder's words, so the rail reads Import, Crosslist, Automations, Marketplaces, Account.
Crosslist carries five pages: Resources at the route `/resources`, Labels, Analytics, Template Manager and Export; Import moved to its own section.
Marketplace Sharing means publishing one resource to every connected marketplace in one scheduled action, and it ships disabled with a stated reason until the app can run it.
Template Manager covers both senses: marketplace mapping templates and new-resource templates.
Export is a CSV of the catalogue carrying each marketplace's status, price and link, and it never moves a file.
Prices were quoted in USD as Solo $12 monthly or $120 annually, Studio $24 or $240, Publisher $48 or $480, beside a free tier and a 14-day Studio trial, with one-off migrations banded at $49, $79, $129, $199 and $299; on 2026-09-11 the founder's mockup replaced them with the Catalogue Import ladder, one subscription and the Founding 100 offer, recorded in `decisions.md` under that date.
"Powered by PLE Group" links back to our own site, not to a PLE Group site: there is no PLE Group URL to point at, and the founder's decision is that the attribution reads as ours.
Brand logos are shown on the Marketplaces page under a disclaimer that the marks belong to their owners.
The landing page takes `/` and the console home moves to `/app`, every deep route unchanged, and the split is revisited at the teachouse.io cutover.
Emails are HTML with an informal greeting and a welcome illustration, and they cover the reset flow; their palette follows the brand kit since 2026-09-11.
The Android build shows this same console inside a phone shell.
A marketplace we have not built appears as a tile reading "Coming soon" for Etsy and Shopify and "On our list" for the rest.

Four earlier decisions bind the wave and are not reopened.
The two-branch automation rule of 2026-09-02 puts every request to a marketplace without an official API on the seller's own device, which is why every marketplace row carries a transport badge, no Automations page has a "now" button, and each one schedules and says so.
The metadata-only import of the same date is stated on the Import page: we receive a description of each resource, and the file itself never leaves the seller's machine.
The surface order of 2026-09-02, narrowed 2026-09-03 — Windows, then Android, then iOS, with the browser extension deferred — is why the Apple download and both extension cards read as not available.
The landing page stays a separate prerendered build, decided 2026-09-03, because the console's root layout turns off both server rendering and prerendering.

## Phase one — the shell

Five slices, all in the console, each falsified by `just web-check`.

| Slice | What it does | Lane | Owner | State |
|---|---|---|---|---|
| S1 tokens and type | Rewrites the `:root` block to the palette values and the size, spacing, radius and shadow scales, leaving every existing class in place. Re-run on the brand kit on 2026-09-11. | `just web-check` | web | in progress |
| S2 icons | Adds a hand-copied Lucide path map and a small `Icon.svelte`, closing the icon name over what is actually shipped so a typo stops the lane. | `just web-check` | web | in progress |
| S3 shell | Rewrites `nav.ts` into a section model and `Console.svelte` into rail, secondary card and content region, with the four mobile tabs derived from the sections. | `just web-check` | web | in progress |
| S4 routes | Renames `/resources` to `/guides`, adds the empty pages, moves the console home to `/app`, and updates the redirect table. | `just web-check` | web | in progress |
| S5 components | Writes RowCard, StatusPill, Button, Toggle, TabBar, Menu, Banner, AddCard, EmptyState, ActivityLog and Field, and splits their rules out of the 2266-line sheet. | `just web-check` | web | in progress |

## Phase two — foundations

What a page in phase three cannot be written against until it exists.

| Slice | What it does | Lane | Owner | State |
|---|---|---|---|---|
| transport class | Adds `transport_class` to `ConnectionView` and `InventoryStatusView`, read from the registry with no storage change. | `just check` | backend | in progress |
| marketplace requests | One migration for `marketplace_request` and one org-scoped `POST /v1/marketplace-requests`. | `just check`, then `just db-test` | backend | in progress |
| label routes | `PATCH` and `DELETE /v1/labels/{name}` over `LabelRepo`, the only label operations missing. | `just check`, then `just db-test` | backend | in progress |
| product icons | Puts the product mark on the console, the landing site, the email header and the desktop and Android bundles. Re-run on the brand-kit mark on 2026-09-11. | `just web-check` | web | in progress |
| export route | `GET /v1/products/export`, streaming CSV from the page walk `list_products` already uses. | `just check` | backend | delivered awaiting review |
| landing serving | `tam-server` answers `/` from the landing build ahead of the console, under its own narrower policy. | `just check` | backend | delivered awaiting review |

## Phase three — the pages

Each page renders a verb that exists or says plainly that it does not, in wording written for a teacher rather than for a marketplace operator.

| Slice | What it does | Lane | Owner | State |
|---|---|---|---|---|
| S6 Marketplaces | The card grid with status, transport line, extension and download sections, and the tiles for what is not built. | `just check`, then `just web-check` | web | planned |
| S7 request form | The dashed add-card and the request form over the new route, collapsing to a confirmation banner. | `just web-check` | web | planned |
| S8 Resources | The filter card, four counted tabs, row cards, the bulk bar, and only the five bulk verbs the backend has. | `just web-check` | web | planned |
| S9 Labels | The label list with counts, rename, merge and delete, with the twenty-label cap stated in the hint rather than discovered on submit. | `just web-check` | web | planned |
| S10 Automations | Splits today's `/sync` into Migration and Sync, moves the open-questions entry point into the Sync header, and ships Sharing behind its not-built banner. | `just web-check` | web | planned |
| S11 Export page | The label, marketplace and column form over the export route. | `just web-check` | web | planned |
| S12 Template Manager | Two tabs over today's override editor, the second an empty state until its backend exists. | `just web-check` | web | planned |
| S13 emails | An HTML template beside the existing text, the corrected subjects, the greeting and the hosted mark, leaving the detached send untouched. | `just auth-check`, plus one live send | backend | delivered awaiting review |
| landing page | The approved prices at `/` and at `/pricing`. | `just landing-check` | web | delivered awaiting review |

## Phase four — review, gates and deployment

Every slice lands the same way, in this order.
Adversarial review first, against the specification and the capability inventory rather than against the diff alone.
Then the slice's own lane, which is the narrowest thing that could falsify it.
Then landing on main.
Then deployment to thunderstorm with `clan machines update`, run from the dotfiles root, never deploy-rs.

The wave closes with one slice of its own: S14 runs `just pre-push` and then `nix flake check` over the whole tree, and nothing runs the full flake check before then.

## Out of scope for this wave

Billing plan metering stays out: there are no plan definitions in the code and exactly one price id, so the pricing page states the ladder while the console keeps tracking Paddle's status and gating nothing.
The Tes analytics reader stays out: both device read routes are fixed to TPT, and the Analytics page states that gap rather than hiding it.
The Sharing automation on the device stays out: no share, bump or relist concept exists anywhere in the codebase and neither marketplace has a captured endpoint for one, so the page ships as shape only.
The browser extension stays out, deferred by the decision of 2026-09-03, and both extension cards say so.
Apple builds stay out, by the surface order and by Apple's licence forbidding macOS on non-Apple hardware, so the Apple card carries no button.

## Owed by the founder

A GitHub read-only token, so the Android download resolves to a real file rather than to a link that 404s.
The TES and TPT logo files, since neither marketplace publishes a mark we may take without asking.
Until both arrive the Android button ships disabled with its reason in the tooltip, and the two live tiles carry wordmarks set in our own typeface.

## Amendment of 2026-09-12 — the phone bottom bar

The founder read the seven-cell phone bar this wave shipped and rejected it: too many icons, and the new-resource action has to sit dead centre rather than beside a fixed account column that was never symmetric.
The bar is now five cells on five equal tracks — Import, Catalogue, New, Automate, Markets — where New is the third of five and is therefore on the viewport's own centre line at any width.
Catalogue, Automate and Markets are bar-only wording for the Crosslist, Automations and Marketplaces sections: the rail's names are unchanged, and each cell keeps the section's full name as its accessible name.

Search left the bar for a control in the Resources page head, because search is a task rather than a destination; it opens the same Ctrl-K palette, which is unchanged.
Account left the bar for a phone-only avatar button in the top right of every main page head, which opens the Account section at `/settings`; above 620px the rail and the top strip already carry it, so the button is drawn only below that width.

The geometry: 80px of bar content with the safe-area bottom inset spent beneath it, `repeat(5, minmax(0, 1fr))` tracks, 24px glyphs, 12px/16px Inter labels at weight 500 and 600 when active, a 64 by 32 lavender pill behind the active glyph, and an inline flat 48px indigo disc with a white plus for New — no elevation, no overlap, and no selected state, since creating a resource is never the page you are on.
Page content reserves 80px plus the inset plus 16px beneath itself.
Measured in headless Chromium against the built stylesheet and the shipped Inter: at 360px the cells are 72px and New's centre is at 180px; at 390px they are 78px and it is at 195px; the longest label, "Catalogue", is 59px inside 64px of content box at 360px, so nothing truncates.
The page header also becomes two rows below 620px — glyph and controls above, title and sentence below — because three 44px controls and a 28px title do not share a 360px row.
