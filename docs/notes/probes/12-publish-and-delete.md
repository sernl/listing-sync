# Probe: Tes publish and published-resource delete

- date: 2026-08-25
- account: EBMC, founder's own; one free test resource published live and then deleted, with the founder's confirmation
- method: create + populate a draft, set a licence, publish, verify live, delete, verify gone

## Observation

Publish is `POST /api/v2/resources/{id}/publish` with an empty body, and it requires the draft to carry a valid `licence` first.
Licence is a draft field with a fixed string enum, set via `POST /api/v2/resources/{id}/draft`.
The free Creative Commons licences validate: `CC-BY`, `CC-BY-SA` and `CC-BY-ND` returned 200, while `CC-BY-NC*`, `CC0`, `PUBLIC-DOMAIN` and various guesses returned 400.
The paid licence `TES-PAID` is rejected without a price, which is why a free resource must use a Creative Commons licence rather than a Tes one.
A published resource reports `draft: false`, `isPublic: true`, and a `url` field of the form `/teaching-resource/{slug}-{id}`.

The delete taught the most important correctness lesson of the spike, observed live.
`DELETE /api/v2/resources/{id}/draft` returns 204 on a published resource but deletes only the draft overlay, leaving the published resource live.
The authoritative published-resource delete is `DELETE /api/v2/resources/{id}` without the `/draft` suffix, which returns 204 and actually removes it.
The public resource URL soft-404s: it returns HTTP 200 with a generic "Teaching resources - Tes" page when the resource is gone, so its status code is worthless for verification.
Delete and publish verification must therefore read the authoritative API, where `GET /api/v2/resources/{id}` returns 404 once the resource is gone, and must never trust the mutating call's own 2xx or the public URL's status.

## Answer to the gated question

The publish transition and the published-resource delete are both captured, completing the write path M0 deferred.
The live test confirmed a real instance of the ambiguous-success failure mode the design is built around: a 204 that means nothing and a 200 that means gone, both defeated only by reading an independent authoritative signal.

## Confidence

High: publish, the licence enum boundary, the published-delete endpoint, and the soft-404 behaviour were each directly observed, and the deletion was verified by API 404 and by the absence of the listing from the public page.
