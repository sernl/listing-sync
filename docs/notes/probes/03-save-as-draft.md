# Probe: Tes save-as-draft

- date: 2026-08-25
- account: EBMC
- method: HAR plus author dashboard inspection
- evidence: probes/local/tes-upload.har; dashboard screenshot

## Observation

Draft is the native state of a resource, not a special mode.
The uploader operates on `/api/v2/resources/{id}/draft`, the author dashboard lists resources with an explicit "Draft" badge and the note "This is a draft. Once published, you'll see data for this resource.", and publish is the fifth wizard step.
The founder's own dashboard shows several resources sitting as drafts alongside published ones, including two entries of "Full Unit: Integers Level 3", one draft and one live at £5.00.

## Answer to the gated question

DraftSupport is Yes.
CreateStrategy is DraftThenPublish: create or update the draft by its numeric id, which is idempotent because the draft is re-POSTed wholesale on every change, then publish as a separate transition.
This collapses the ambiguous-create risk, because the file lands in S3 by key, the metadata is an idempotent draft POST, and publishing is a transition on a known id rather than a blind create.
The correlation-marker fallback is therefore not needed for Tes.

## Confidence

High that drafts exist and are the native state.
Medium on the idempotency claim until the create-draft and publish calls are captured, because whether the initial create is itself idempotent depends on how the id is minted.
