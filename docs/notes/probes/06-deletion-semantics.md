# Probe: Tes resource deletion semantics

- date: 2026-08-25
- account: EBMC
- method: author dashboard inspection; founder-reported deletion outcome
- evidence: dashboard screenshot

## Observation

Deletion is available from the author dashboard, where each resource offers Edit, Change price and Delete.
The founder reports that deletion was possible and is not reversible.
The uploader separately warns that a resource URL cannot be changed after creation, and that changing a title significantly requires deleting and reuploading as a new resource, so deletion is the sanctioned path for a title change as well as for removal.

## Answer to the gated question

Deletion is irreversible and destructive, which confirms the design decision to keep Delete out of the adapter action vocabulary and to treat any delete as an operator-gated action rather than an automated one.
The blast radius of an erroneous automated delete is the permanent loss of a live resource and its URL, so the near-zero account-safety tolerance applies directly here.
Price change is a distinct dashboard action and is a candidate for a separate, non-destructive adapter capability later.

## Confidence

High, corroborated by the dashboard controls and the uploader's own delete-and-reupload guidance.
