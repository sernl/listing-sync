# Probe: Tes upload request shape

- date: 2026-08-25
- egress: founder's own host; account EBMC (International market), userId 28768710
- method: HAR capture of the upload flow to save-as-draft, plus DOM inspection
- evidence: probes/local/tes-upload.har (gitignored; contains session material)

## Observation

The Tes uploader is a cookie-authenticated JSON REST API, not a browser-only form.
Every API call authenticates with the session cookie alone: no Authorization header and no CSRF or XSRF header appears on any request, and the session is refreshed through `GET /api/authn/refresh-cookies`.
The `document.querySelectorAll('input[type=file]')` count is 1, but that input is driven by an Uploadcare widget rather than a plain form post, and the file itself never transits a Tes endpoint.

The write path observed is entirely plain HTTP that a Rust client can replicate.
`POST /api/v2/resources/{id}/draft` carries the whole listing as JSON — `title`, `descriptionRaw` with `descriptionRawType:"md"`, `categories:[{id}]`, `ageRanges`, `ages`, `yearGroups`, `mainType`, `mainAge` — and is re-sent on every change as an idempotent autosave.
`POST /api/resources/v3/draft/{id}/attachment` with `[{name,tempId,previewOption}]` returns an `s3pending.params` presigned S3 POST policy.
The file is then a single `multipart/form-data` POST to `resource-attachments-live.s3.amazonaws.com` returning 204, preceded by a CORS `OPTIONS`, with no chunking, no `PUT`, and no tus or resumable protocol.
A second `POST .../attachment` confirms the upload with `isUploaded:true` and the real `filesize`.
The taxonomy is fetched from `GET /taxonomy/v4/{region}/{subjectId}`, and the seller tier from `GET /api/tier/gmv/me`.

Two facts bound the file step and the identity.
The Add Files screen states the supported types are PDF, Word, Smartboards, JPEG, Powerpoint, Excel and ePub, with a maximum file size of 200 MB.
The Description screen states that once a resource is created its URL cannot be changed, and that changing a title significantly requires deleting and reuploading, so the numeric resource id minted at creation is the durable key and the slug is fixed to the original title.

Two calls were not captured because the session ended at save-as-draft rather than publish.
The create-draft call that mints a new numeric resource id was not seen, because the captured draft already existed.
The publish transition, which is step 5 of the five-step wizard (Description, Add Files, Categories, Licence, Publish), was not seen.

## Answer to the gated question

UploadTransport is a cookie-authenticated JSON REST API plus a presigned direct-to-S3 file POST, which is the best of the anticipated outcomes and better than either branch the plan framed.
`tam-browser` is not required for the Tes adapter, and with it the per-session browser isolation, the shared-memory work, and the forced browser-upgrade cadence are removed for Tes; the adapter is a `reqwest` client against the internal JSON API.
The remaining unknowns are the create-draft and publish calls, both a short follow-up capture.

## Confidence

High for the observed flow, which is directly captured and internally consistent across the draft, attachment, S3, taxonomy and tier calls.
Medium overall until the create-draft and publish calls are captured, because the idempotency of the create and the shape of the publish transition are what finalise the correctness design.
