# M0 spike synthesis

- date: 2026-08-25
- account: EBMC (userId 28768710), founder's own, no publish
- deliverable: the Tes draft write path proven server-side in Rust, the two endpoints M-1 could not capture, and the fault taxonomy

## Both milestone kill gates passed

A session established on the server from the founder's stored credentials completes authenticated writes: the Rust `reqwest` client authenticated as userId 28768710 and drove every call with the cookie alone.
An interrupted write can be made to classify as ambiguous rather than as success or failure: the three-valued `WriteOutcome` classifier passes seven tests over the six fault shapes, and a real transport error is mapped to `Ambiguous` at the call site.

## Confirmed endpoints and shapes

Create a draft: `POST /api/v2/resources` with `{}` returns 201 and a full draft object carrying a new numeric id.
Set metadata: `POST /api/v2/resources/{id}/draft` with the field shape from M-1, including `descriptionRawType: "md"` markdown and numeric taxonomy ids, and it echoes the stored state.
File upload is a three-step AWS presigned POST: `POST /api/resources/v3/draft/{id}/attachment` returns `s3pending.params` whose fields are `AwsAccessKeyId`, `key`, `policy` and `signature`; the base64 policy carries the bucket `resource-attachments-live` and starts-with conditions for `name`, `Content-Type` and `Content-Disposition`; a `multipart/form-data` POST to `https://{bucket}.s3.amazonaws.com/` with those params as fields, the three starts-with fields, and the file part last returns 204; a confirm `POST .../attachment` that echoes the full attachment object including `s3pending` with `isUploaded` set true makes the server verify the S3 object and return `isUploaded: true`.
Read back: `GET /api/v2/resources/{id}/draft` returns the resource for a field diff.
Delete: `DELETE /api/v2/resources/{id}/draft` returns 204; note `DELETE /api/v2/resources/{id}` without the `/draft` suffix is 404.

## Fault taxonomy

`WriteOutcome` is `Committed`, `Failed(reason)`, or `Ambiguous(reason)`.
Committed requires a 2xx body that is JSON carrying the expected id, a positive assertion rather than an absence check.
A 2xx without the expected id, a 2xx that is a sign-in interstitial, a truncated or non-JSON 2xx body, a 401 or 403, a 429, a 408/502/503/504, and a transport error are all `Ambiguous`.
A 400, 404 or 422 is `Failed`.

## Operational notes

Each run creates and deletes exactly one `ZZ-SPIKE-DELETE-ME` draft; the last draft was confirmed gone by a 404 on read-back, and cleanup runs even when an intermediate step errors.
A dedicated list-my-drafts endpoint was not confirmed, so straggler sweeping relies on the per-run delete, verified by the 404.
The happy path completes in a few seconds with every request under an explicit timeout.

## Go or no-go for M1

Go.
The draft write path is proven and promotes into the production `tam-marketplace-tes` crate under the full enforcement gate at M1, carrying the confirmed endpoints, the S3 handshake, the read-back diff and the `WriteOutcome` classifier.
Publish, moderation and the live state remain deferred to a supervised step on a resource the founder intends to publish.
