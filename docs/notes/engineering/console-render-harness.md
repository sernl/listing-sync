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

## The fixtures are checked against nothing

The canned `/v1` is hand-written and the Rust views it imitates are not, and no gate compares the two.
A field renamed or a variant added on the server therefore leaves the fixtures answering a shape the client no longer expects, and the harness goes on rendering a console that agrees with itself and with nothing else.
That is tolerable while the harness is something run deliberately and read with one's own eyes.
It stops being tolerable the moment a render is asked to gate a merge, because a gate resting on fixtures nobody checks is a gate that passes vacuously.
The precedent for the repair is already in the tree: `web/src/lib/generated/vocab.ts` is generated from the closed Rust enums by `cargo run -p tam-api --bin typegen`, and the web lane regenerates it into a temporary file and diffs it against the committed copy, so a drifted vocabulary fails the lane by name.
Fixtures would need the same treatment, generated from the view types or type-checked against them, before a render lane could be trusted to fail for a real reason.
