# TanStack and Astro as frontend alternatives

Whether to move the Teachouse console off SvelteKit onto TanStack Start, adopt more TanStack libraries inside Svelte, or use Astro — for the app, for the teachouse.io landing page, or both.

- date: 2026-09-03
- method: read-only inspection of this working tree (no jj or git command run, no dependency added, no build started) plus read-only retrieval of public primary sources; every external claim carries its URL and the retrieval date 2026-09-03, and every registry figure is a live query made today
- measurements: taken from `web/src` and the already-built `web/build` directory dated 2026-08-31; no build was run to produce them
- inherits: `docs/research/rethink/leptos-frontend-fit.md` (2026-09-03) for the Leptos verdict and `docs/research/rethink/client-surfaces-and-cross-compile.md` (2026-09-02) for Tauri and cross-compilation, neither of which is re-derived here

## 1. Executive summary

TanStack is not one thing, and the two readings of "switch to TanStack" have opposite answers: adopting more TanStack libraries inside Svelte is cheap and partly worth doing, while TanStack Start is a React framework we cannot adopt without rewriting the client.
There is no Svelte adapter for TanStack Router and therefore none for Start: the `TanStack/router` repository ships `react-*`, `solid-*` and `vue-*` packages and no Svelte package, and `@tanstack/svelte-router` and `@tanstack/svelte-start` both return 404 from the npm registry.
Start is honest about its own status — "TanStack Start is currently in the **Release Candidate** stage" and "a full-stack React framework" — so choosing it is choosing React, not choosing a router.
The Svelte adapters that do exist are real and current: `@tanstack/svelte-table` 9.2.4 and `@tanstack/svelte-form` 1.33.5 both declare `svelte: ^5.0.0` and both published within the last month, alongside the `@tanstack/svelte-query` 6.1.48 we already depend on.
Their weakness is population, not staleness: `@tanstack/svelte-query` draws 175,553 weekly downloads against `@tanstack/react-query`'s 65,663,515, a ratio of about one to 374, which is the bus-factor number the founder should actually weigh.
Migrating 12,330 lines of Svelte and TypeScript to React plus Start is 45 to 80 engineer-days on the model in section 4, against a console redesigned to an approved identity two days ago and a launch gated on a landing page, not on a framework.
The bundle argument runs against React and it is measurable: our entire first-load payload is 80,083 bytes gzipped today, while React 19.2.8 plus its DOM client is 61,953 bytes gzipped before a line of Teachouse code and TanStack Router adds a further 39,345.
Where React genuinely wins is accessible primitives for the components we have not built yet — `react-aria-components` ships Tree, TokenField, TagGroup, Table, DropZone and FileTrigger, and neither `bits-ui` nor Radix ships any of those four hardest ones — but we currently consume no UI component library at all, so that advantage is prospective rather than forfeited.
Astro is the wrong host for a logged-in dashboard by its own documentation, which places "logged-in admin dashboards" among the things other frameworks "excel at", and it has no client-side router beyond a body-swapping view-transition shim.
Verdict: stay on SvelteKit for the app, adopt `@tanstack/svelte-table` when the first real data table lands, and build the teachouse.io landing page as a separate Astro 7 site on Cloudflare Pages rather than as a route inside a SPA whose root layout sets `ssr = false`.

## 2. What "TanStack" actually means

TanStack is a family of headless, framework-agnostic core libraries with per-framework adapters, plus one full-stack meta-framework.
The distinction matters because the family is portable and the meta-framework is not.

We already use one of them.
`web/package.json` pins `@tanstack/svelte-query` at `^6.1.48`; nineteen files import it, and the tree contains 58 `createQuery` occurrences and 11 `createMutation` occurrences.
That is our largest external frontend dependency after SvelteKit itself.

The adapter picture today, from the npm registry and the upstream monorepos, all retrieved 2026-09-03:

| Library | Svelte package | Latest | Published | Svelte 5 peer | Weekly downloads | React counterpart downloads |
|---|---|---|---|---|---|---|
| Query | `@tanstack/svelte-query` | 6.1.48 | 2026-08-27 | yes (runes migration documented) | 175,553 | 65,663,515 |
| Table | `@tanstack/svelte-table` | 9.2.4 | 2026-08-28 | `svelte: ^5.0.0` | 58,351 | 20,010,461 |
| Form | `@tanstack/svelte-form` | 1.33.5 | 2026-08-11 | `svelte: ^5.0.0` | 33,717 | 2,891,125 |
| Virtual | `@tanstack/svelte-virtual` | 3.13.36 | 2026-08-18 | `^3.48 \|\| ^4 \|\| ^5` | 75,565 | not compared |
| Store | `@tanstack/svelte-store` | 0.12.1 | 2026-08-05 | transitive dep of svelte-table | not compared | not compared |
| Router | none | — | — | — | — | 1.170.32 |
| Start | none | — | — | — | — | 1.168.49 |

Sources: `https://registry.npmjs.org/-/package/<name>/dist-tags` and `https://registry.npmjs.org/<name>` for versions and publish times; `https://api.npmjs.org/downloads/point/last-week/<name>` for downloads; all retrieved 2026-09-03.

The absence of a Svelte router adapter is structural, not accidental.
`https://api.github.com/repos/TanStack/router/contents/packages` (retrieved 2026-09-03) lists `react-router`, `solid-router`, `vue-router`, `react-start`, `solid-start`, `vue-start` and their satellites, and contains no Svelte package of any kind.
`https://registry.npmjs.org/@tanstack%2fsvelte-router` and `https://registry.npmjs.org/@tanstack%2fsvelte-start` both return `{"error":"Not found"}` (retrieved 2026-09-03).
By contrast `https://api.github.com/repos/TanStack/table/contents/packages`, `.../form/contents/packages` and `.../virtual/contents/packages` each list a `svelte-*` package (all retrieved 2026-09-03).

So option (a), adopting more TanStack libraries inside Svelte, is available for Table, Form and Virtual and unavailable for Router and Start.
Option (b), TanStack Start, is a React migration wearing a router's name.

### Option (a) assessed on its own

`@tanstack/svelte-table` is the one worth adopting, and the trigger is a real data table rather than a date.
We render nine plain `<table>` elements today with no sorting, no column model and no virtualisation, and the moment one of them needs column sorting plus pagination plus persisted column state, hand-rolling it costs more than the adapter does.
Its Svelte-side risk is contained: `@tanstack/table-core` carries the logic, `svelte-table` is a thin binding over `@tanstack/svelte-store`, and if the binding stalls, the core remains usable with a hand-written wrapper.
The comparable Svelte-native option, `svelte-headless-table` 0.18.3, last published 2024-10-28 at 23,357 weekly downloads, is the weaker bet on both recency and population.

`@tanstack/svelte-form` is a maybe and not now.
Our largest screen, `src/routes/listings/new/+page.svelte` at 507 lines, is exactly the sort of form it targets, but that screen was redesigned two days ago and works; adopting a form library into finished, tested screens buys structure we have already paid for by hand.
`@tanstack/svelte-virtual` is a no until a list actually exceeds a few hundred rows.

## 3. TanStack Start honestly

Version and status.
`@tanstack/react-start` is at 1.168.49, published 2026-08-22, with `@tanstack/react-router` at 1.170.32, published 2026-08-22 (npm registry, retrieved 2026-09-03).
The upstream overview is unambiguous about maturity: "TanStack Start is currently in the **Release Candidate** stage! This means it is considered feature-complete and its API is considered stable. **This does not mean it is bug-free or without issues**" (`https://raw.githubusercontent.com/TanStack/router/main/docs/start/framework/react/overview.md`, retrieved 2026-09-03).
The same page states plainly that it "is a full-stack React framework powered by TanStack Router".
There is a Solid adapter at `@tanstack/solid-start` 1.168.47 and a Vue one in the repository, so "React-only" is not literally true, but Solid and Vue are as much a rewrite for us as React is and have thinner ecosystems, so they do not change the decision.

Rendering modes.
Start documents full SSR, `ssr: 'data-only'`, and `ssr: false` per route, plus a whole-application SPA mode (`https://tanstack.com/start/latest`, retrieved 2026-09-03).
SPA mode is the mode that matters to us, and it does what a Tauri webview needs: enabling `spa: { enabled: true }` in the Start plugin makes the build "prerender your application's **root route only**", write the result to "a static HTML page called `/_shell.html`", and configure "default rewrites ... to redirect all 404 requests to the SPA mode shell" (`https://raw.githubusercontent.com/TanStack/router/main/docs/start/framework/react/guide/spa-mode.md`, retrieved 2026-09-03).
The same page names the benefit in terms that describe our deployment exactly: "**Easier to deploy** - A CDN that can serve static assets is all you need."
So yes, Start can produce a static SPA bundle that `tam-server --ui-dir` and a Tauri webview would both consume; this is not a blocker.

Deployment targets.
Start "is **designed to work with any hosting provider**" and documents Cloudflare Workers, Netlify, Railway, Nitro, Vercel, Node, Bun and Appwrite Sites (`https://raw.githubusercontent.com/TanStack/router/main/docs/start/framework/react/guide/hosting.md`, retrieved 2026-09-03).
None of this is a gain for us, because our server is `tam-api` in Rust and would remain so; we would use Start's client build and none of its server.
That is the same shape of mismatch the Leptos note found: we would adopt the framework's least differentiated half.

What Start adds that SvelteKit lacks, stated fairly.
The genuine one is typed, validated search params: TanStack Router "provides convenient APIs for validating and typing search params" through `validateSearch`, integrating Zod, Valibot, ArkType and Effect/Schema via Standard Schema, and it presents this as a differentiator over raw `URLSearchParams` (`https://tanstack.com/router/latest/docs/framework/react/guide/search-params`, retrieved 2026-09-03).
Our list screens are filter-heavy and would benefit.
The second is per-route selective SSR, which we do not want, since our root layout already sets `ssr = false`.
The third, server functions, is a capability SvelteKit now also has as remote functions — `query`, `query.batch`, `query.live`, `form`, `command` and `prerender` — but SvelteKit's are explicitly "currently experimental, meaning it is likely to contain bugs and is subject to change without notice" and gated behind `kit.experimental.remoteFunctions` (`https://svelte.dev/docs/kit/remote-functions`, retrieved 2026-09-03).
We use neither, because our server is Rust.
Netting it out: for a dashboard whose backend is not JavaScript, Start's advantage over SvelteKit reduces to typed search params and a router with a better data-loading story, which is not a 45-to-80-day purchase.

Bundle size, measured.
Our current first load is 31 entry chunks referenced by `web/build/index.html` totalling 80,083 bytes gzipped, and the whole client — 81 files, 620 KB on disk — is 132,172 bytes gzipped in JS and CSS together (measured 2026-09-03 against the build dated 2026-08-31).
React's floor, measured the same way from minified ESM builds: `react@19.2.8` is 3,826 bytes gzipped and `react-dom@19.2.8/client` is 58,127, so 61,953 bytes before any application code (`https://esm.sh/react@19.2.8/es2022/react.mjs` and `https://esm.sh/react-dom@19.2.8/es2022/client.bundle.mjs`, retrieved 2026-09-03).
`@tanstack/react-router` 1.170.32 is a further 39,345 bytes gzipped (`https://bundlephobia.com/api/size?package=@tanstack/react-router`, retrieved 2026-09-03), against `svelte` 5.57.0 at 13,609.
Framework floor before our code would therefore be roughly 101 KB gzipped versus our entire 80 KB payload today.
Caveat stated honestly: the router figure is a whole-package measurement and real applications tree-shake it, so treat 101 KB as an upper bound on the floor rather than a predicted payload; the direction, though, is not in doubt, and inside a Tauri webview on a mid-range Android device that direction is the one that costs.

Hiring and agent fluency.
The React ecosystem is larger by roughly 29 times on framework downloads — `react` 171,637,376 weekly against `svelte` 5,841,239 — and by roughly 374 times on the specific library we depend on most (`https://api.npmjs.org/downloads/point/last-week/...`, retrieved 2026-09-03).
For a solo founder working with coding agents this is a real argument and should not be waved away: agents have seen far more React than Svelte, and React answers are denser on the public web.
It is also the argument with the shortest half-life, because it is about the cost of writing code we have already written.
The counterweight is that Svelte 5 with runes is closer to plain JavaScript than React's hook rules are, and our 248 passing tests plus an approved visual identity encode the knowledge that a rewrite would put back at risk.

Migration cost, with assumptions stated.
The tree splits into 5,450 lines across 42 `.svelte` files, 6,880 lines across 50 `.ts` files (of which 2,565 are `vitest` tests in 21 files and 204 are generated), and 1,806 lines of CSS, over 29 route directories.
The `.ts` logic modules — `api.ts`, `listings-view.ts`, `authoring.ts`, `publish-readiness.ts` and their siblings — are framework-free and port essentially unchanged, as does the Tailwind CSS.
Estimate, at one experienced engineer producing 150 to 250 lines per day of finished, reviewed, test-passing component code against a settled design:

| Work | Days |
|---|---|
| Rewrite 5,450 lines of Svelte components as JSX | 22–36 |
| Port 58 `createQuery` and 11 `createMutation` sites to `useQuery`/`useMutation` | 3–5 |
| Re-express 29 SvelteKit routes as TanStack Router routes, adding search-param schemas | 4–7 |
| Swap the better-auth Svelte client for its React client | 1–2 |
| Rework the component-level share of 248 tests; framework-free ones port free | 4–8 |
| Rewire Tauri, the build, `just web-check` and the nix check | 3–5 |
| Re-verify the approved identity across every screen | 3–6 |
| Regression tail and live re-verification against both marketplaces | 5–10 |
| Total | 45–79 |

Call it 45 to 80 engineer-days, nine to sixteen working weeks for one person, and note that it is cheaper than the 90-to-140-day Leptos estimate and still larger than the entire remaining path to a landing page.
The risk is not the arithmetic; it is that the thing being rewritten was approved by the founder two days ago and is not what is blocking launch.

## 4. Component ecosystems for the hard components

First, the fact that reframes this section: we currently consume no UI component library.
The only non-relative runtime imports in `web/src` are `@tanstack/svelte-query`, `better-auth`, `svelte` and SvelteKit's own `$app/*`.
Every dialog, panel, upload field and axis picker in the console is hand-rolled over Tailwind — `src/lib/UploadField.svelte` is 137 lines and already does progress, `src/lib/AxisField.svelte` handles taxonomy axis selection, and there is no rich-text editor anywhere yet.
So this comparison is about components we have not built, not about a library we would lose.

| Component | React options | Svelte options |
|---|---|---|
| Capped multi-select | `react-aria-components` `TagGroup` and `TokenField`; `react-select` 5.10.2 (9,679,441/wk); `downshift` 9.4.0; shadcn/ui `combobox` | `bits-ui` `combobox` + `select` (no tag/token primitive); `svelte-multiselect` 11.8.0 (30,477/wk); shadcn-svelte registry has no `combobox` entry |
| Searchable tree | `react-aria-components` `Tree` and `NavigationTree`; `react-arborist` 3.16.0 | none first-class; `bits-ui`, `melt`, shadcn-svelte and Radix all ship no tree |
| Data table | `@tanstack/react-table` 9.2.4 (20,010,461/wk); `react-aria-components` `Table`, `GridList`, `Virtualizer` | `@tanstack/svelte-table` 9.2.4 (58,351/wk); `svelte-headless-table` 0.18.3, last published 2024-10-28 |
| Uploader with progress | `react-aria-components` `DropZone` + `FileTrigger`; `@uppy/core` 6.0.0 (1,242,604/wk, framework-agnostic); shadcn/ui `attachment` | `svelte-file-dropzone` 2.0.9, last published 2024-10-19 (14,457/wk); Uppy works here too; ours is already hand-rolled |
| Tabbed diff form | `@tanstack/react-form` 1.33.5 + Radix/`bits-ui`-class tabs; no first-class diff view either side | `@tanstack/svelte-form` 1.33.5 + `bits-ui` `tabs`; no first-class diff view either side |
| Rich-text editor | TipTap 3.31.0 (18,613,079/wk); `@lexical/react` 0.49.0 | TipTap via `svelte-tiptap` 3.0.1, last published 2025-10-28; TipTap core is framework-agnostic |

Sources: `https://api.github.com/repos/adobe/react-spectrum/contents/packages/react-aria-components/src`, `https://api.github.com/repos/radix-ui/primitives/contents/packages/react`, `https://api.github.com/repos/huntabyte/bits-ui/contents/packages/bits-ui/src/lib/bits`, `https://api.github.com/repos/huntabyte/shadcn-svelte/contents/docs/src/lib/registry/ui`, `https://api.github.com/repos/shadcn-ui/ui/contents/apps/v4/registry/new-york-v4/ui`, plus the npm registry, all retrieved 2026-09-03.

Accessibility primitives are where the asymmetry is sharpest and it favours React decisively.
`react-aria-components` 1.21.0, published 2026-09-01 at 4,025,930 weekly downloads, ships `Tree`, `NavigationTree`, `TagGroup`, `TokenField`, `Table`, `GridList`, `DropZone`, `FileTrigger` and `Virtualizer` — four of our six hard components have a maintained, WAI-ARIA-audited primitive there.
Radix ships none of those four: its package list has no tree, no table, no dropzone and no tag or token field, which means shadcn/ui inherits that gap and covers it with `combobox` and `attachment` recipes rather than primitives.
On the Svelte side, `bits-ui` 2.19.0 (published 2026-08-20, 1,016,830/wk) is healthy and covers the conventional set — dialog, select, combobox, command, calendar, tabs, tooltip — and covers none of the four hard ones.
`shadcn-svelte` 1.6.0 published 2026-09-01 is actively maintained but is a styling layer over `bits-ui` and inherits the same gap.
The Svelte headless landscape also has a live fragmentation problem: `@melt-ui/svelte` 0.86.6 last published 2025-03-28 at 209,150 weekly downloads is the widely-installed version, while the successor package `melt` 0.44.0, published 2026-01-04, draws 8,331 — the users have not moved.

The honest reading is that React would let us buy a searchable accessible tree and a token-style capped multi-select instead of building them, saving perhaps five to fifteen days across the two, against a 45-to-80-day migration.
The arithmetic does not close, and it closes even less once you notice we would also inherit React's accessibility primitives for the twenty components we already built correctly.

## 5. Astro honestly

Astro is at 7.2.10, published 2026-08-31, with `@astrojs/svelte` at 9.0.1, published 2026-07-01 (npm registry, retrieved 2026-09-03).
It draws 5,097,604 weekly downloads, comparable to Svelte itself.

What it is for, in its own words: "the web framework for building content-driven websites like blogs, marketing, and e-commerce" (`https://docs.astro.build/en/concepts/why-astro/`, retrieved 2026-09-03).
Its islands model strips JavaScript by default and hydrates named components through `client:load`, `client:idle` and `client:visible`; "An island always runs in isolation from other islands on the page", though islands "can still share state and communicate with each other" (`https://docs.astro.build/en/concepts/islands/`, retrieved 2026-09-03).
It supports React, Preact, Svelte, Vue and SolidJS components on the same page.

Whether it can host our dashboard: no, and the documentation says so about the category rather than about us.
The same page distinguishes Astro from frameworks that "excel at building more complex, application-like experiences" and names "logged-in admin dashboards, inboxes, social networks, todo lists" as their territory, describing Astro's own model as "Multi-Page App (MPA)" in contrast to the SPA model of "Next.js, SvelteKit, Nuxt, Remix".
There is no first-party client-side router.
The closest thing is `<ClientRouter />`, which "intercepts page navigation" and calls `document.startViewTransition`, performing a swap in which "the `<body>` is completely replaced with the new page's body"; `transition:persist` can carry an island across a navigation, but "not all state can be preserved" (`https://docs.astro.build/en/guides/view-transitions/`, retrieved 2026-09-03).
That is a body-swapping shim over an MPA, not client routing.
A logged-in console with a persistent query cache, an EventSource connection and a shared session would fight it on every navigation.
Astro is also absent from Tauri's documented frontends: `https://api.github.com/repos/tauri-apps/tauri-docs/contents/src/content/docs/start/frontend` lists `leptos`, `nextjs`, `nuxt`, `qwik`, `sveltekit`, `trunk` and `vite`, and no Astro page (retrieved 2026-09-03).
The generic Vite guide would apply, since Astro is Vite-based, but there is no first-party recipe.
The degenerate way to make it work — one catch-all Astro page containing a single `client:load` Svelte island — is a Svelte SPA wearing an Astro shell, which adds a build layer and returns nothing.

Where Astro genuinely fits us is the marketing surface, and it fits it well.

## 6. The landing page decision

The relevant fact about our current app is in `web/src/routes/+layout.ts`: the root layout sets `export const ssr = false` and `export const prerender = false`, and `svelte.config.js` uses `adapter-static` with `fallback: 'index.html'`.
Every route therefore serves the same near-empty shell and renders on the client.
SvelteKit's own adapter documentation is blunt about what that costs: the fallback option "has large negative performance and SEO impacts" and is "only recommended in certain circumstances such as wrapping the site in a mobile app" (`https://svelte.dev/docs/kit/adapter-static`, retrieved 2026-09-03).
We are precisely that circumstance for the console, and precisely the wrong one for a marketing page that must rank, unfurl in social previews and paint fast for a first-time visitor.

Three concrete options.

A SvelteKit route inside the existing app is possible and cheapest in tooling: a marketing route can override the root layout with `export const prerender = true` and `export const ssr = true` in its own page module, because the adapter documentation notes that prerendering with `ssr` false yields "an empty 'shell' page instead of the fully rendered content".
Cost is roughly one to two days plus the standing tax of every marketing edit invalidating the console's build and vice versa, and no path to a blog or help-content programme without importing one.

Plain HTML on Cloudflare Pages is the true floor: one to two days, no build, no framework, and it stops scaling the moment there are five pages sharing a header or the first blog post appears.

Astro 7 as a separate site is the recommendation.
It costs roughly two to four days to stand up, our Tailwind design tokens port verbatim as CSS, `@astrojs/svelte` 9.0.1 lets us reuse a Svelte component if a page needs interactivity, and content collections give the help centre and any future blog a home that does not touch the console's build.
It is fully isolated from the app, so it carries no regression risk to a client redesigned two days ago.
Hosting is free at our scale: the Cloudflare Pages free plan allows 500 builds per month, one concurrent build with a 20-minute timeout, 100 projects, 20,000 files per site and a 25 MiB per-asset ceiling (`https://developers.cloudflare.com/pages/platform/limits/`, retrieved 2026-09-03), and "requests to static assets are free and unlimited" on both free and paid plans (`https://developers.cloudflare.com/pages/functions/pricing/`, retrieved 2026-09-03).
The Paddle checkout the landing page gates is a client-side script, so nothing about it requires a server on that host.

If no content programme materialises — no blog, no help centre, five static pages forever — the prerendered SvelteKit route is the better answer and the decision is cheap to revisit, because a landing page is days of work in either direction.

## 7. Svelte 5 and SvelteKit today

Svelte is at 5.57.0, published 2026-08-28, with releases roughly weekly (5.56.9 on 2026-08-12, 5.56.10 on 2026-08-20).
SvelteKit is at 2.70.3, published 2026-08-18, and `@sveltejs/adapter-static` at 3.0.10.
We pin `svelte: ^5.38.0`, `@sveltejs/kit: ^2.27.0` and `@sveltejs/adapter-static: ^3.0.8`, so we are current on majors and a little behind on minors.

Runes are the shipped reactivity model of Svelte 5, not an experiment: the language documentation describes them as "symbols that you use in `.svelte` and `.svelte.js` / `.svelte.ts` files to control the Svelte compiler" and frames pre-runes reactivity as "Legacy mode" (`https://svelte.dev/docs/svelte/what-are-runes`, retrieved 2026-09-03).
Note the shape of the evidence: the page asserts no stability guarantee in so many words, and the stability claim rests on runes being the documented default with the older model marked legacy, plus fifty-seven minor releases without a rune-breaking major.

A major is in flight.
`@sveltejs/kit` carries a `next` dist-tag at `3.0.0-next.25` published 2026-08-21, and the `version-3` branch is at `3.0.0-next.26` with active fixes (`https://registry.npmjs.org/-/package/@sveltejs%2fkit/dist-tags` and `https://raw.githubusercontent.com/sveltejs/kit/version-3/packages/kit/CHANGELOG.md`, both retrieved 2026-09-03); `@sveltejs/adapter-static` has a matching `4.0.0-next.4`.
What SvelteKit 3 breaks is not something I established, and it should be treated as an unpriced upgrade sitting in the next year rather than as a reason to move now — every alternative on this list has its own next major, and Start has not reached its first.

The static adapter fits Tauri exactly, and Tauri documents this configuration for SvelteKit specifically: `adapter-static` with `fallback: 'index.html'`, `ssr = false` in the root layout, `frontendDist: "../build"`, `devUrl: "http://localhost:5173"`, with the note that "Tauri doesn't support server-based solutions" and that prerendering is discouraged because "load functions will not have access to tauri APIs during the build process" (`https://v2.tauri.app/start/frontend/sveltekit/`, retrieved 2026-09-03).
Our `svelte.config.js` and `+layout.ts` already match that recipe line for line.

The reason to leave that the founder should hear, stated without softening: the Svelte ecosystem is thin exactly where our remaining hard components live.
There is no maintained accessible tree, no token-style multi-select primitive, the headless-primitives layer is fragmenting between `@melt-ui/svelte` and `melt`, and the TanStack adapters we would lean on have between 0.27% and 1.2% of their React counterparts' installed base.
That is a real cost that will be paid in hand-written components over the next year.
It is smaller than 45 to 80 engineer-days, and it is the only entry on the leave-Svelte side of the ledger that survives scrutiny.

## 8. Comparison

| | SvelteKit (stay) | Svelte + more TanStack | React + TanStack Start | Astro for the app | Astro for the landing page only |
|---|---|---|---|---|---|
| Fits a Tauri webview | yes, documented recipe we already match | yes, unchanged | yes, SPA mode emits a static shell; no Tauri recipe published | no first-party client router; no Tauri recipe; degenerates to a Svelte SPA in a shell | not applicable; the landing page is not shipped in the app |
| Migration cost | zero | 1–3 days per library adopted | 45–80 engineer-days | comparable to the React rewrite with no compensating gain | 2–4 days, isolated |
| Ecosystem for our hard components | thin: no tree, no token multi-select | adds a maintained data table, form and virtualiser | strongest: TanStack Table, react-select, react-arborist, TipTap, Uppy | irrelevant; islands would import one of the other two ecosystems | irrelevant; a marketing page needs none of them |
| Accessibility primitives | `bits-ui` covers the conventional set, none of the four hard ones | unchanged; TanStack is headless, not accessible-by-default | `react-aria-components` covers Tree, TokenField, TagGroup, Table, DropZone, FileTrigger | inherits whichever island framework is chosen | not applicable |
| Bundle for first paint | 80,083 bytes gzipped, measured today | +6–15 KB gzipped per library adopted | ~62 KB React floor plus up to 39 KB router before our code | near zero for static pages, but the dashboard's payload returns inside the island | near zero; that is the point |
| Agent and hiring fluency | `svelte` 5,841,239/wk; `svelte-query` 175,553/wk | unchanged for Svelte; adapters are the thinnest layer | `react` 171,637,376/wk; `react-query` 65,663,515/wk | `astro` 5,097,604/wk, but no dashboard precedent to learn from | same as the app column, and the surface is small |
| What it adds | nothing new; keeps 248 passing tests and an approved identity | a data table and a form engine without a rewrite | typed search params, per-route SSR, server functions we would not use, React's ecosystem | nothing we need | SEO, fast first paint, social unfurls, a home for content, free hosting |

## 9. Verdict

For the app: stay on SvelteKit.
The deciding evidence is that TanStack Start is React, that a React migration is 45 to 80 engineer-days, and that what it buys for a dashboard whose backend is Rust reduces to typed search params plus a component ecosystem we do not currently consume on either side.
The bundle direction runs against React on the one surface where it matters most, a mid-range Android Tauri webview, and Start is a Release Candidate whose first stable major has not shipped.
Revisit only if two things become true together: we accumulate real evidence that the Svelte component gap is costing us weeks per quarter, and the console is due a rewrite for a reason of its own.

Adopt `@tanstack/svelte-table` when the first screen genuinely needs sorting, pagination and persisted column state, not before, and take `@tanstack/svelte-form` only if a second form of the listing editor's complexity appears.

For the landing page: build teachouse.io as a separate Astro 7 site with our Tailwind tokens, deployed to Cloudflare Pages on the free plan, at roughly two to four days.
The reason is not that Astro is fashionable but that our console is a client-rendered SPA shell by deliberate design, and SvelteKit's own documentation says that shape has "large negative performance and SEO impacts" — which is acceptable for a logged-in console and disqualifying for the page that has to rank and convert.
If the founder is certain there will be no blog and no help-content programme, a prerendered SvelteKit route is the smaller answer and costs a day less.

## 10. Open questions

1. Does the teachouse.io surface include a blog, help centre, or any ongoing content programme?
   Recommendation: assume yes and use Astro; if no, use a prerendered SvelteKit route instead.
2. Is a searchable accessible tree an actual requirement, or is `AxisField.svelte` already the shape the taxonomy picker needs?
   Recommendation: treat the tree as hypothetical until a screen demands it, since it is the single strongest React argument and it may not be real.
3. Should the SvelteKit 3 upgrade be scheduled now or deferred?
   Recommendation: defer until 3.0.0 ships stable, then price it as a separate change.
4. Is a rich-text editor in scope for listing copy, given that models generate that copy today?
   Recommendation: assume TipTap when it arrives, which is framework-agnostic and settles the question for both ecosystems.

## 11. Unverified

- What SvelteKit 3.0 changes or breaks; only its existence at `3.0.0-next.26` on the `version-3` branch was established.
- Whether `@tanstack/react-start`'s 16,466,103 weekly downloads reflect application use rather than CI and transitive installs; the figure is reported as returned by the registry and not interpreted.
- Real-world first-load payload of a comparable React plus TanStack Start SPA; the 62 KB and 39 KB figures are package measurements, and no equivalent application was built or measured.
- Whether the 45-to-80-day migration model matches this team's actual throughput; the 150-to-250 lines-per-day assumption is stated, not calibrated against our history.
- Cloudflare Pages bandwidth limits on the free plan; the limits page names builds, projects, files and asset size but no bandwidth figure, and the pricing page asserts only that static asset requests are "free and unlimited".
- Whether Astro has any first-party Tauri support beyond the generic Vite guide; the Tauri frontend documentation directory lists no Astro page, which is evidence of absence rather than a statement of incompatibility.
- Accessibility quality of `react-aria-components`' Tree and TokenField in practice; only their presence in the package source was established, not their fitness for our screens.
