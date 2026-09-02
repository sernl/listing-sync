# Running the core in the browser

D28's WebAssembly step: which rules move out of TypeScript into the compiled core, what crosses the boundary, and what deliberately stays where it is.

- date: 2026-09-03
- method: read `crates/tam-domain/src/product/`, `crates/tam-taxonomy/`, `crates/tam-api/src/product/check.rs` and `web/src/lib/tpt-form.ts`; every size, version and verdict below was measured on this machine rather than estimated
- status: built and landed; `just purity`, `just check-portable`, `svelte-check` and 318 web tests are green, and the two decisions section 7 was waiting on were answered before the code was written

## 1. Why

`web/src/lib/tpt-form.ts` reproduces the create form's refusals in TypeScript so a seller reads a message as they type.
Its own header says the rules are a mirror and that `POST /v1/authoring/check` is the authority.
A mirror is two implementations of one rule, and the cost of two is that they can disagree without either side being edited: a rule changed in Rust leaves the browser saying the old thing until somebody remembers the TypeScript.

D28 removes the second implementation rather than documenting it.
The core compiles to `wasm32-unknown-unknown` today — `just check-portable` has proved that for six crates since the client-surface work — so the browser can call the same function the server calls instead of a restatement of it.

## 2. What runs in the browser

Four exported functions, all pure, none performing I/O.

`check_draft(draft_json, caps_json)` answers what the form refuses.
It deserialises the body of `POST /v1/authoring/check` and returns that endpoint's own `CheckView`, byte for byte, because it calls the same function the handler calls.
`caps_json` is null on every production path, which reads the compiled-in capture and so cannot disagree with the server; a caller that states caps is a test holding them fixed.

`selection_caps()` returns the measured picker caps as JSON, read from the same committed capture the server reads.
Measured here: `{"grades":4,"subject_areas":null,"tags":6,"formats":3,"thumbnails":4}`.
A null cap is unmeasured, never unlimited, exactly as the model reads it, so the counter beside a picker says "2 chosen" rather than inventing a ceiling.

`project_preview(draft_json, marketplace)` answers what one marketplace will carry for the three fields a draft holds text for.
It reads the compiled-in field registry, so it states a real cap and a real truncation loss where one is on file and says nothing where nothing is measured.
Etsy declares a 140-codepoint title cap and marks title, description, price and taxonomy required; TPT declares a 45,000 UTF-16 description cap; Tes declares `CanonicalFields::UNRECORDED` and so yields three rows with no cap and no loss.
Its `undecided_axes` names the equivalence axes the browser cannot decide, so the page says so rather than implying the list is complete.

`core_version()` reports the crate version, so a stale bundle is identifiable rather than merely suspected.

## 3. The boundary

JSON strings in both directions, and `serde-wasm-bindgen` is deliberately not used.
A string has one encoding to agree on, and the strings crossing here are the same ones the HTTP endpoint already carries, so the browser and the server read one wire format rather than two.
No npm dependency is added: Vite loads the `.wasm` as an asset through a `?url` import and `wasm-bindgen --target web` initialises from that URL.

Nothing panics across the boundary.
Every function returns a JSON string, and a failure is `{"error":{"kind":"...","message":"..."}}` where `kind` is one of `malformed_draft`, `malformed_caps`, `unknown_marketplace` or `encode_failed`.
The error is a distinct top-level key rather than a verdict variant, because a verdict with no refusals means the draft is submittable and a boundary failure rendered as one would read as approval.
A trap would poison the module instance and take every later call with it, which is why the encoder's own unreachable failure path is written out rather than left to `expect`.

## 4. Where the shared rules live

The rules cannot be called from both sides while they live in `tam-api`, which carries axum, `tam-storage` and sqlx and does not reach `wasm32`.
The pure half of `crates/tam-api/src/product/check.rs` — the draft and verdict wire types, `read_draft`, `price_of`, `refusal_of`, `advisory_of` and `verdict`, together with `selection_caps` and the capture it reads — moves to a new portable crate, and `tam-api` keeps the axum handler, `record_of` and the `APIError` conversions that genuinely need the server.

That lift is what makes the equivalence structural rather than aspirational.
Every alternative leaves the browser running a second copy of the rules, which is the duplication D28 exists to remove.
The move is verbatim: only the import header changes, plus `DraftHead` and `into_draft` widening from `pub(crate)` to `pub`, and one addition, `verdict_with(draft, caps)`, so a test can hold the caps fixed while `verdict(draft)` keeps calling `selection_caps()` as it does today.

## 5. The equivalence proof

`crates/tam-core-wasm/fixtures/drafts.json` holds seventeen drafts chosen to exercise both outcomes and each refusal branch.
`fixtures/verdicts.json` records what the native path — the one the server runs — decides for each, generated by the `verdict-fixtures` bin in the shape `tam-api` already uses for `vocab.ts`.

Two tests read that one file from opposite sides.
A Rust integration test asserts the recorded verdicts are what `verdict` currently reaches, so a rule changed in Rust fails until the file is regenerated with `just web-wasm-fixtures`.
A vitest test loads the compiled module under node and asserts `check_draft` returns the same verdict for the same draft, so a browser that drifted fails too.
Neither side can move without one of the two going red, and a rule changed on purpose is a visible diff in a committed file rather than a silent divergence.

Measured: seventeen of seventeen drafts agree, and both tests assert the set is non-trivial — that it contains a submittable draft and a refused one — so a comparison that passed vacuously would itself fail.

The strongest evidence that the port preserved behaviour is what did not change.
The eight refusal assertions in `web/src/lib/tpt-form.test.ts` were written against the TypeScript implementation and still pass unedited against the compiled core, including the exact strings `Grade Level takes up to 4, and 5 are chosen.` and `$0.95`.

## 6. Three rules moved into the server

The TypeScript held four refusals the Rust model did not.
Three of them are TPT's own submission rules from the field catalogue in `docs/research/rethink/tpt-product-model.md`, and they are now refused by the shared verdict rather than by a client.
This is a change in what the server refuses, recorded here rather than left to be discovered: a draft the API previously called submittable may now be refused.

A missing `payload_hash` is refused.
This was a defect as well as a gap: `read_draft` returned early through its `let ... else` binding without recording anything, so a titled draft with no downloadable file came back `submittable: true` and only the client's own refusal was catching it.
The `no_payload` fixture pins the fixed behaviour.

A paid draft priced below TPT's own floor is refused, and so is one with no price at all.
The floor is `min_price: 0.95` from the create page's `var cfg` bootstrap, read from the committed capture by `tam_authoring::min_price_minor_units` rather than typed into the source.
The two cases carry different messages, because a blank field and an underpriced one are different things to a seller, but they are one rule: a paid listing states a price at or above the floor.

A paid draft with no tax code is refused, naming the control.
This applies D7 rather than departing from it.
TPT's Terms of Service make the seller answerable for designating the code, so requiring them to choose is the opposite of choosing one for them, and it is the same stance the copyright attestation already takes.
Free resources are untaxed, so neither price rule nor the tax-code rule applies to one.

One rule stays in TypeScript.
"Choose at least one marketplace" has no counterpart in the domain at all, because the model describes a product while the set of marketplaces to publish it to is a decision about this listing.

The counters, the picker toggles, the facet search, the grade grid, the price parsing and the two request bodies also stay.
They are presentation and wire assembly rather than rules, and the core answers none of them.
What is gone from `tpt-form.ts` is every refusal it used to state, plus `PICKER_LABEL` and `REQUIRED_PICKERS`, whose content is `Picker::label` and the three required pickers in `tam-domain`.

## 7. What is still owed

The equivalence relation behind the axis rows.
`project_preview` decides field rows from the compiled-in registry and names the axes it does not decide in `undecided_axes`, because `project_listing` reads terms, edges, no-counterpart records, standing rules and settled elections, all of which live in Postgres.
No endpoint serves that slice: `GET /v1/vocabulary/{inventory}` serves the registry's field table and `GET /v1/mappings` serves mapping heads with recorded losses, and neither answers "what will this listing's subject be on Tes".
It is the same gap `docs/notes/design/creation-flow.md` already names, to be filled by the per-organisation override slice.

One duplicate reading of the price floor.
`tam_authoring::min_price_minor_units` and the `floor_minor_units` helper in `crates/tam-api/src/product/mod.rs` read the same `minPrice` from the same capture.
Folding the second into the first is a one-line change to a file outside the scope this work was granted, so it is recorded rather than taken.

## 8. Build and size

`cargo build --target wasm32-unknown-unknown --release --lib -p tam-core-wasm`, then `wasm-bindgen --target web` into `web/src/lib/core/generated/`, both wrapped by `just web-wasm`.
The `--lib` is load-bearing: the fixture generator is a bin in the same crate and does not cross-compile.

The generated files are gitignored and built by the recipe, which `web-check` and `web-dev` both run first.
Committing them would put a build artefact in review diffs and let a stale one ship, and the dev shell now carries the CLI so there is no machine where the recipe cannot run.

The crate pins `wasm-bindgen = "=0.2.121"`, because wasm-bindgen refuses a CLI whose version differs from the crate's and 0.2.121 is what the nixpkgs revision `flake.lock` pins carries.
`Cargo.lock` previously resolved 0.2.127 transitively through Tauri's tree; the pin unifies that node down, since cargo unifies semver-compatible versions rather than carrying both.
Five entries move, all of them wasm32-gated in Tauri's tree and never compiled for the desktop.

Measured on the shipped artefact: 449,961 bytes raw, 118,076 gzipped, with 9,758 bytes of JavaScript glue that gzips to 2,832.
That is 39 percent of the 300 KB gzipped budget, so no split is proposed.
No profile change was needed either, and none was available: cargo reads `[profile]` only at a workspace root, so a size-tuned profile would have been a workspace-wide change.
`wasm-opt` is not run and is not in the dev shell; the figure above is without it.

The Vite production build emits the module as a hashed immutable asset referenced from the create page's own chunk, so the `?url` import needs no configuration and no npm dependency was added.

Both crates compile clean under the workspace lint set on the native and the `wasm32` target, including `unsafe_code = "forbid"`, which wasm-bindgen 0.2.121 on edition 2021 does not trip.
Two `#[expect]` attributes are carried, each naming its reason: `needless_pass_by_value` on `check_draft`, because wasm-bindgen implements `OptionFromWasmAbi` for `String` and not for `&str` so an optional string argument cannot be taken by reference; and `cast_possible_truncation` on the price-floor conversion, guarded by the range check above it.
