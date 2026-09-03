# TPT header-envelope probe (gate G1)

A read-only probe that answers the question gate G1 poses in
`docs/research/rethink/oxichrome-extension-client.md` section 6: does
TeachersPayTeachers serve the create form to a request that originates in a
browser extension, or does it answer with a Cloudflare interstitial?

It is the gate the whole extension question turns on.
`crates/tam-marketplace-tpt/src/live.rs:18-24` records that the same document
navigation was answered with an interstitial under a truncated user agent and
with the form under a real browser's headers, and it does not record which
header carried the difference.
So the probe tries four request shapes rather than assuming the expensive one,
and records what each actually sent alongside what came back.

It never fills a form, never submits anything, and carries no credential of its
own; it rides your existing signed-in session and only reads.

## What you need

Chrome or Chromium, signed in to TeachersPayTeachers as yourself, on your own
machine.
Nothing to install and nothing to build.

## Loading it

Open `chrome://extensions`, turn on Developer mode with the toggle at the top
right, click "Load unpacked", and choose this directory
(`apps/extension-probe/g1-tpt-envelope`).
Chrome will show it as "TPT header-envelope probe (gate G1)" and warn that it
can read data on `www.teacherspayteachers.com`, which is exactly what it does.
Pin it to the toolbar so its icon is reachable, then click the icon to open the
probe's panel.

## Running it

Open a tab on `https://www.teacherspayteachers.com` and sign in as normal.
Leave that tab open; routes B and C need it, and route B specifically needs a
tab that was already there rather than one the probe opened.

Then click the six buttons in the panel in order, A then B then C then D,
waiting for the line under them to change before clicking the next.
The order matters: each route is more expensive to build against than the one
before it, and the cheapest that passes is the design.

Route A asks whether the request works at all from the extension's own
background service worker, which is the shape the existing Rust adapters
transpose into almost unchanged.

Route B asks whether it works when issued from inside your open
TeachersPayTeachers tab, so it carries that page's own origin and referer but
still a fetch's envelope.

Route C asks whether a real navigation works when the extension starts it,
opening a new tab and landing it on the create form.
Leave that tab alone, do not fill anything in, and close it when the panel says
the route finished.

Route D asks the same question when your own TeachersPayTeachers tab starts the
navigation instead, which is the only shape that sends what a browser sends when
you click a link on the site yourself.
It navigates the tab you already had open, so you will see it move to the create
form; that is expected, and nothing is submitted.

## Sending the result back

Click "Show everything recorded", then either "Copy to clipboard" and paste it
into a message, or "Save as file", which writes `g1-tpt-envelope.json` to your
downloads folder.
Send that one file back.
That is the whole result; there are no other files.

If a route fails, send the file anyway.
A failure is a finding, and the report records what was sent and what came back
either way.

## What is in the file, and what is deliberately not

For each route the report carries the request headers the browser actually sent
(`origin`, `referer`, `user-agent`, `accept-language` and every `sec-fetch-*`),
the response status, every `cf-*` response header, and a verdict on the page.

Four things are recorded by name only, never by value, because you are sending
this file to someone else.
Your cookies appear as a list of cookie names with the values stripped.
`set-cookie` responses appear the same way.
Every request or response header not on a short allowlist of non-sensitive
names is recorded as present without its value.
Page text is captured only when a bot-protection marker fired, and then only
the first 300 characters, because that page is Cloudflare's rather than yours.

You can confirm all of this before sending: open the file and search it for any
value you recognise as private.

## Reading the verdict

Each route reports one of three words, on the same scale
`tools/login-probe` uses.
CHALLENGE means at least one bot-protection marker was found, and the report
names each one and where it was seen.
CLEAR means no marker, and the page carried either a password input or the
create form.
UNKNOWN means neither, which usually means the page had not finished loading.

CHALLENGE on the login page is expected and is not a failure: Cloudflare's
bot-management script and the reCAPTCHA widget are always present there, which
is why the report separates `wall` from the markers.
`wall: true` means a challenge with nothing usable behind it, and that is the
failing shape.
The gate's pass condition is `formServed: true` on the create form, which means
all five of the hidden inputs
`crates/tam-marketplace-tpt/src/form.rs` scrapes were present, so a write could
have proceeded.

The report also answers one side question that costs nothing to collect:
`csrfTokenReadableFromDocumentCookie` says whether the `csrfToken` cookie is
visible to page script.
If it is, the transport shim can read its own CSRF token in place; if it is not,
the token has to be fetched in the background and messaged across on every
request.
Only routes B and C can answer this, because only they run inside a page.

## What this probe does not settle

All four routes issue GET requests, and the Fetch specification attaches an
`Origin` header only to requests that are not GET or HEAD.
So the report will show `origin` absent on every route, and the question of what
`Origin` a POST from an extension context carries — which
`docs/research/rethink/oxichrome-extension/r2-platform.md` F3 answers only by
inference — stays open after this run.
That is deliberate rather than an oversight: every POST TeachersPayTeachers
accepts on these paths is a write, and this probe reads and never writes, so
there is no harmless POST for it to make.

## Verifying it without touching the marketplace

`apps/extension-probe/verify-g1.sh` runs all six route invocations against a local server
that imitates the four page shapes — a login page with a password input, a
Cloudflare-like interstitial, a redirect, and a create form carrying the five
hidden inputs — and never contacts a marketplace.
It drives the panel by opening it with `autorun`, `open` and `report` query
parameters, which the panel accepts only from the local harness: the `report`
destination is refused unless it addresses `127.0.0.1`, so that affordance
cannot send anything off the machine.
Verified on Chromium 151.0.7922.71 from nixpkgs.
