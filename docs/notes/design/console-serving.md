# Serving the console, the landing page and the downloads from one origin

- date: 2026-09-05
- status: implemented in `crates/tam-server`; `nix/landing.nix` builds the landing site as `packages.teachouse-landing` and `checks.landing`, and `services.teachouse.landingPackage` is null by default because what a deployment's root answers with is a per-host decision rather than an upgrade
- paths: `crates/tam-server/src/serving.rs`, `crates/tam-server/src/downloads.rs`, `crates/tam-server/src/main.rs`, `nix/module.nix`, `flake.nix`, `apps/landing/`

## The precedence rule

One origin answers four surfaces, and the order it tries them in is fixed: the API's own routes, then the landing build, then `/downloads`, then the console.
The API is first by construction — `tam_api::router` matches its routes before anything reaches a fallback — so no directory placed behind it can shadow `/healthz` or a path under a version this build serves.
Four namespaces are then reserved ahead of everything below: `app` and `_app` are the console's home and its client bundle, `resources` is the catalogue board, and `downloads` is the tier under it.
Reservation rather than ordering, because the hazard is a landing build quietly taking one of them: a page at `app/` would answer the console's home with a marketing page under the marketing policy, and anything at `_app/` would break the console at every route.
`resources` is the one a marketing site would take by accident rather than by collision, since a teaching-resources site has an obvious use for the word, and the seller asking for their catalogue would get that page with nothing anywhere saying so.
All four are reserved whether or not the tier behind them is configured, so a deployment serving no downloads cannot let a landing build take `/downloads/` and then have enabling the tier shadow a live page.
The landing page answers only the paths it actually holds a file for: `/` through its `index.html`, `/pricing`, `/privacy` and `/terms` through theirs, and every hashed asset under `_astro/` by its exact name.
The console is last and unchanged: it answers everything else, serving its asset tree where the tree has the file and its single-page shell where it does not.
`crates/tam-server/src/serving.rs` writes that order down as one pure function, `route`, which takes a request path, a way to ask whether the landing build holds a file, and whether downloads are configured, so the rule is tested without a listener, a directory or a socket.

The landing build is read into memory once at start-up, the way the console's shell already was, which is what lets a request be answered by a map lookup rather than by joining a client-supplied string to a filesystem path.
That read happens once: a page edited under a running server is neither re-read nor re-hashed, so its bytes and its policy stay consistent with each other and stale relative to disk until a restart — correct for a store path, and worth knowing before pointing the flag at a working directory.
Traversal is refused twice over: `route` percent-decodes first and then rejects any `..` or `.` segment, and the only names that can be served at all are the ones the start-up walk put in the map.
Decoding comes before that guard and never after, which is what keeps `/%2e%2e/` and `/../` the same refusal while letting a page named `für-lehrer` resolve from the `/f%C3%BCr-lehrer/` a browser actually sends.
A symlink inside the landing directory is neither followed nor served, because following one is the only way that directory's contents could name bytes outside it, and both it and a name that is not UTF-8 are reported at start-up rather than silently dropped.
Only `GET` and `HEAD` reach a static tier; everything else is refused with 405 and `Allow: GET, HEAD`, which is what the console's own `ServeDir` did before any of this existed.
Every static response carries a strong entity tag over its bytes and a freshness rule matched to how its name changes: `no-cache` for a page, a year and `immutable` for a content-addressed asset under `_astro/`, an hour for everything else.

## Two policies, not one

The two document surfaces carry different content-security policies, and the landing page's is the narrower of them.
It is `default-src 'self'`, `script-src 'self'` plus a `sha256-` token for every inline block found in every `.html` file at start-up, `style-src 'self'`, `font-src 'self'`, `img-src 'self' data:`, `connect-src 'self'`, and `frame-ancestors 'none'` so the marketing surface cannot be framed at all.
It admits no third-party host on any directive and no `'unsafe-inline'` on any: the built site self-hosts both font faces from `/fonts/*.woff2` and emits neither a `<style>` element nor a `style` attribute, so the two Google font hosts and the inline-style grant the first version carried were dead grants on the public origin.
The hashes are computed by the same routine the console's policy uses, generalised from one shell to every page in a directory, so a static site of several pages is covered by one policy and a repeated block contributes one token.
A download carries no policy at all, because a policy governs a document and nothing renders an installer.
The static decision is layered outside the console's policy layer, and that placement is load-bearing: a landing or download answer short-circuits before the console's layer can insert the console's header over it.

## `/app`, and what the desktop client has to do about it

The console's home moved to `/app` and its deep routes did not move with it, so `/labels` and `/sync` are still where they were.
That is the whole of what the server needs to know about the move, because the shell answers any path no other tier claims, so `/app` and everything below it resolve exactly as an unknown console route always did.
The consequence for the desktop client is that its window must navigate to `/app` rather than to `/`, and until a release does that the landing page itself is what sends the app on.
`--landing-dir` and `--downloads-dir` are both optional and, absent, the server behaves exactly as it did before either flag existed.

## The downloads tier, and the unit that fills it

`tam-server`'s unit denies IP egress outright and has to keep doing so, so the process that serves a download can never be the process that fetches one.
`teachouse-downloads-refresh` is that second process: a oneshot on a fifteen-minute timer, under its own system account, hardened like the other oneshots but without the egress deny, writing one directory and holding no database url, no marketplace credential and no seller session.
It reads the vendor's update endpoint named by `updateUrl` for the current version and the Windows installer's URL, and — where `githubReleaseTokenFile` names a token — the configured repository's releases for the Android package and its two `SHA256SUMS` files.
The release it takes is the highest `vN.N.N` version among the newest twenty, never a draft and a prerelease only where the `prerelease` option allows it: by version rather than by publish date, so a hotfix to an older line cannot regress the public download.
The release workflow marks every release it publishes to a named channel as a prerelease, so `prerelease` defaults to whether `updateUrl` names a channel.
Until 2026-09-07 the filter refused every prerelease outright, on the ground that a package cut for internal testing must not become the public one within an interval; on the beta channel that refused the very release whose Windows installer the same run had just published, which is why the live manifest carried `"android": null` beside a Windows entry for v0.3.4 while the release itself carried the APK and both sums files.
A digest is looked up by the exact filename in the sums file and compared against the staged bytes, in both the `HASH  NAME` and `HASH *NAME` forms `sha256sum` writes; a file the sums do not name is a failure and not a pass, which is what `--check --ignore-missing` could not tell us, since it succeeds whenever anything else in the directory happens to match.
An unreadable or empty token file fails the run and says so, rather than sending an empty bearer that keeps working against a public repository and starts failing the day it becomes private.

Each component — the Windows installer, the Android package, the two sums files — is published only when its own bytes were fetched and verified, and a component that was not is left exactly as it was: its file stays, the manifest keeps naming it, the cleanup keeps it, and the run ends non-zero.
That asymmetry is the point, and it is what the first version got wrong: a listing that returned only prereleases deleted a working Android download and published `"android": null`, taking the surface down to fix nothing.
Without a token the Android half is deliberately absent rather than degraded, and `downloads.json` carries `"android": null`.
The Windows installer comes from the update channel while its digest comes from a GitHub release, so the two can sit at different versions; where a sums file exists it must name the installer, because publishing an executable nothing vouches for is the one outcome a download surface must not have, and where no sums file exists at all it is published on the vendor's TLS alone.
`version` is the channel's, and the Android half carries its own `version` beside its digest, so a manifest never reports one number for two different releases.

Staging is `.staging/` inside the served directory: the same filesystem, so the rename out of it is atomic, and a name `route` refuses, so nothing half-written is ever reachable — the earlier `.incoming-…` beside the published files was a partial installer a client could ask for by a guessable name, and a run killed between writing and renaming left it served until the next successful refresh.
The staging directory is cleared at the start of every run, each file is put in place by a rename, and `downloads.json` is renamed last, so no client reads a manifest naming a file that is not there yet.
Stale files go after that — the ordering buys that the manifest never names a file that is not present, not that a file the manifest named a moment ago is still there.
The directory is `/var/lib/teachouse-downloads`, a sibling of `/var/lib/teachouse` rather than a child, because `tam-server`'s own `StateDirectory` owns that parent at 0700 and a second unit declaring a subdirectory of it would fight over the parent's owner on every start.
It is created by a tmpfiles rule rather than by either unit, so it exists — owned by the refresh account, world-readable — before either starts, which is what lets `tam-server`'s start-up check pass on a host whose first refresh has not run yet.

On the serving side the tier is the one that reaches the filesystem per request, because its contents change under a running process, and it streams rather than reading a sixty-megabyte package into the heap of an HTTP process.
It holds the landing tier's construction a different way: `route` yields a single path segment and never one beginning with a dot, and `downloads.rs` enumerates the directory and serves the entry whose name is equal rather than joining that segment to anything, so the path opened is one this process produced.
Only a regular file is served, and the refusal is on the entry's own type before anything is opened: an open follows a symlink and every check after it describes the target, so a link planted by the one account in the deployment with internet egress would otherwise have `tam-server` read out a key that account cannot read itself.
The manifest is served `no-cache` and the files beside it for an hour.
Nothing consumes the surface yet — the console's download cards are the next step, reading `/downloads/downloads.json` and linking to `/downloads/<file>` — and `apps/landing/src/site.js` still carries `downloadUrl = null`.
A refresh that keeps failing is visible only in the journal and in `refreshed_at`, which is stamped when the published set last changed rather than when the unit last ran; the last good set keeps serving, which is the right failure, but nothing alerts on it.

## Where the artefact-shape claims are made

Two tests used to check `route` against the real builds by reading `apps/landing/dist` and `web/build` off disk.
Both directories are gitignored and neither matches the flake's `src` filter, so in every sandboxed run those tests returned early and reported a pass — a skip that `cargo test` and `cargo nextest` both discard, which made the two tests that touched a real artefact the two that never ran where it mattered.
Those claims are now `checks.served-artefacts` in `flake.nix`, which depends on `checks.landing` and `checks.console` and asserts them against the store paths: the landing build has an `index.html`, a directory per page, a hashed asset under `_astro/` and no symlink, and the console's shell does boot from an inline script block.
The collision claim is no longer a list of four names.
The check derives every first segment the console's route table defines by reading `web/src/routes` at eval time, recursing into a group directory and dropping a parameter one, adding `_app` and `downloads`, and it refuses a landing entry under any of them.
The message says which of the two failures happened, because they want different fixes: a name `route` reserves would 404 into the console shell, and a name it merely probes second would let the landing page shadow a live console route with nothing anywhere saying so.
A second assertion runs the reservation the other way: every name in `CONSOLE_NAMESPACES` must still be a console route, since a reservation the route table no longer backs would 404 a landing page into the shell rather than render it.
A separate assertion guards an unrelated collision: a top-level file both builds carry must be byte-identical, since the landing copy answers that name for both tiers.
A flake evaluates the source its VCS reports, so a route directory not yet snapshotted is invisible to the walk in a local build, and CI's run from a committed tree is the authority.
The pure decision keeps its own tests, and the ones that assert a path is *not* the landing's now run against a landing build that claims every name there is, so no assertion can pass by a fixture omitting a name.
