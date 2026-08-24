# Probe: Tes anti-automation terms review (M-1 kill gate)

- date: 2026-08-25
- egress: NZ consumer ISP; the en-au variant is geo-served, and the en-gb variant was fetched by explicit path
- account: unauthenticated / public documentary read
- method: fetched Tes's binding author terms and searched for anti-automation, anti-bot, anti-scraping, anti-crawler, and programmatic-access language
- evidence: probes/local/tes-general-terms.html (en-au), tes-general-terms-gb.html (en-gb), tes-additional-au.html, tes-additional-gb.html

## Observation

The binding set for a resource author is the General Terms of Business plus the Additional Terms for Tes Resources, and both were read in the en-au and en-gb variants.
The en-au General Terms are issued by Tes Aus Global Pty Limited; the en-gb variant by Tes Global Ltd; the Additional Terms are common to both and issued by Tes Education Resources / Tes Global Ltd.
Across all four documents there is no express anti-automation, anti-bot, anti-scraping, anti-crawler, or programmatic-access clause.
The keyword scan for `automat`, `scrap`, `crawl`, `spider`, `robot`, and `programmat` returned zero substantive hits; the only `automat` occurrence is "automatically renewed" in the subscription term, and the `bot` and `api` matches are substrings of "both" and "capitalised".

The three closest restrictions are none of them an automation prohibition.
The General Terms competitive-use clause forbids accessing "all or any part of a Product ... in order to build a product or service which competes with the Product" and forbids reverse-engineering a Product, where Product is Tes's own offering, so it does not reach an author using the interface to upload their own content.
The Additional Terms state "You should only upload your own content to Tes Resources", which the product satisfies by construction because it acts for the author on the author's own catalogue.
The Additional Terms indemnity attaches to authors who are corporations, companies, partnerships or institutions, who "agree to fully indemnify, defend and hold Tes ... harmless", which is a liability allocation the customer terms must mirror rather than an access restriction.

Two operational signals are recorded rather than gating.
The discretionary fair-usage upload limit is the only volume lever and carries no numeric threshold, so the enforcement risk is an account restriction rather than a clause breach.
`robots.txt` sets `Disallow: /uploader/` for all crawlers, which is crawler-scoped and governs unauthenticated access, so the design's commitment to honour robots on unauthenticated read paths stands and the authenticated-upload path is governed by the terms above rather than by robots.

## Answer to the gated question

PASS.
No express anti-automation clause binds an authenticated Tes author acting on their own account, in either the en-au or the en-gb variant, so the M-1 kill gate does not stop the build on documentary grounds.
This is the desk-read half only; the Stage-B written enquiry to Tes remains the other half of the gate, and the competitive-use and indemnity clauses should be re-read by counsel once the operating entity and jurisdiction are set.

## Confidence

High that no anti-automation clause is present in the fetched binding terms, which are directly quoted and were checked in both geo-variants.
Medium on completeness, because a clause could live in a document not linked from the policy index, and the interpretation of the competitive-use clause is a lay reading pending counsel.
