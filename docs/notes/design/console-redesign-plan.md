# The console redesign

Redrawing the console, its emails and its landing page in one wave.

- date: 2026-09-05
- status: in progress; the shell and the backend wave are under way, four slices are delivered and awaiting review, the rest are planned
- sources: the console design specification and its build order, the console capability inventory, the founder's reference screenshots of a comparable product, the pricing and landing research, the marketplace catalogue, and `docs/notes/design/vendoo-for-teachers-rethink.md`
- paths: `web/`, `crates/tam-api/`, `crates/tam-types/`, `crates/tam-server/`, `auth/`, `apps/landing/`

## Purpose and scope

The console is redrawn on a four-band shell — icon rail, secondary navigation card, gap, rounded content region — with our own palette, our own words and Lucide icons.
Layout and interaction are modelled on the founder's reference screenshots of a comparable product; palette, iconography, wording and data are ours.
The wave covers the console, the two identity emails and the landing page, and it ends with the whole surface green under `nix flake check` and deployed.
No marketplace capability is added: every page renders a verb the backend already has, or states plainly that the verb is not built.

## Decisions of 2026-09-05

The palette is Kauri: ground `#F7F2E9`, primary `#6B4423`, accent `#C2543A`, additive `#3E5A8C`.
The house-and-book mark is the product icon everywhere.
Main navigation is three sections — Crosslist, Automations, Marketplaces — drawn with Lucide icons, and the Lucide dependency is approved.
Crosslist carries six pages: Resources at the route `/resources`, Labels, Import, Analytics, Template Manager and Export.
Marketplace Sharing means publishing one resource to every connected marketplace in one scheduled action, and it ships disabled with a stated reason until the app can run it.
Template Manager covers both senses: marketplace mapping templates and new-resource templates.
Export is a CSV of the catalogue carrying each marketplace's status, price and link, and it never moves a file.
Prices are quoted in USD: Solo $12 monthly or $120 annually, Studio $24 or $240, Publisher $48 or $480, beside a free tier and a 14-day Studio trial.
One-off migrations are banded at $49, $79, $129, $199 and $299, the last covering the first 500 resources with $0.25 for each one beyond.
Bands combine with tiers by three rules: a migration is buyable with no subscription and includes 30 days of Studio; a subscriber's annual allowance of 50, 200 or 500 resources is consumed first and the band price applies at half beyond it; and a migration buyer who takes an annual plan within 30 days has the migration price credited in full against that first payment, capped at the annual price.
"Powered by PLE Group" links back to our own site, not to a PLE Group site: there is no PLE Group URL to point at, and the founder's decision is that the attribution reads as ours.
Brand logos are shown on the Marketplaces page under a disclaimer that the marks belong to their owners.
The landing page takes `/` and the console home moves to `/app`, every deep route unchanged, and the split is revisited at the teachouse.io cutover.
Emails are HTML in the Kauri palette with an informal greeting and a welcome illustration, and they cover the reset flow.
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
| S1 tokens and type | Rewrites the `:root` block to the Kauri values and the size, spacing, radius and shadow scales, leaving every existing class in place. | `just web-check` | web | in progress |
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
| product icons | Puts the house-and-book mark on the console, the landing site, the email header and the desktop and Android bundles. | `just web-check` | web | in progress |
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
