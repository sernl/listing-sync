# Leptos UI library coverage

All external pages in this file were fetched on 2026-09-03.
Repository metadata comes from the GitHub API on the same date.

## Executive summary

Rust/UI (rust-ui.com) is a real, actively developed, MIT-licensed copy-paste component registry for Leptos 0.8, with 98 source files covering roughly 90 components — larger than shadcn-svelte's 55.
It is not a crate you depend on: `cargo install ui-cli` then `ui add button` copies Rust source into your tree, so you own and maintain every component.
Its breadth is genuine but its depth is not uniform: twelve of those files implement their interactive behaviour by emitting raw JavaScript inside `<script>` tags that mutate the DOM behind Leptos's back, including MultiSelect and Command.
`DataTable` is a six-line re-export of `Table`; the sorting, filtering and row-selection its documentation advertises are demo-level wiring, not library code.
The real Leptos data-table answer is a different crate, `leptos-struct-table` 0.19.1, which is maintained (published 2026-09-02) and does supply sorting, selection, virtualisation and pagination.
Everything else in the Leptos ecosystem is stalled or dead: Thaw's last commit was 2026-05-07 and Leptonic's was 2024-12-19; RustForWeb's shadcn-ui and radix ports were both archived on 2026-02-02.
Of our documented UI grammar, roughly 15 of 24 patterns are covered by a rust-ui component, four need trivial composition, and five must be hand-built.
The five hard components cost 25 to 38 engineer-days in Leptos against 16 to 26 in Svelte, a delta of about 9 to 13 days — and the Svelte figure is against a codebase where 42 routes and 11 components already exist.
Accessibility is where the gap is widest and least recoverable: bits-ui documents the WAI-ARIA combobox descendant pattern per component, while rust-ui's MultiSelect trigger carries `tabindex="0"` and no `role`, no `aria-expanded`, and no arrow-key navigation.
Verdict: Leptos plus these libraries can reproduce the grammar, but not today and not for free, and the accessibility deficit is a product risk for a form-heavy tool aimed at teachers.

## What Rust/UI is

Rust/UI is a Shadcn-inspired component registry for Rust, published at rust-ui.com and developed at https://github.com/rust-ui/ui (fetched 2026-09-03).
The repository is MIT-licensed, created 2026-03-11, has 617 stars and 3 open issues, and was last pushed 2026-09-02 with 77 commits in the preceding 90 days.
It is maintained under the Rustify.rs banner, which the docs footer advertises alongside a nine-week paid bootcamp; the project is a marketing surface for a training business as well as a library, which is a maintenance-incentive fact rather than a criticism.

Components are consumed by copy-paste, not by a crate dependency.
The installation page states the design goal plainly: "Rust/UI is designed to be as minimal as possible. We don't rely on third party libraries like Radix UI, instead building components from the ground up" (https://rust-ui.com/docs/installation.md).
The workflow is `cargo install ui-cli --force`, then `ui init` in an existing project, then `ui add button` per component (https://rust-ui.com/docs/cli.md).
`ui-cli` is on crates.io at 0.3.16, published 2026-06-19, with 12,689 downloads.
Two supporting crates are real dependencies rather than copied source: `leptos_ui` 0.3.22 and `tw_merge` 0.1.21, both published 2026-04-03.

The version target is Leptos 0.8 with the `nightly` feature enabled, and the repository pins `channel = "nightly"` in `rust-toolchain.toml`.
Leptos itself is at 0.8.19 stable (2026-06-25) with 0.9.0-beta published 2026-07-18, so rust-ui tracks the current stable line but requires a nightly compiler.
For a repository that builds under a nix flake, a nightly pin is a reproducibility cost that should be priced before adoption, not after.

Release cadence has no semantic versions.
The repository's tags are deploy stamps — `deploy_2026/09/02_10h08m11s` is the most recent — so consumers cannot pin a component-set version and there is no changelog contract for breaking changes to copied source.
In practice this matters less than it would for a crate, because copied source does not update itself; it matters more when you want to pull a fix.

The behaviour finding is the one that should drive the decision.
Twelve files under `app_crates/registry/src/ui/` contain a literal `<script>` tag (GitHub code search, 2026-09-03).
In `multi_select.rs`, opening and closing the dropdown, outside-click dismissal and Escape handling are implemented in a JavaScript string emitted into the page, which registers `document.addEventListener('keydown', ...)` globally and calls `window.ScrollLock.unlock(200)`.
In `command.rs`, arrow-key navigation and Enter selection are likewise JavaScript, and the active option is tracked by calling `item.setAttribute('aria-selected', 'true')` directly on DOM nodes.
Three consequences follow: the interaction logic is untyped and outside Rust's compile-time reach, which negates a principal reason for choosing Leptos; the global listeners have no unmount cleanup, so repeated mounts accumulate handlers; and DOM mutation outside the reactive graph is a hydration hazard under SSR.

## Component inventory against our UI grammar

The nav on https://rust-ui.com/docs/components (fetched 2026-09-03) lists 77 components; the repository tree carries 98 Rust files under `app_crates/registry/src/ui/`, a handful of them `mod.rs` and internal helpers, so the site under-reports what has been written.
The registry modules are: accordion, action_bar, alert, alert_dialog, animate, aspect_ratio, attachment, auto_form, avatar, badge, bento_grid, bottom_nav, breadcrumb, bubble, button, button_action, button_group, callout, card, card_carousel, carousel, charts, chat, checkbox, chips, collapsible, command, context_menu, data_grid, data_table, date_picker, date_picker_dual_state, dialog, direction_provider, drag_and_drop, drawer, dropdown_menu, empty, expandable, faq_transition, field, footer, form, header, hover_card, image, input, input_group, input_otp, input_phone, input_prompt, item, kbd, label, link, marker, marquee, mask, menubar, message, multi_select, navigation_menu, pagination, popover, pressable, progress, radio_button, radio_button_group, scroll_area, select, select_native, separator, sheet, shimmer, sidenav, skeleton, slider, sonner, spinner, status, stepper, switch, table, tabs, textarea, theme_toggle, toast, toast_custom, toaster, toggle_group, tooltip.
Thirty-one hooks ship alongside, including `use_virtual_scroll`, `use_pagination`, `use_column_state`, `use_cell_selection`, `use_cell_edit`, `use_drag_selection` and `use_form`.

The table below scores each pattern from `docs/research/rethink/vendoo-workflows-and-ux.md` §11 and each control from the field catalogue in `docs/research/rethink/tpt-product-model.md`.
Covered means a component exists and does the job.
Compose means existing primitives suffice with layout and state written by us.
Build means no component in any surveyed Leptos library does it.

| Pattern or control | rust-ui component | Elsewhere in Leptos | Verdict |
|---|---|---|---|
| Left sidebar, text labels, grouped | `sidenav` | Thaw `nav` | Covered |
| Help menu behind an icon | `dropdown_menu` | Thaw `menu` | Covered |
| Three-column inventory board | none | none | Compose (CSS grid) |
| Per-marketplace glyph strip, five states | none | none | Compose (icon row plus data attributes) |
| Stale and sold-but-listed badges with tooltips | `badge`, `tooltip` | Thaw `tag`, `tooltip` | Covered |
| Coloured label chips, searchable | `chips`, `badge` | Thaw `tag` | Covered |
| Advanced filters | `multi_select`, `select`, `input` | — | Compose (no filter-state engine) |
| Bulk select, page-size selector | `checkbox`, `pagination`, `select` | — | Covered |
| Minimisable floating progress box | `progress`, `sonner` | Thaw `loading_bar` | Compose |
| Activity log table with failure rows | `data_table` is a `table` alias | `leptos-struct-table` 0.19.1 | Covered by a different crate |
| Detected-sales inbox | `item`, `badge`, `card` | — | Compose |
| Split editor with a destination rail | no `resizable` | none | Build, or use a fixed rail |
| Per-marketplace tabs | `tabs` | Thaw `tab_list` | Covered (shell only) |
| Inline "Update all" and "Reset" on diverged fields | none | none | Build |
| Required-field asterisk and inline missing-field notice | `field`, `form`, `auto_form` | Thaw `field` | Covered |
| Connections rows with per-marketplace toggles | `switch`, `card` | Thaw `switch` | Covered |
| Automation rule editor | `form`, `select`, `switch`, `date_picker` | Thaw equivalents | Compose |
| Toast queue | `sonner`, `toaster`, `toast` | Thaw `toast` | Covered |
| Command palette | `command` | — | Covered, script-injected |
| Capped multi-select with chips and a visible count | `multi_select`, no cap, no chips in trigger | Thaw `tag_picker` has chips, no cap | Build a wrapper |
| Four-level searchable standards tree, multi-select | none | Thaw `tree`, no search, no checkbox multi-select | Build |
| Dropdowns: tax code, duration, answer key, audience | `select`, `select_native` | Thaw `select`, `combobox` | Covered |
| File upload with per-file progress | `dropzone` (install reads "Coming soon"), `attachment` | Thaw `upload`, `upload_dragger`; `leptos-use` `use_drop_zone` | Build the progress path |
| Four fixed thumbnail slots | none | none | Compose over the uploader |
| Price group: free toggle, price, licence price, bundle discount | `input`, `switch`, `field` | Thaw equivalents | Covered |
| Two copyright checkboxes | `checkbox` | Thaw `checkbox` | Covered |
| Draft or live status control | `toggle_group`, `radio_button_group` | Thaw `radio` | Covered |
| Rich-text description editor, HTML to 45,000 chars | none | none | Build, see below |

Five gaps have no component anywhere in the Leptos ecosystem surveyed: a capped multi-select combobox with chips, a searchable tree picker, a resizable split pane, an upload with progress, and a rich-text editor.
The sixth apparent gap, a sorting and filtering data table with row selection, is closed by `leptos-struct-table` rather than by rust-ui.

Two corrections to what the documentation claims are worth recording, because both would otherwise be taken at face value.
The Data Table page describes "Filtering, Sorting, Column Visibility, Row Selection, Pagination" (https://rust-ui.com/docs/components/data-table.md), but `app_crates/registry/src/ui/data_table.rs` is six lines that re-export `Table` under new names; the features are demo wiring built from MultiSelect, Checkbox, Input and DropdownMenu, not library code.
`data_grid` is a separate, genuine thing but a different thing: `use_data_grid_state` composes cell selection, drag range selection and clipboard copy, which is a spreadsheet grid, not a record table.

The rich-text editor deserves a flag it did not get in the brief.
TPT's description field is HTML with a 45,000-character ceiling, and no Leptos library surveyed ships a WYSIWYG editor.
In Svelte you bind TipTap or ProseMirror directly; in Leptos you bind the same JavaScript library through `wasm-bindgen`, which is a hand-written FFI surface rather than an import.

## Alternatives in the Leptos ecosystem

| Library | Licence | Latest release | Last commit | Commits in 90 days | Leptos target | Status |
|---|---|---|---|---|---|---|
| rust-ui/ui | MIT | deploy tags only, 2026-09-02 | 2026-09-02 | 77 | 0.8, nightly | Active |
| thaw-ui/thaw | MIT | 0.4.8 (2025-08-03), 0.5.0-beta (2025-05-03) | 2026-05-07 | 0 | 0.8.5 | Stalled |
| lpotthast/leptonic | Apache-2.0 | 0.5.0 (2024-02-01) | 2024-12-19 | 0 | 0.5 era | Dead |
| cloud-shuttle/leptos-shadcn-ui | MIT | 0.9.0 (2025-09-20) | 2026-01-10 | 0 | 0.8+ claimed | Stalled |
| cloud-shuttle/radix-leptos | — | 0.9.0 (2025-09-22) | — | — | — | Stalled |
| RustForWeb/shadcn-ui | MIT | — | 2026-02-02 | — | — | Archived |
| RustForWeb/radix | MIT | — | 2026-02-02 | — | — | Archived |
| RustForWeb/floating-ui | MIT | — | 2026-09-02 | — | Leptos, Yew, Dioxus | Active |
| Synphonyte/leptos-struct-table | MIT/Apache-2.0 | 0.19.1 (2026-09-02) | 2026-09-02 | — | 0.8 | Active |
| Synphonyte/leptos-use | Apache-2.0 | 0.19.2 (2026-09-01) | 2026-09-01 | 65 | 0.8 | Active |

Thaw is the most interesting of the stalled options because its inventory is closest to our needs.
Its 52 modules include `tree` and `tree_item`, `tag_picker` with `tag_picker_input` and `tag_picker_option`, `upload` with `upload_dragger`, `combobox`, `auto_complete`, `calendar`, `color_picker`, `toast` and `message_bar`, plus a private `_aria` module.
Nothing else in Leptos ships a tree or a chip-bearing multi-select picker.
Against that, Thaw is Fluent-derived — it will look like a Microsoft product, not like cream and terracotta — its stable release is 13 months old, its 0.5 line has been in beta since 2025-05-03, and it has 33 open issues with no commits in 90 days.
Depending on it means either accepting a frozen library or forking it.

Leptonic is dead: the last commit to its default branch was 2024-12-19 and its only crates.io release is from 2024-02-01, predating Leptos 0.6.

The shadcn ports are a cautionary set.
RustForWeb built the most principled ones — a Radix port and a shadcn/ui port on top of it — and archived both on 2026-02-02.
The cloud-shuttle crates that remain claim "46 production-ready components with 100% test coverage" and "WCAG 2.1 AA compliance" in the repository description, but the repository has 70 stars, 10 open issues, no commits since 2026-01-10, and the compliance claim is unaudited here.
The one RustForWeb project still alive is `floating-ui`, pushed 2026-09-02, which supplies the positioning primitives any popover, combobox or tooltip needs.

daisyUI via Tailwind with hand-written components is a legitimate fourth path and the least discussed.
It supplies class names only, so every behaviour — focus management, keyboard navigation, dismissal, ARIA state — is ours to write.
That is the honest baseline against which the libraries above should be measured: rust-ui's value is the twelve script-injected behaviours plus the styling of 86 others, and if the script-injected ones must be rewritten anyway, the gap between rust-ui and daisyUI narrows considerably.

The Tailwind integration story is the one part of this that is genuinely easy.
Tailwind v4 scans every file in the project regardless of extension, excluding only `.gitignore`d paths, `node_modules`, binaries, CSS files and lockfiles (https://tailwindcss.com/docs/detecting-classes-in-source-files, fetched 2026-09-03), so `.rs` files are picked up with no content configuration at all.
`cargo-leptos` 0.3.7 (published 2026-07-03, 757,183 downloads) reads a `tailwind-input-file` key from `[package.metadata.leptos]` and runs the Tailwind build itself; rust-ui's own `Cargo.toml` uses exactly that.
Trunk 0.22.0-beta.2 is the alternative for client-side-only builds.
One caveat carries over from the plain-text scanning rule: Tailwind cannot resolve classes assembled by string concatenation, which is precisely what `tw_merge` and Rust `format!` calls encourage, so dynamic class construction needs `@source inline(...)` safelisting.

Our existing investment partly survives.
`web/package.json` already depends on `tailwindcss` and `@tailwindcss/vite` at ^4.1.0, and `web/src/app.css` is 1,806 lines defining our palette as CSS custom properties: `--ground: #faf7f2`, `--accent: #c2543a`, `--ink: #1f1a17`, with `--serif: 'Fraunces'` and `--sans: 'Instrument Sans'`.
rust-ui components are styled against shadcn's semantic token names — `bg-popover`, `text-muted-foreground`, `bg-accent`, `border-input`, `ring-ring` — so adopting them means writing a mapping layer from our token names to theirs, which is perhaps a day of work and is the only part of the styling investment that transfers cleanly.
The 1,806 lines of hand-written component CSS do not transfer, because they are keyed to Svelte component markup.

## The five hard components, priced

Estimates are engineer-days for one competent engineer, including keyboard and ARIA work and unit tests, excluding design iteration.
Leptos figures assume rust-ui as the starting point where a component exists.
Svelte figures assume the current codebase, which already holds 42 route and component files including `UploadField.svelte` and `AxisField.svelte`.

| Component | Leptos library support | Leptos days | Svelte library support | Svelte days |
|---|---|---|---|---|
| Capped multi-select with chips and count (grade ≤4, subject ≤3, tag ≤6, format ≤3) | rust-ui `multi_select` exists; no cap, no chips in trigger, no arrow keys, Escape only, behaviour in injected JS | 4–6 | bits-ui `Combobox` with `type="multiple"`, full keyboard, WAI-ARIA descendant pattern; no cap | 1.5–2.5 |
| Four-level searchable standards tree, multi-select | none in rust-ui; Thaw `tree` has no search, no checkbox multi-select, no virtualisation, and is stalled | 8–12 | none in bits-ui or shadcn-svelte either; hand-built from primitives | 6–9 |
| Sortable, filterable, virtualised data table with row selection | `leptos-struct-table` 0.19.1: sorting including multi-column, single and multi selection, virtualisation, pagination, async data provider, headless classes, custom cell renderers | 3–5 | `@tanstack/svelte-table` 9.2.4 published 2026-08-28, plus TanStack Virtual | 2–4 |
| File uploader with per-file progress, four thumbnail slots | rust-ui `dropzone` install reads "Coming soon"; `leptos-use` `use_drop_zone` gives the drop primitive; no progress anywhere, and `fetch` has no upload progress so `XmlHttpRequest` must be bound through `wasm-bindgen` | 4–6 | `UploadField.svelte` already exists at 137 lines; XHR progress is idiomatic | 2–3 |
| Per-marketplace tabbed form with divergence badges and "Update all" / "Reset" | tabs exist in both; the canonical-versus-override signal graph across N marketplaces and roughly 30 fields is application logic with no library anywhere | 6–9 | same logic; Svelte 5 runes plus existing `AxisField.svelte` and `Panel.svelte` scaffolding | 5–7 |
| Total | | 25–38 | | 16.5–25.5 |

The delta on these five is roughly 9 to 13 engineer-days.
Two smaller items sit outside the table and widen it slightly: a resizable split pane, which shadcn-svelte ships as `resizable` and rust-ui does not (1–2 days in Leptos, 0 in Svelte, and 0 in both if the rail is fixed-width as Vendoo's appears to be), and the rich-text description editor, where a Svelte TipTap binding is an import and a Leptos one is a hand-written `wasm-bindgen` FFI surface (add 3–5 days in Leptos).

The delta above prices components only.
It does not price re-authoring the 42 existing route and component files, which is the framework agent's question rather than this one.

## Accessibility

The evidence points one way and it is not close.

For rust-ui, `app_crates/registry/src/ui/multi_select.rs` gives options `role="option"` and a reactive `aria-selected`, which is correct as far as it goes.
The trigger, at line 159 of that file, is a `<button>` with `tabindex="0"` and no `role="combobox"`, no `aria-expanded`, no `aria-controls` and no `aria-haspopup`.
There is no arrow-key navigation in the multi-select at all; the only keyboard handling is an Escape listener registered on `document` from an injected script.
In `command.rs`, ArrowDown, ArrowUp and Enter are handled, but again in injected JavaScript that sets `aria-selected` by direct DOM mutation rather than through Leptos state, and the listener is attached to `document` without teardown.
The project publishes no accessibility statement and its docs pages carry no per-component keyboard tables.
End-to-end tests exist under `e2e/tests/components/`, which is more rigour than most Leptos libraries show, but they are behavioural rather than assistive-technology tests.

Thaw carries a private `_aria` module and inherits Fluent UI's interaction patterns, which is the strongest a11y position in Leptos, but it has had no commits in 90 days and its stable release is 13 months old.

cloud-shuttle/leptos-shadcn-ui asserts "WCAG 2.1 AA compliance" in its GitHub description.
That claim is recorded here and marked unverified; the repository has not been committed to since 2026-01-10 and the assertion was not audited against its sources.

On the Svelte side, bits-ui documents accessibility per component rather than as a banner claim.
The Combobox page (https://bits-ui.com/docs/components/combobox, fetched 2026-09-03) states that it is "Built with ARIA attributes and keyboard interactions to ensure screen reader compatibility and accessibility standards", that the input "retains focus the entire time, even when navigating with the keyboard" — the WAI-ARIA combobox descendant pattern — and exposes `loop`, `scrollAlignment` defaulting to `'nearest'`, a `data-highlighted` attribute, and `onHighlight` and `onUnhighlight` callbacks.
It supports `type="multiple"` natively, though it has no maximum-selection cap either.

For a product whose core surface is a long form that teachers will fill in repeatedly, this is a substantive difference, and it is the least recoverable one.
Breadth of components can be closed by writing components; a keyboard and ARIA model has to be designed into each one, and rust-ui's is currently implemented in a layer where Rust's type system cannot help.

## Design reproduction

This section states a commercial risk, not legal advice; a lawyer should rule on it if the founder wants to proceed with close copying.

Reproducing a competitor's dashboard "in every way" is a different act from adopting its workflow.
Interaction patterns, information architecture, navigation structure and the sequence of steps a user moves through are the normal currency of the industry and are copied freely and openly between products.
An exact visual reproduction — the same palette, the same typography, the same iconography, the same spacing and the same overall look — is where trade-dress and copyright questions start, because the visual presentation of an interface can function as an identifier of source in a way that its workflow does not.
The prior legal memo for this project already found that the exposed surface is architecture rather than permission; visual identity is a second, independent surface, and it is cheap to stay clear of.

The recommended line is: same patterns, same information architecture, our own visual identity.
Copy the three-status board, the glyph strip encoding per-marketplace state, the badge overlays, the destination rail with per-marketplace tabs, the per-field "Update all" and "Reset" divergence affordance, the connections page shape, and the rule-editor structure — all of which are documented in `docs/research/rethink/vendoo-workflows-and-ux.md` §11 precisely enough to rebuild without looking at Vendoo again, which is itself the right way to do this.
Render all of it in the identity already approved and already implemented in `web/src/app.css`: cream `#faf7f2`, terracotta `#c2543a`, Fraunces for display and Instrument Sans for text.
This costs nothing, because the patterns carry the usability and the palette carries none of it.

## Verdict

Leptos plus the available libraries can reproduce the documented UI grammar.
Nothing in the grammar is impossible, and rust-ui is a better starting point than the Leptos ecosystem had a year ago.

The cost is 25 to 38 engineer-days on the five hard components, plus 3 to 5 more for a rich-text editor binding and 1 to 2 for a split pane, against 16 to 26 in Svelte for the same five.
The delta is 13 to 20 engineer-days of net new component work, and that number sits on top of, not instead of, whatever the framework migration itself costs.

Three risks travel with the Leptos choice and should be weighed alongside the day count.
The interactive behaviour we would inherit from rust-ui's multi-select and command palette is untyped injected JavaScript with global listeners and no unmount cleanup, which we would end up rewriting in Rust for exactly the reasons that motivated choosing Rust.
The ecosystem has one healthy component library, and the previous three — Leptonic, RustForWeb/radix, RustForWeb/shadcn-ui — are dead or archived, so concentration risk is real and recent.
Accessibility is materially behind and the deficit sits in the components a form-heavy product uses most.

Against that, the Leptos side has two genuine strengths that a pessimistic reading would miss: `leptos-struct-table` is a better data table than the Svelte ecosystem's default answer of assembling TanStack Table by hand, and Tailwind v4 needs no configuration at all to scan `.rs` files, so the styling pipeline is a non-issue.

Finishing in Svelte is the lower-cost path for the UI specifically, by roughly two to four engineer-weeks, and it starts from 42 files that already exist.
Whether that is decisive depends entirely on the reasons for the Leptos move, which are cross-platform reach and language unification rather than UI capability, and which this file does not assess.
If the founder wants the Leptos move for those reasons, the UI is not a blocker; it is a bill.

## Open questions

Q1. Is a nightly Rust toolchain acceptable in this repository, given rust-ui pins `channel = "nightly"` and enables Leptos's `nightly` feature, and given the flake and the enforcement toolchain assume a pinned stable?

Q2. Do we adopt rust-ui by copying its components and then rewriting the twelve script-injected ones in Rust, or do we take daisyUI and write everything, given that the second option's cost gap narrows once the rewrites are counted?

Q3. Is Thaw's `tree` and `tag_picker` worth forking to close the two biggest gaps, accepting maintenance of a stalled Fluent-styled library, or is hand-building both the cleaner commitment?

Q4. Does the standards tree need virtualisation at launch? The answer depends on the node counts per jurisdiction in `docs/research/rethink/education-standards-sources.md`, which this file did not open, and it moves the tree estimate by several days.

Q5. Is the rich-text description editor in scope for the first release, or does a plain textarea with a character counter suffice until it is not?

Q6. Does the founder accept the "same patterns, own identity" line, or does "the same design in every way" need a legal ruling before the design work starts?

## Unverified

U1. rust-ui component quality beyond MultiSelect, Command, DataTable and DataGrid; four modules of 98 were read in full and the remainder are assessed by name, documentation and the presence of a `<script>` tag.

U2. Whether the twelve script-injecting modules are being migrated to Rust; no issue or roadmap entry was checked.

U3. cloud-shuttle/leptos-shadcn-ui's "WCAG 2.1 AA compliance" and "100% test coverage" claims, taken from the repository description and not audited.

U4. Thaw's `tree` capabilities in detail; the module names `tree`, `tree_item` and `tree_item_layout` were read but the source was not, so whether it supports checkbox multi-select or lazy loading is unknown.

U5. Whether `leptos-struct-table`'s virtualisation performs acceptably at our row counts; the README claims it, no benchmark was run.

U6. The engineer-day estimates are the author's judgement calibrated on the component inventories, not measurements, and they assume an engineer already fluent in Leptos signals; a first Leptos project should be expected to run over.

U7. bits-ui's per-component ARIA claims were read from its Combobox documentation page only; other components were not checked.

U8. Whether rust-ui's e2e tests cover keyboard paths; the file list under `e2e/tests/components/` was seen, the tests were not read.

U9. Leptos 0.9's release timing and whether rust-ui, `leptos-struct-table` and `leptos-use` will follow it promptly; 0.9.0-beta has been out since 2026-07-18 with no stable date announced.

U10. Whether `tw_merge`'s runtime class merging defeats Tailwind's static scanning in practice, requiring `@source inline(...)` safelists; the mechanism implies a risk, no build was run to confirm it.
