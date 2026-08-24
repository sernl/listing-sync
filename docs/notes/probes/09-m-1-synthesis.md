# M-1 synthesis (interim)

- date: 2026-08-25
- status: kill gate passed; six of eight probes answered; two follow-up captures and the longevity window remain

## Kill gate

PASS. No express anti-automation clause binds a Tes author in either the en-au or en-gb binding terms (probe 01). The build proceeds.

## Answers

The Tes uploader is a cookie-authenticated JSON REST API plus a presigned direct-to-S3 file POST (probe 02), so `tam-browser` is not required for the Tes adapter.
Drafts are the native resource state and publish is a separate transition, so CreateStrategy is DraftThenPublish and the correlation-marker fallback is unnecessary for Tes (probe 03).
One account reaches every market through a Curriculum field value rather than separate logins, which reframes the wedge as same-account duplication (probe 04).
The vocabulary is numeric ids with a publicly crawlable subject-and-topic tree at `GET /taxonomy/v4/{country}/{id}`, plus fixed option sets for resource type, age range and curriculum captured as data (probe 05).
Deletion is irreversible, confirming Delete stays out of the adapter vocabulary (probe 06).
The Bronze seller tier pays a 0.6 royalty rate, banded by gross merchandise value (probe 07).

## Forward decisions

`tam-browser` is not built for the Tes adapter; `tam-marketplace-tes` is a `reqwest` client against the internal JSON API and S3.
The M0 spike changes shape accordingly: create a draft, upload a file via presigned S3, set metadata as a draft POST, publish, then read back and diff, all in Rust with no browser.

## Pending

Capture the create-draft call that mints a new resource id, and the publish transition, to finalise create idempotency and the publish step.
Confirm whether Curriculum American on the International account actually populates the US inventory, or whether the US site is a separate upload target, which finalises the wedge.
Run the session-longevity logger over its multi-week window (probe 08, not yet started; needs an exported cookie).
