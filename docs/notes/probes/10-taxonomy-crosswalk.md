# Probe: Tes taxonomy and the GB-NZ crosswalk

- date: 2026-08-25
- method: paced crawl of the public `GET /taxonomy/v4/{country}` API for GB and NZ, reference roots for US, AU, IE, CA
- evidence: docs/design/data/tes-taxonomy-GB.json, tes-taxonomy-NZ.json, tes-taxonomy-refs.json; probes/crawl-taxonomy.py

## Observation

The Tes taxonomy is a public, unauthenticated, per-country tree, and every market shares one parallel structure of 43 subjects.
GB and NZ each carry 43 subjects, with 453 and 451 topics respectively, and every one of the 43 subject descriptions is identical across the two markets.
The ids are a mechanical per-market prefix over a shared suffix: GB `1000454` "Maths for early years" corresponds to NZ `7000454` of the same description, and all 43 subjects match on suffix and description with zero mismatches.
Topics follow the same rule, matching 13 of 13 on the sampled subject, and the small 453-versus-451 difference is the only residue at topic level.
Each node also carries a `mapTo` field in a third id scheme, prefix 9, and a `phases` list of early-years, primary and secondary.

## Answer to the gated question

The GB-to-NZ taxonomy crosswalk is a deterministic id transform rather than a mapping problem: strip the market prefix, match the suffix, and the descriptions are already identical.
The taxonomy hub the design provisioned a reconciliation queue for is therefore nearly free for the founder's two target markets, and the queue would fire only for the handful of topic-level differences, not for subjects.
The full GB and NZ vocabularies are captured as data with no founder session required, closing the vocabulary probe for these markets.

## Confidence

High: the crosswalk is directly measured across all 43 subjects and verified at the topic level, and the crawl is complete per market because the bare-root children are the complete subject set.
