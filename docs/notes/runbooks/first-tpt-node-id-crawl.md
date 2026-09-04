# The first TPT node-id crawl

TPT binds an education standard by an opaque numeric node id rather than by the published code, so `docs/design/data/standards/tpt-node-ids.jsonl` is what makes the standards feature work on the wire, and it is committed empty.
Filling it means walking four jurisdiction subtrees under a live TPT session.
TPT is a no-API marketplace, so under D1 the server may never issue one of those requests, and decision 5 of `docs/notes/design/standards-ingestion.md` puts the walk on the founder's own machine under the founder's own session rather than on every seller's device.
This file is the procedure for that one act.

It drives a binary that already exists, `crates/tam-standards-crawl`, and changes no code.
Where a step this document would need does not exist yet, it is named under "Gaps" at the end rather than invented here.

## Prerequisites

A checkout of this repository and its dev shell, since the crawl is a workspace binary run with `cargo run` rather than an installed tool.

A TPT login in an ordinary browser, on the founder's own account, exported as a Netscape cookie jar to `probes/local/tpt-cookies.jar`.
That is the ingestion shape the code already proves: `TptSession::from_netscape_jar` at `crates/tam-marketplace-tpt/src/session.rs:70`, used by the supervised live read at `crates/tam-marketplace-tpt/examples/live_read.rs:187` and by the crawl binary itself.
The jar must carry the `csrfToken` cookie, because TPT's CSRF is a double submit and the session type mirrors that cookie into an `x-csrf-token` header; a jar without it is refused with "the jar carries no csrfToken cookie, so no request can be authorised".
`probes/local/` is gitignored at `.gitignore:21`.
The jar is a secret: it is never printed, never pasted into a report, and never committed.

The desktop client is not the source of that jar.
It holds the seller's cookies in the operating system keychain and no path in it sends them anywhere or writes them to disk in Netscape form (`apps/desktop/src-tauri/src/session/mod.rs`), and neither the client nor its release notes carry a crawl or capture command.
The browser export is the only path that exists today.

The server is not needed to run the crawl and does not participate in it.
It is needed afterwards, to serve the ids the crawl bound, and that half is not yet configurable — see G1 under "Gaps".

## What the crawl does, in the order it does it

Reading this before running it is what lets a wrong step be recognised as one.

One `EducationStandardsJurisdictionsQuery` enumerates TPT's roots; the research records 166 of them, and this operation is the only enumeration that exists.
Each of the four frameworks is then confirmed against that answer by `confirm_root`, which is the notation check described below, and a failure there stops everything before any expansion.
Each framework is then walked the way TPT's own picker walks it: the root expanded at `depth: 1`, then each child subtree expanded whole with no depth argument, with a one-second pause between calls (`BETWEEN_CALLS` in `crates/tam-standards-crawl/src/main.rs`).
The crawled nodes are bound against the standards corpus compiled into the binary, the bindings are written sorted and newline-terminated, and a residue report names every crawled node that bound nothing.

## Steps

1. Confirm the working directory is the repository root and that `probes/local/tpt-cookies.jar` exists and is current.
   A jar already sits at that path from an earlier session; refresh it from the browser rather than assuming it still authenticates.

2. Prove the jar before spending a bulk walk on it, with the read-only live example:

   ```
   cargo run -p tam-marketplace-tpt --example live_read -- probes/local/tpt-cookies.jar
   ```

   It walks the founder's own catalogue and prints; it builds no submission, posts no form and touches no write path.
   A lapsed jar fails here in one request instead of failing partway through a four-framework walk.

3. Run the crawl, capturing the output, because the report it prints is the only record of what happened:

   ```
   cargo run -p tam-standards-crawl -- --i-am-the-founder docs/design/data/standards "$(date -u +%Y-%m-%dT%H:%M:%SZ)" probes/local/tpt-cookies.jar 2>&1 | tee logs/first-tpt-node-id-crawl-$(date +%Y%m%d-%H%M%S).log
   ```

   The acknowledgement flag is required and has no default; it exists so that each run is a deliberate act rather than an available one.
   The timestamp is an argument rather than a clock read, and the binary refuses anything that is not an RFC 3339 UTC timestamp of the shape `2026-10-03T12:00:00Z`.
   The jar path is optional and defaults to `probes/local/tpt-cookies.jar`; it is written out above so the log records which jar was used.
   `logs/` is ignored at `.gitignore:16`, so the capture leaves no untracked file behind.

4. Record the timestamp argument verbatim before doing anything else.
   It becomes `verified_at` on every binding the run writes, and it is the value the crawl window is later opened at.

5. Read the report before committing anything.
   Per framework the binary prints `<framework>: N bindings over M of R taggable rows; residue A outside the walked root, B no row carries, C no row states alike, D claims on rows two nodes both reached`, then `wrote <path> (N bindings, X bytes)`, then one line per residue node with the reason attached.
   Nothing in the residue is a defect to fix in the file: an unbound node stays unbound, a seller's tag for it is carried in our own catalogue, and a TPT publish omits it with a visible loss record.

6. Commit `docs/design/data/standards/tpt-node-ids.jsonl`.
   `just check` fails at this point, by design and not by accident — see G3 under "Gaps".

## What is captured, and where it lands

One file: `docs/design/data/standards/tpt-node-ids.jsonl`, written into the out-dir given on the command line, one binding per line, sorted by framework and then by the mirror's `source_guid`.
Each binding carries the TPT node id, the name TPT itself returned for that node, the SHA-256 of the statement TPT returned, and the crawl timestamp.

Nothing lands in Postgres.
Migration 0041 creates a `standards_node` table, but the API reads the committed corpus rather than the database, and no repository writes the node-id table today.

The file is compiled into two binaries with `include_str!`, at `crates/tam-api/src/product/standards.rs:46` and in the crawl binary itself.
So serving the ids the crawl bound needs a rebuild and a redeploy of `tam-server`, not a restart.

## Verifying it landed, and how many to expect

The binary's own `wrote … (N bindings, X bytes)` line is the first check, and `wc -l docs/design/data/standards/tpt-node-ids.jsonl` must equal N.
A second run over the same answers produces the same bytes, which is the determinism the ingest has for the same reason.

For coverage, the per-framework report's `over M of R taggable rows` is the number to read.
R is fixed by the ingest and is the ceiling on distinct rows that can bind: 2,240 for Common Core, 440 for NGSS, 5,425 for Texas and 4,388 for Virginia, 12,493 in total ("What landed, in numbers" in `docs/notes/design/standards-ingestion.md`).
One node binding many rows is expected, because the mirror repeats a Common Core anchor standard across eleven grade sets, so N can exceed M.

No expected TPT-side node count is recorded anywhere in this repository, and this crawl is the measurement rather than a check against one.
What is recorded is the shape: 166 roots exist, the four walked are 3054 Common Core, 3055 NGSS, 3326 TEKS and 5785 VA SOL, and TPT's own picker made 58 expansion calls in the one capture we hold.

Read a wholly empty or wholly-residue result as evidence about our code first.
Neither crawl operation appears in any HAR capture: the query texts are reconstructed from recorded selection sets and the two `children` argument names are inferred, so this run is the first test of the request shaping and of the join, not only of TPT's tree.
A capture whose `parentIds` chain omitted the walked root would put every node in the residue under "outside the walked root", which is a visible refusal to read rather than a silent mis-scope.
Statement transcription differences between the mirror and TPT will put some codes in the residue that a human would call the same standard; that is expected and is not a fault to patch around.

## Notation refusal, and what to do when it fires

Before expanding anything, `confirm_root` reads TPT's own enumeration back and checks each of the four roots against the notation recorded for it — `ccss`, `ngss`, `teks` and `va sol`, compared case-insensitively (`tpt_jurisdiction_notation` at `crates/tam-standards/src/crawl.rs:47`).
It refuses in three shapes: the enumeration no longer carries the root id at all, the root notates something other than the expected value, or the root carries no notation and so nothing confirms it.
Any of the three stops the crawl before a single expansion, and no file is written.

The reason is the same one the kill gate exists for.
The id read back as `sphinxId` names a search index, search indexes get rebuilt, and the only evidence that 3326 still means Texas is TPT saying so.
Expanding a root on the strength of its number alone would make exactly the bet the design refuses.

What the founder does is stop.
Do not edit the four constants to make the check pass, and do not remove the check.
Record the refusal message and the root row TPT actually returned, and treat it as the kill gate arriving early: the ids moved, and what to do about it — store-and-post, tighten the cadence, or resolve ids at post time on the device — is the founder code decision the gate was designed to force, not a step in this file.

Two further refusals are fail-closed in the same way and mean something different.
A framework that answers no standards at all, and a walk that produces no binding at all, each stop the run and leave the existing table exactly as it stands, because an empty write would blank a table the founder had already filled.
Both most plausibly mean the session lapsed or a query text is wrong, not that TPT emptied its tree; re-prove the jar with step 2 before rerunning.

## What to send back to the team

The whole captured log, which carries the four per-framework report lines, the wrote line and the residue report.
The timestamp argument used, verbatim, because the crawl window is opened at it and a remembered approximation is not usable.
The line count and byte count of the written file.
Whether any refusal fired, and which of the five it was.
A note on how long the walk took end to end and how many expansions it made per framework, since no estimate of that exists yet and the next capture's cadence is planned against it.

Never send the jar, a cookie header, or any line of either, and never attach the raw responses.

## What must never be done

No server-side crawl, under any circumstance.
TPT is a no-API marketplace, the server never composes or issues a request to one, and a test fails the build if a no-API marketplace gains a server transport (`crates/tam-domain/src/registry/mod.rs:323`).

No scripted, scheduled or bulk crawl beyond this one supervised walk.
The binary's one-second pause between calls is not to be shortened, the walk is not to be parallelised, and the run is not to be repeated to "get a better result".
Bulk enumeration is the behaviour most likely to draw the response the marketplace-terms memo names as the pivot, which is precisely why it is concentrated on one account the founder controls.

Never run it from CI, from a timer, or on a seller's behalf.
Enumeration stays off every seller's device; what runs there is a single-question freshness check on exactly the ids a device is about to post.

Never widen the walk beyond the four roots and never raise the depth argument.
The root at `depth: 1` and each child whole is the shape TPT's own client uses, and asking for a whole subtree in one call is asking TPT for something its own client never asks for.

Do not run the next capture sooner than thirty days from this one.
The gate protocol is a second capture at least thirty days later, diffed against this one, and decision 7 puts it before the standards feature is enabled for any seller.

## Gaps

Six places where this procedure assumes something the code does not yet provide.

G1. There is no way to open the crawl window.
`Config.standards_crawl_window` exists at `crates/tam-api/src/lib.rs:80` and the handler reads it at `crates/tam-api/src/product/standards.rs:381`, but `tam-server`'s argument parser has no flag for it and reads no environment, and `Config::default()` leaves it absent.
Absent means every node id is withheld, which is the intended fail-closed reading, so committing a filled table changes nothing a seller sees until a flag or equivalent is added.
That is a code change, and no command for it is invented here.

G2. Neither the desktop client nor its release notes carry a crawl or capture command, and the client has no cookie-jar export.
The jar comes from a browser, by hand.

G3. Committing a filled table turns `just check` red.
`crates/tam-standards/tests/committed_ingest.rs:242` asserts that the committed table is empty, with the reason recorded in the assertion message.
Landing the crawl's output means changing that test in the same change, which is a deliberate act rather than a build repair.

G4. The kill gate has no runner.
`diff_capture` and `gate` in `crates/tam-standards/src/crawl.rs` decide a second capture's verdict and are tested against fixtures, but nothing takes a second capture and feeds it to them; the crawl binary writes the table and does not re-crawl against one already written.
The second capture that decision 7 requires therefore needs that runner built first.

G5. Neither crawl operation is verified against a real TPT response, because no capture of one exists in this repository.
The query texts are reconstructed and the two `children` argument names are inferred, so a failure in this run may be ours rather than TPT's.

G6. Nothing converts a jar to a cookie header, refreshes a jar, or reports how long a TPT session lasts.
The jar is re-exported by hand when it lapses, and there is no measurement on record of how often that is.
