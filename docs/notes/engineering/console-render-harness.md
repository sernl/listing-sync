---
title: Rendering the console headlessly
---

What a headless render of this console has to account for, collected while taking one.
Every item below is a way a render lies rather than fails: the capture succeeds, the picture looks like an answer, and the answer is wrong.
It is a working note rather than charter, and promoting any of it into the engineering charter is a later deliberate edit.

## The page fades in, so the first frame is blank

`app.css` gives `.page` an animation named `rise` that begins at `opacity: 0` and runs for 160ms, under `@media (prefers-reduced-motion: no-preference)`.
A capture taken on the first frame therefore photographs a route that is fully laid out and entirely invisible, which reads as an unstyled or empty page rather than as a timing problem.
The obvious repair is to remove the animation, by forcing reduced motion or by injecting a stylesheet that clears it, and it is the wrong one.
The animation is on the element holding the whole route, so suppressing it changes the question from whether the console draws to whether the console draws once its own opening is taken away.
The faithful repair is to capture after the animation has settled, and the way to do that without a fixed sleep is to hold the load event open: give the page one resource that resolves slowly, an image is enough, and a capture that waits for load then lands after the fade.

## Render from a uniquely named snapshot, never from `web/build`

`web/build` is where `npm run build` writes, and it is shared.
A rebuild by anyone else replaces the JavaScript chunks under an open page, and because the chunk filenames carry content hashes the page's own script stops resolving mid-session, so the route empties for a reason that has nothing to do with the change being looked at.
The failure in the other direction is the dangerous one, because it is silent: a snapshot copied before the change was built renders perfectly and shows the previous version, and nothing in the picture says so.
So copy `web/build` to a snapshot directory whose name nothing else will take, serve that, and verify it before rendering rather than assuming it.
Grep the snapshot for a string only the new code contains — a class name the change added, or one it deleted — and refuse to render when the string is missing.
That check costs one command and is the only thing standing between a green render and a render of last week.

## A readiness check must prove the server is yours, not that one answers

The snapshot rule above stops a render from photographing a stale build.
It does not stop it from photographing somebody else's build, and on a shared machine that is the easier mistake to make.
Several sessions run this harness at once, each on a port it picked, and a port two of them pick is a port only one of them gets.
The loser's server exits on `EADDRINUSE` at once; the winner's goes on answering, correctly, on that port.

What makes it silent is the readiness check.
Starting the server in the background and then curling the page for a 200 proves that a server is listening, which was never in doubt — it does not prove the server is the one just started.
So the capture runs, the page renders, and every pixel of it belongs to another session's snapshot of another slice's work.
This cost a render that showed a page with none of the change on it, read as the change being broken, and was one edit away from being reported that way.
Nothing in the picture said so, and nothing in the exit status did either: the shots were written, the text dumped, and the harness reported success throughout.

The check has to name the tree.
The server prints the snapshot path it is serving on its first line, so grep its log for the path this run created and refuse to render when it is absent; a server that lost the port leaves a traceback there instead, which the same grep catches.
Reading the background process's exit status is not a substitute, because the shell reports the status of the pipeline the log was tee'd through rather than of the server.
Picking an unlikely port is not a substitute either — it lengthens the odds and changes nothing about what happens when they come in.

## A snapshot copied during someone else's build is two builds

The rule above says to copy `web/build` before serving it, and the readiness check says to prove the server is yours.
Neither covers the window inside the copy itself.
`cp -r web/build <snapshot>` is not atomic, and `web/build` is shared, so a build that lands while the copy is walking the tree leaves a snapshot holding `index.html` from one build and chunks from the other.

The page then goes blank, and nothing says why.
SvelteKit's shell boots from an inline script that assigns a global named `__sveltekit_<id>`, where the id is regenerated per build, and the entry chunk reads that same global back by name.
Across two builds the names disagree, so the chunk reads `undefined`, its start function throws on the first property access, and the document is left holding the shell's own markup and nothing else.
The screenshot is an empty page, the capture reports success, the probe reports a handful of elements, and the obvious reading is that the change under review broke the console.
This cost a render that was one message away from being reported as a regression in a change that turned out to be a stylesheet and two comments.

The grep in the snapshot rule cannot catch it, because both halves of a mixed snapshot contain the string the change added.
The check that does is to compare the two: extract `__sveltekit_<id>` from the snapshot's `index.html`, scan every `.js` under `_app/immutable` for the same pattern, and refuse to render when any chunk names an id the shell does not declare.
It is one pass over the snapshot and it is the difference between a blank page you can explain and one you cannot.

The same evidence tells a mixed snapshot from a real failure after the fact, which is worth knowing when a blank render has already happened: a page that failed for its own reasons still boots, so its errors carry component names and its probe carries a drawn shell, while a mixed one throws inside the framework's own entry before any of that exists.

## Only one `npm run check` in the tree at a time

`npm run check` is `svelte-kit sync && svelte-check`, and the sync step writes into `.svelte-kit/` — the generated `tsconfig.json` and the ambient type declarations the whole project compiles against.
That directory belongs to the tree rather than to the run, so two checks started at once write over each other's output, and the second one type-checks against a directory the first is still rewriting.

The symptom is not a crash and not an obviously broken tool.
It is a long, plausible list of type errors in files nobody has touched: `window` and `MouseEvent` unknown, `Object.hasOwn` missing, `svelteHTML` undefined, a module that plainly exists reported as not found.
Every one of them reads like a real regression in shared configuration, and the tempting repair — widening `lib` in `tsconfig.json`, or adding a triple-slash reference — makes the noise go away by loosening a gate that was never wrong.
Never do that.

The rule: run one `npm run check` in the tree at a time, and treat a sudden tree-wide type failure as a race until a serialised second run reproduces it.
A failure that survives a run with nothing else building is a real one and worth every minute spent on it; one that vanishes was never in the code.

## A build outside the web lane uses whichever core happens to be there

Some of the console's rules are decided by a compiled core that is generated rather than committed, and the directory holding it is ignored, so a fresh checkout has none of it at all.
Every sanctioned lane rebuilds it: `just web-check` and the dev server both depend on the recipe that produces it, and the packaged console builds it from source.
A bare `npm run build` or `npx vitest` inside `web/` does not, and both succeed anyway.
They use whatever core is already sitting there — the one the last local build left, another session's, or on a clean tree nothing — so a page can render its rules from one version of the source while every other file on screen comes from another.
Nothing in the output says which one it used, which is what makes this the same species as the stale snapshot above rather than a build failure.
So anyone rendering or testing outside the web lane runs `just web-wasm` once first, inside `nix develop`, because the default toolchain here cannot build for the browser and the recipe fails outside the development shell with an error that reads like a broken recipe rather than a missing target.

## Failing one named endpoint is how an unread state is reached

The console deliberately separates a collection that has not been read from one that is empty, and both states have to be renderable or the distinction is untested.
The harness answers `/v1` from fixtures, so the way to reach the unread state is a switch that makes one named endpoint answer 500 while every other endpoint answers normally.
Naming the endpoint is what makes the render worth taking.
A page with one unread panel shows whether the rest of the page still reads correctly around it; a page where nothing answered shows only that nothing answered.

## Dump the settled page's text as well as its picture

A screenshot answers whether the page is laid out and answers little else.
Dumping the text of the settled page beside the image turns a render into something that can be read rather than squinted at: figures, headings and empty-state sentences all become greppable, and a claim such as "the panel says the ledger has not been read" becomes checkable without opening the picture.
It is also what makes one run comparable to the last, since two text dumps diff and two screenshots do not.

## Capture at the document's height, not the viewport's

Images in this console are lazily loaded, so a capture sized to the nominal window height photographs everything below the fold as an empty box and reports a styling failure that does not exist.
Capturing at the full document height loads the images a real reader would scroll to before the shutter opens.
The same short viewport truncates the text dump, so both artefacts are taken from the full-height page for the same reason.

## Full document height is not enough at phone width

Capturing at the document's height fixes the fold, and at 1280 and 768 that is the whole of it.
At 390 it is not.
A phone-width render of the Marketplaces console makes a document over seven thousand pixels tall, and Firefox decides which lazily-loaded images to fetch before the full-page capture expands the viewport to that height, so marks far down the page are never requested and photograph as empty tiles.
The picture then reports a styling failure on a page that was rendering every one of them correctly, which is the same class of lie as the opening fade: the capture succeeds and the answer is wrong.
Four marks were lost this way before the cause was found, and nothing in the image said so.

The repair is to force eager loading in the harness immediately before the shutter, which changes when an image is fetched and nothing about how it is laid out, sized or drawn.
It belongs to the capture and never to shipped markup: the attribute is flipped by the harness's own injected script on a query switch, so the built page keeps its lazy loading and only the render sees eager.
That distinction is what separates this from suppressing the fade animation, which would have changed the thing being photographed rather than the moment of photographing it.

## A framework reset can silently un-centre a native element

The console imports Tailwind, whose preflight sets `margin: 0` on every element.
A modal `<dialog>` is centred by the user agent's own `margin: auto` acting against its `position: fixed; inset: 0`, so the reset collapses every modal to the top-left corner of the window, and the sheet's own `dialog` rule restored `padding` but not `margin`.
Nothing about that reads as a bug in the page being rendered: the dialog draws correctly, its backdrop draws correctly, and it is simply in the wrong place.

What made it survive was that one dialog looked right.
The command palette positions itself in `shell.css`, so the console showed one centred overlay and one cornered one, which reads as an inconsistency between two components rather than as a single missing declaration.
A local workaround in the component that was noticed is the thing that hides the shared cause from the components that were not.

The general form: when a framework reset is in play, a native element's user-agent defaults are not a safe baseline, and the ones that position rather than paint are the ones whose loss does not look like a style bug.
The same file had the fault twice over — a `max-width: 620px` block sitting *earlier* than the base `dialog` rule, so the phone sheet's `width`, `max-height` and corners were overridden by the desktop rule beneath it and had never once applied.
A media query adds no specificity, so ordering is the whole of it.

## The fixtures are checked against nothing

The canned `/v1` is hand-written and the Rust views it imitates are not, and no gate compares the two.
A field renamed or a variant added on the server therefore leaves the fixtures answering a shape the client no longer expects, and the harness goes on rendering a console that agrees with itself and with nothing else.
That is tolerable while the harness is something run deliberately and read with one's own eyes.
It stops being tolerable the moment a render is asked to gate a merge, because a gate resting on fixtures nobody checks is a gate that passes vacuously.
The precedent for the repair is already in the tree: `web/src/lib/generated/vocab.ts` is generated from the closed Rust enums by `cargo run -p tam-api --bin typegen`, and the web lane regenerates it into a temporary file and diffs it against the committed copy, so a drifted vocabulary fails the lane by name.
Fixtures would need the same treatment, generated from the view types or type-checked against them, before a render lane could be trusted to fail for a real reason.
