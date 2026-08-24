# Probe: Tes dual-inventory access, and the market model

- date: 2026-08-25
- account: EBMC (top navigation reads "International")
- method: upload-flow and classification-screen inspection
- evidence: classification screenshots; HAR

## Observation

There is one account and one cohesive upload flow, with no GB-versus-US login or inventory switch visible.
The market or audience of a resource is expressed as a field value rather than as a separate login: the Curriculum dropdown offers None, No curriculum, American, Australian, Canadian, English, International, Irish, New Zealand, Northern Irish, Scottish, Welsh and Zambian.
The founder's account is on the International market, and the subject vocabulary shown includes Australian-curriculum subjects such as Aboriginal studies.

This reframes the wedge.
The design's "GB-to-US inventory duplication" is, on this evidence, the same account and the same JSON API producing a second resource whose Curriculum is American and whose text is US-English, rather than a separate US login, inventory, or upload system.
The disjoint GB and US resource-id spaces the research measured are consistent with this, because each market-targeted copy is a distinct resource with its own id.

## Answer to the gated question

InventoryReach is OneLoginBoth: one Tes author account reaches every market through the Curriculum field, so `Connection`, the session lease, the write budget and revocation are keyed per account, not per inventory.

## Open question this raises

Whether tagging Curriculum American on the International site actually places a resource in the US inventory that the US site surfaces, or whether the US inventory requires a separate US-site upload, is not settled by this capture and is the single most important wedge question to confirm before M1.

## Confidence

High that it is one account and one flow with a Curriculum field.
Low that Curriculum American alone populates the US inventory; that specific causal link is unverified and must be checked against the live US site.
