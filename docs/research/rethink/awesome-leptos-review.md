# Awesome Leptos, tested against the two Leptos notes

Whether anything in the `leptos-rs/awesome-leptos` catalogue changes the "do not switch" conclusion reached in `docs/research/rethink/leptos-frontend-fit.md` and `docs/research/rethink/leptos-ui-library-coverage.md`.

- date: 2026-09-03
- method: read-only retrieval of the list, the repositories it links, the crates.io API and the GitHub API; every claim carries its URL and the retrieval date 2026-09-03; no jj or git command was run, no dependency was added, nothing was built
- scope: the list as evidence, not a re-derivation of the two notes; where a finding contradicts them it is marked as a correction
- source of record: <https://raw.githubusercontent.com/leptos-rs/awesome-leptos/main/README.md> (retrieved 2026-09-03), 226 lines, last commit 2026-08-04

## Executive summary

The list does not change the conclusion, and its own commit history is the sharpest evidence for that conclusion rather than against it.
One entry in it is genuinely new to us: Leptodon, added on 2026-03-11, is a stable-toolchain Leptos 0.8.20 component library from openanalytics with eight releases between 2026-03-10 and 2026-08-21 and zero injected JavaScript in its library crate.
Leptodon closes one of the coverage note's five hard gaps outright — its `tag_picker` is a capped multi-select with chips in the trigger, arrow-key navigation and fuzzy search, all implemented in Rust — and supplies a real modal with `role="dialog"`, `aria-modal`, an Escape handler and a sentinel-span focus trap.
It closes none of the other four: no tree, no upload progress, no rich-text editor, no virtualisation of its own, and roughly thirty `aria`/`role` attributes across about thirty-five components, with tabs and popover carrying none at all.
The list also corrects one of the notes in the wrong direction: `leptos-fetch`, recorded as the live replacement for the stale `leptos_query`, has had no commit since 2026-03-02, so the TanStack Query gap is six months wider than the note recorded.
The list carries no Tauri bridge crate at all, only two starter templates and two demo applications, so it adds nothing to the fit note's finding that `tauri-sys` is git-only.
It also carries no testing tool, no form-validation crate, no tree widget, no virtualisation library and no upload-with-progress component, which is the ecosystem's own curated index reporting those absences.
Two of its entries are dead and still listed: `leptix` points at a repository last touched 2024-11-22 that pins `leptos = "0.6"`, whose crates.io name now belongs to an unrelated two-component crate, and Papelito's repository was archived on 2023-05-06, the day after it was added.
In the twelve months to 2026-09-03 the list gained two libraries and lost one; every other change was a website.
Verdict: the best combination on the list moves the component bill by roughly three to five engineer-days against a 90-to-140-day migration, which is under five per cent, and leaves the accessibility, data-fetching and Tauri findings standing.

## 1. The list, section by section

Six of the list's eight sections are irrelevant to the question and are dismissed here with their reason.
Resources is two links, one of them the book the fit note already cites.
Starter Templates is nine entries, none of which is a library.
Styling and Design is seven entries covering scoped CSS and icon sets, and the coverage note already established that Tailwind v4 scans `.rs` files with no configuration, so styling is not the constraint.
Quality of Life is two crates, `tracing-subscriber-wasm` and `wasm-bindgen-struct`, neither of which bears on any gap.
Alternate Macros is one entry, `leptos-mview`.
Blogs / Websites is thirty entries of prior art, two of which are worth naming and are treated in section 5.

That leaves Tools and Components, and within Components only the entries touching a documented need.

### Tools

| Entry | What it is | Latest release | Last commit | Bears on |
|---|---|---|---|---|
| `cargo-leptos` | Build coordinator, runs the Tailwind build itself | 0.3.7, 2026-07-03 | 2026-08-31, 23 commits since 2026-06-05 | Toolchain addition priced in fit note §4 |
| `leptosfmt` | `view!` macro formatter | 0.1.33, 2025-01-30 | 2025-03-21, 29 open issues | Toolchain addition; unmaintained for 17 months |
| `leptosfmt-action` | CI action wrapping the above | — | — | Enforcement lane |
| `leptos-fmt` / `cargo-runner` / `vscode-leptos-snippets` | Editor plugins | — | — | Nothing |

The Tools section contains no test harness, no linter, no accessibility checker and no component-test runner.
The fit note's three-layer testing story — `#[test]`, `wasm-bindgen-test`, Playwright — is the whole story, and the list confirms it by omission.
`leptosfmt` is the notable degradation: the fit note lists it as a toolchain addition, and it has had no commit since 2025-03-21 with 29 issues open (<https://api.github.com/repos/bram209/leptosfmt>, retrieved 2026-09-03).

### Components, scored against our documented needs

Only entries touching a need from the two notes are listed.
Release and commit dates come from the crates.io and GitHub APIs, retrieved 2026-09-03.
Star counts are recorded nowhere in this table because they are not evidence.

| Entry | What it is | Latest release | Last commit | Commits since 2026-06-05 | Leptos version | Bears on |
|---|---|---|---|---|---|---|
| Leptodon | Flowbite-styled component crate, ~35 components | 1.6.0, 2026-08-21 | 2026-08-31 | 19 | 0.8.20, stable toolchain | Capped multi-select, dialog, tabs, upload, forms |
| Rust/UI | Copy-paste registry, ~90 components | deploy tags only | 2026-09-02 | 75 | 0.8 with `nightly` features, `channel = "nightly"` | Everything; assessed in full by the coverage note |
| Thaw | Fluent-derived component library | 0.4.8, 2025-08-03 | 2026-05-07 | 0 | 0.8.5 | Tree, `tag_picker`, upload |
| `leptos-struct-table` | Table derived from a struct | 0.19.1, 2026-09-02 | 2026-09-02 | 21 | 0.8 | Data table, virtualisation, row selection |
| `leptos-use` | Reactive primitives | 0.19.2, 2026-09-01 | 2026-09-01 | 65 | 0.8 | `use_drop_zone`, `use_infinite_scroll`, `use_draggable` |
| `leptos-fetch` | Async query cache | 0.4.10, 2026-03-02 | 2026-03-02 | 0 | 0.8 | TanStack Query substitute |
| Rust Floating UI | Positioning primitives | 0.6.0, 2026-05-22 | 2026-09-02 | 46 | Leptos, Yew, Dioxus | Popover, combobox, tooltip anchoring |
| `leptos-chartistry` | `<Chart>` component | 0.2.3, 2026-01-23 | 2026-01-23 | 0 | 0.8 | Analytics page |
| `leptos_i18n` | Translation library | 0.6.2, 2026-04-14 | 2026-06-04 | 0 on default branch | 0.8 | Internationalisation |
| `leptos-fluent` | Fluent-based i18n, not in either note | 0.3.1, 2025-12-29 | 2026-04-24 | 0 | 0.8 | Internationalisation |
| `leptos_toaster` | Sonner-inspired toaster | 0.3.0, 2026-07-15 | 2026-07-15 | — | 0.8 | Toast queue |
| `leptoaster` | Minimal toast library | 0.2.3, 2025-05-23 | 2025-05-23 | — | 0.8 era | Toast queue |
| `leptix` | "Accessible and unstyled components" | `leptix_primitives` 0.2.3, 2024-11-22 | 2024-11-22 | 0 | `leptos = "0.6"` | Accessibility primitives |
| Rust shadcn/ui | RustForWeb shadcn port | — | archived 2026-02-02 | — | — | Component breadth |
| Papelito | "A simple WYSIWYG editor for leptos" | 0.1.0, 2023-05-05 | archived 2023-05-06 | 0 | git branch `main`, no version | Rich-text editor |
| `leptos-material` | Material Web Components wrapper | 0.6.1, 2024-08-23 | 2024-08-23 | 0 | 0.6 era | Component breadth |
| `leptos-hotkeys` | Declarative keybindings | 0.2.2, 2024-07-04 | 2025-01-25 | 0 | 0.6 era | Command palette shortcuts |
| `tailwind-fuse` | `tw_merge` equivalent | 0.3.2, 2025-01-19 | 2025-01-19 | 0 | n/a | Class merging, coverage note U10 |
| `leptos-unique-ids` | Globally unique DOM ids | 0.1.1, 2025-06-16 | — | — | 0.8 era | `aria-controls` wiring |
| `leptos-pdf` | PDF rendering components | 0.8.1, 2025-12-27 | — | — | 0.8 | Nothing in our grammar |
| `leptos_workers` | Web-worker abstraction | 0.3.0, 2025-02-20 | 2025-11-24 | — | 0.8 era | Off-thread work |

Sources: `https://crates.io/api/v1/crates/<name>` and `https://api.github.com/repos/<owner>/<repo>` for each row, all retrieved 2026-09-03.

Three absences in this table matter more than any entry in it.
There is no Tauri bridge crate anywhere in Components; the only Tauri material on the list is the `tauri-leptos-ssr` starter template (last push 2026-05-29) and two applications, so the list neither surfaces nor contradicts the fit note's finding that `tauri-sys` returns 404 on crates.io.
There is no form-validation crate; Leptodon's `form_input` and `input_group/presets` and Rust/UI's `auto_form` and `use_form` are the whole surface, and neither is a validation engine.
There is no tree widget outside Thaw, no virtualisation library outside `leptos-struct-table`, and no upload component with a progress path.

## 2. Leptodon, the one entry that changes an estimate

Leptodon is a crate dependency rather than a copy-paste registry, Apache-2.0, published by openanalytics, added to the list on 2026-03-11 (<https://github.com/openanalytics/leptodon>, <https://crates.io/api/v1/crates/leptodon>, retrieved 2026-09-03).
It has released 1.0.0 on 2026-03-10, 1.1.0 on 2026-03-31, 1.2.0 on 2026-04-16, 1.3.0 on 2026-05-04, 1.4.0 on 2026-06-03, 1.5.0 on 2026-07-17 and 1.6.0 on 2026-08-21, a roughly monthly cadence and the only such cadence in the Leptos component field.
Its `rust-toolchain.toml` reads `channel = "stable"`, against Rust/UI's `channel = "nightly"` with `features = ["nightly"]` on both `leptos` and `leptos_router`, which answers the coverage note's Q1 directly: a stable-toolchain Leptos component library now exists.
`grep` for `<script` across `leptodon/src` returns nothing, against the twelve script-injecting modules the coverage note found in Rust/UI.
The repository carries a nix flake, a `deny.toml`, a `rust-toolchain.toml`, `cargo-nextest` unit tests and a Playwright end-to-end suite under `overview/end2end`, which is more enforcement apparatus than any other Leptos component library surveyed.

Its `tag_picker` is the finding.
`leptodon/src/tag_picker/mod.rs` is 538 lines and takes `max_number: Signal<usize>`, refusing a toggle once the cap is reached (lines 106, 484, 492 to 495).
Selected tags render as chips inside the trigger with a per-chip remove control (lines 285 to 300).
The search input handles `ArrowUp`, `ArrowDown`, `Enter`, `Escape` and `Tab` in Rust (lines 330 to 356) and carries `role="combobox"`, and option filtering runs through `nucleo-matcher` for fuzzy search.
That is the coverage note's "capped multi-select with chips and a visible count", which it priced at four to six Leptos engineer-days as a Build with no library anywhere.

Its `modal` is the second finding: `leptodon/src/modal/mod.rs` carries `role="dialog"`, `aria-modal=true`, an `aria-label` bound to the title, an Escape handler, and a focus trap built from two `<span tabindex="0" aria-hidden="true" on:focus=...>` sentinels (lines 107 to 149).
The source comment on the trap reads "Somehow this hack makes focus work..", which is worth recording as a confidence signal, but the mechanism is correct and it is in Rust.

Against that, four limits.
Accessibility across the library is thin: the whole of `leptodon/src` contains five `aria-hidden`, four `aria-label`, three `aria-current-page`, three `aria-controls`, two `role="combobox"`, two `role="alert"`, two `aria-labelledby`, two `aria-expanded`, one each of `role="progressbar"`, `role="list"`, `role="img"`, `role="group"`, `role="dialog"`, `aria-modal`, `aria-disabled` and `aria-current`, across roughly thirty-five components.
`tabs/mod.rs` and `popover/mod.rs` contain no `role` or `aria` attribute at all, so the WAI-ARIA tabs pattern is unimplemented and would have to be retrofitted.
The chip remove control in `tag_picker` is bound to `on:click` with no keyboard equivalent, so a capped selection cannot be reduced from the keyboard.
Its published manifest pins `web-sys =0.3.103` and `wasm-bindgen =0.2.126` and `reactive_stores =0.4.3` exactly, the last with the source comment "Bumping reactive_stores causes hydration errors", and exact pins in a founder-gated workspace `Cargo.toml` collide directly with the `wasm-bindgen` CLI-to-crate version coupling the fit note documents in §3.

Concentration risk applies to it as much as to the libraries that preceded it.
It is one company's library with one open issue, and the coverage note's warning was written after Leptonic, RustForWeb/radix and RustForWeb/shadcn-ui died in sequence.

## 3. Gap by gap

Each row is a gap the two notes identified, with what the list does to it.

| Gap | List entry | Verdict | Evidence |
|---|---|---|---|
| Rich-text or WYSIWYG editor for TPT HTML descriptions | Papelito | Open | Repository archived 2023-05-06, `Cargo.toml` depends on `leptos` by git branch with no version, 6 downloads in 90 days; nothing else on the list is an editor |
| Capped multi-select combobox with chips | Leptodon `tag_picker` | Closed | `max_number` cap, chips in trigger, arrow keys and `role="combobox"` in Rust; keyboard chip removal still missing |
| Searchable four-level standards tree, multi-select | none; Thaw `tree` only | Open | Thaw has 0 commits since 2026-06-05, last commit 2026-05-07; Leptodon has no tree module; `leptos-use` has no virtual-list primitive |
| Sortable, filterable, virtualised data table with row selection | `leptos-struct-table` 0.19.1 | Closed, as the coverage note already found | 21 commits since 2026-06-05, last 2026-09-02; Leptodon depends on it at `^0.19.0`, so the two compose |
| File uploader with per-file progress | Leptodon `input/upload.rs`, `leptos-use` `use_drop_zone` | Partially closed | `FileUpload` gives a dropzone, click-to-pick, `multiple` and an accept filter in 107 lines, and contains no progress path; the coverage note's `XmlHttpRequest`-through-`wasm-bindgen` requirement stands |
| Tabs with keyboard and ARIA | Leptodon `tabs`, `leptix` | Open | Leptodon `tabs/mod.rs` has no `role` or `aria` attribute; `leptix_primitives` has a `tabs.rs` and a `roving_focus.rs` but pins `leptos = "0.6"` and has not been committed to since 2024-11-22 |
| Dialogs with keyboard and ARIA | Leptodon `modal` | Closed | `role="dialog"`, `aria-modal`, Escape, sentinel focus trap, all in Rust |
| Toasts | Leptodon `toast`, `leptos_toaster` 0.3.0, `leptoaster` 0.2.3 | Closed | Three implementations; `leptos_toaster` released 2026-07-15 |
| Data fetching and caching comparable to TanStack Query | `leptos-fetch` | Open, and wider than recorded | 0 commits since 2026-06-05, last commit 2026-03-02, six months; the list dropped `leptos_query` for it on 2025-05-04 and has not revisited |
| Tauri v2 bridge crate published on crates.io | none | Open | The list contains no bridge crate; `tauri-sys` remains absent from crates.io per the fit note §3 |
| Charts | `leptos-chartistry` 0.2.3 | Closed but stalling | 0 commits since 2026-06-05, last commit 2026-01-23; adequate for a 164-line analytics page |
| Forms with validation | none | Open | No validation crate on the list; Leptodon `form_input` and Rust/UI `auto_form` are input components |
| Internationalisation | `leptos_i18n` 0.6.2, `leptos-fluent` 0.3.1 | Closed | Two options; `leptos_i18n` default branch last committed 2026-06-04, `leptos-fluent` 2026-04-24 |
| Testing tools | none | Open | Tools section holds a formatter, a formatter action and three editor plugins |
| Component library without a nightly pin and without injected JavaScript | Leptodon | Closed | `channel = "stable"`, zero `<script>` occurrences in `leptodon/src`, `leptos = "0.8.20"` |

Five of the coverage note's "five hard components" therefore become one closed, one partially closed and three open, and the sixth apparent gap stays closed by `leptos-struct-table` as the note already recorded.

## 4. The maintenance claims, retested

The Leptos status finding survives without amendment and is slightly reinforced.
`leptos-rs/leptos` took 71 commits on its default branch between 2026-06-05 and 2026-09-03, of which 40 are by `gbj`, the author, and the next highest contributor is `sabify` at six; no second maintainer is publicly committed (<https://api.github.com/repos/leptos-rs/leptos/commits>, retrieved 2026-09-03).
The release picture is unchanged: 0.8.20 published 2026-06-25 is the newest stable and 0.9.0-beta published 2026-07-18 is the newest anything, with nothing since (<https://crates.io/api/v1/crates/leptos>, retrieved 2026-09-03).
"Lightly maintained" is therefore accurate as a governance statement rather than as an activity statement, and the fit note's revisit condition — 0.9 stable plus a second committed maintainer — is not met.

On the staleness claims the list splits three ways.
It confirms the `leptos_query` finding and acts on it: commit "Replace unmaintained leptos_query with leptos-fetch (#69)" landed on 2025-05-04, so `leptos_query` is not on the list at all, and the replacement it names is the same one the fit note names.
It confirms the Thaw finding by inaction: Thaw is still listed, its last commit is 2026-05-07 and it has taken 0 commits since 2026-06-05.
It confirms the Leptonic finding by omission: Leptonic is not on the list.

On the archived RustForWeb ports the list is half right and the half that is wrong matters.
It removed RustForWeb/radix on 2026-03-29 in a commit reading "remove unmaintained RustForWeb/radix (closes #87)".
It still lists "Rust shadcn/ui" pointing at `shadcn-ui.rustforweb.org`, whose repository has `archived: true` and a last push of 2026-02-02 (<https://api.github.com/repos/RustForWeb/shadcn-ui>, retrieved 2026-09-03).
The two RustForWeb projects the list keeps that are genuinely alive are Rust Floating UI, 46 commits since 2026-06-05, and Rust Lucide, `lucide-leptos` 3.38.0 published 2026-09-02.

The list surfaces an actively maintained alternative for exactly one of the four stale things the notes named.
For Thaw and for the archived shadcn port, the alternative is Leptodon, added 2026-03-11, releasing monthly through 1.6.0 on 2026-08-21 — with the caveat that Leptodon has no tree and no `upload` progress, so it does not replace the two Thaw modules the coverage note wanted Thaw for.
For `leptos_query` the alternative the list names is `leptos-fetch`, which is now itself six months without a commit.
For Leptonic there is nothing to replace, because the list already dropped it.

Two entries on the list are dead and still carried, which is the curation quality the rest of this note should be read against.
`leptix`, added 2024-05-13 and described as "Accessible and unstyled components for Leptos", links a repository whose last commit is 2024-11-22 and whose crate `leptix_primitives` 0.2.3 pins `leptos = "0.6"`; separately, the bare `leptix` name on crates.io belongs to an unrelated two-component crate by a different author whose own README reads "Somehow alive. May die at any moment. No guarantees." (<https://crates.io/api/v1/crates/leptix>, <https://github.com/nishujangra/leptix>, retrieved 2026-09-03).
Papelito, added 2023-05-05, was archived on 2023-05-06.

The list's own commit log is the summary statistic.
Its README changed on 2026-03-11 (add Leptodon), 2026-03-24 (add `leptos-content-collection`), 2026-03-29 (remove RustForWeb/radix), 2026-04-02, 2026-04-12, 2026-05-20, 2026-07-03, 2026-07-13 and 2026-08-04.
Every change after 2026-03-29 added a website, a template or an SDK; two libraries were added and one removed in twelve months.

## 5. Prior art on the list, for the Tauri question

Two of the thirty website entries are Leptos-plus-Tauri applications and are worth naming because the fit note's §3 is otherwise a documentation-only assessment.
Origa, a Japanese-learning application claiming iOS, Android, Web, Windows and macOS from Leptos plus Tauri, was pushed 2026-09-02 and has 19 open issues (<https://github.com/yurvon-screamo/origa>, retrieved 2026-09-03).
RustyTube, a YouTube client for desktop and web built with Leptos and Tauri, was last pushed 2024-09-19 (<https://github.com/opensourcecheemsburgers/RustyTube>, retrieved 2026-09-03).
Neither is a data-dense form-heavy dashboard, and neither publishes an IPC crate, so they establish that the combination ships and nothing more.

## 6. Verdict

Nothing on the list changes the "do not switch" conclusion.

The best combination the list supports is Leptodon 1.6.0 for forms, `tag_picker`, modal and the component shell; `leptos-struct-table` 0.19.1 for the activity log and any record table; `leptos-use` 0.19.2 for `use_drop_zone`, `use_infinite_scroll` and `use_draggable`; Rust Floating UI 0.6.0 for popover and combobox anchoring; `leptos-chartistry` 0.2.3 for the analytics page; and `leptos_i18n` 0.6.2 for internationalisation.
That combination runs on stable Rust with no injected JavaScript, which is a materially better position than the coverage note found, and it is the first time the Leptos ecosystem has offered one.

Its residual gaps are five and they are the expensive ones.
The four-level searchable standards tree remains a Build with no library anywhere, at the coverage note's 8 to 12 engineer-days.
Upload progress remains a hand-written `XmlHttpRequest` binding through `wasm-bindgen`, at 3 to 5 days on top of Leptodon's dropzone.
The rich-text description editor remains a hand-written FFI surface to TipTap or ProseMirror, at 3 to 5 days, and the list's only editor is archived.
Tabs and popover need an ARIA and roving-focus retrofit that Leptodon does not supply and `leptix` cannot supply without a two-major port, at 2 to 4 days.
The data-fetching layer standing in for 58 `createQuery` and 11 `createMutation` call sites rests on a crate with no commit since 2026-03-02, or is hand-rolled on `Resource` and `Action`, at 5 to 8 days either way.

The arithmetic is the answer.
The coverage note priced the component work at 29 to 45 engineer-days — 25 to 38 for the five hard components, 3 to 5 for the rich-text binding, 1 to 2 for a split pane.
Leptodon removes roughly 3 to 5 of those days by closing the capped multi-select and the dialog, and adds back an exact-version-pin conflict with the founder-gated workspace manifest.
Against the fit note's 90-to-140-day migration, that is a movement of under five per cent, and it does not touch the three findings the fit note actually decided on: the framework is lightly maintained by one person, "native" is false inside a webview, and the correctness prize is available today by compiling `tam-domain` and `tam-taxonomy` to `wasm32-unknown-unknown` behind the existing Svelte console for 5 to 10 days.

If the founder's interest in the list is really an interest in Leptodon, the honest framing is that a fourth Leptos component library has appeared, it is better engineered than its three dead predecessors, it is six months old, and betting a launch-blocking frontend on a 1.6.0 release from a single vendor is the same bet that Leptonic, RustForWeb/radix and RustForWeb/shadcn-ui lost.

## 7. Open questions

Q1. Does Leptodon's existence reopen the coverage note's Q2 — adopt Rust/UI and rewrite twelve script-injected modules, versus daisyUI and write everything — with a third option of Leptodon plus hand-written gaps, and if so does that need answering before or after the framework decision?

Q2. Leptodon pins `web-sys =0.3.103`, `wasm-bindgen =0.2.126` and `reactive_stores =0.4.3` exactly. Is an exact third-party pin on `wasm-bindgen` acceptable in a founder-gated workspace, given the fit note §3 records that the CLI and crate schema versions must match exactly and that the failure surfaces specifically when a project moves into a Cargo workspace?

Q3. `leptos-fetch` has had no commit since 2026-03-02. Does that change the answer to the fit note's Q2 — whether "Rust domain rules in the browser via wasm inside the Svelte console" satisfies the intent — by making the Leptos path strictly worse than when that question was asked?

Q4. Is the standards tree still the largest single component cost regardless of framework, and if so should it be specified and priced independently before any framework decision, since it is 8 to 12 days in Leptos and 6 to 9 in Svelte and neither ecosystem supplies it?

## 8. Unverified

U1. Leptodon's component behaviour beyond `tag_picker`, `modal`, `dialog`, `tabs`, `select` and `input/upload`. Six of roughly thirty-five modules were read; the remainder are assessed by module name and by the repository-wide `aria` and `role` counts.

U2. Whether Leptodon's Playwright suite under `overview/end2end` covers keyboard paths. The directory was seen in the repository tree; no test was read.

U3. Leptodon's rendered visual identity and how far its Flowbite-derived styling can be retargeted to cream and terracotta. `https://leptodon.dev` was not fetched.

U4. Whether Leptodon's `tag_picker` announces the cap to assistive technology. The cap is enforced in `toggle_tag` with a `debug_log!`; no `aria-live` region was found, but the popover content was not read line by line.

U5. Whether openanalytics maintains Leptodon as a product or as internal infrastructure released publicly. The organisation's other repositories were not examined and no roadmap or support statement was located.

U6. `leptos_i18n`'s activity picture. Its GitHub `pushed_at` is 2026-08-31 while its default branch's last commit is 2026-06-04, so work exists on a non-default branch that was not inspected.

U7. Whether `leptos-fetch`'s six-month gap is abandonment or completeness. No issue tracker or maintainer statement was read; the crate has one open issue.

U8. `leptix_primitives`' portability to Leptos 0.8. It pins `leptos = "0.6"` and `leptos-use = "0.13"`; the size of the port was not estimated and its 23 primitives include no combobox, dialog, select, popover or tree, so a port would not close any of the three remaining hard gaps.

U9. Whether any entry on the list has an accessibility statement or a published conformance report. None was found for Leptodon, Rust/UI, `leptos-struct-table` or `leptos_toaster`; only `leptix`'s one-line "Accessible components for Leptos" and the coverage note's already-recorded unaudited cloud-shuttle claim exist, and cloud-shuttle is not on this list.

U10. The engineer-day adjustments in section 6 are this note's judgement calibrated on the coverage note's own table, not measurements, and they assume the coverage note's estimating basis is sound.
