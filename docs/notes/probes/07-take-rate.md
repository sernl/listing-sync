# Probe: Tes take rate

- date: 2026-08-25
- account: EBMC, userId 28768710
- method: captured from `GET /api/tier/gmv/me` in the upload HAR
- evidence: probes/local/tes-upload.har

## Observation

The seller tier and economics are exposed by an authenticated endpoint rather than needing a manual dashboard read.
`GET /api/tier/gmv/me` returned the Bronze tier with `royaltyRate` 0.6, `transactionFee` 0.2, `sellerGmvMinimum` 0, `sellerGmvMaximum` 999.99, `maximumTransactionValueWithFee` 2.99, and features including bundles and shop.
The tier is GMV-banded, so the royalty rate rises with cumulative sales, and Bronze is the entry band.

## Answer to the gated question

At the entry Bronze tier the author receives a 0.6 royalty rate, with a fixed transaction fee component, banded by gross merchandise value.
This is a live economics input for the commercial model and supersedes any assumed split; the full band table is retrievable from the same endpoint as tiers change.

## Confidence

High for the Bronze values, which are directly captured; medium on the full band schedule until the higher tiers are read.
