# Education standards: authoritative sources, licences, structure, and ingestion cost

Every URL below was fetched 2026-09-02 unless a different date is stated beside it.
Counts marked "measured" were derived by parsing a response captured on that date; the derivation is stated where it is not obvious.

## Executive summary

1. Only one of the four frameworks has an official machine-readable feed from its own owner: TEKS, published by the Texas Education Agency as a CASE-certified REST API at `teks-api.texasgateway.org`, which answered unauthenticated on 2026-09-02.
2. Common Core's official XML is dead — the documented "append XML to any identifier URL" endpoint now returns the site's 404 page, and `corestandards.org` is today a four-PDF WordPress stub while `www.thecorestandards.org` carries the browsable standards behind Cloudflare bot protection.
3. NGSS publishes no machine-readable form at all; `nextgenscience.org` offers two PDFs (DCI arrangements, topic arrangements) and a paginated search UI.
4. Virginia publishes PDF and Word only, and `doe.virginia.gov` returns HTTP 403 to every non-browser client we tried; Virginia's CASE encoding exists but is produced by a third party, not by VDOE.
5. The licences differ sharply and one of them matters commercially: Common Core's public licence grants the right to "copy, publish, distribute, and display" and never names modification or derivative works, and requires the NGA Center/CCSSO copyright notice on any publication or public display.
6. NGSS's public licence — the permissive "copy, reproduce, alter, adapt, edit, delete and rearrange … without permission" grant — is addressed to states, districts, schools, teachers and non-profit education entities, which we are not; commercial users fall under a separate trademark regime requiring a disclaimer on the home page and every internal page that prominently uses the mark, plus sample submission to WestEd with 4-6 weeks' processing.
7. The practical primary source for three of the four is a mirror, not the owner: the Common Standards Project's open API (no key, CC BY, ASN identifiers) carries current-cycle CCSS, NGSS and Virginia data, and TEA's own API carries TEKS.
8. No source in this ecosystem publishes a cross-framework crosswalk: TEA's CASE packages contain only `isChildOf` associations, and the Common Standards Project's `exactMatch` field points a standard at its own canonical URI, not at a counterpart in another framework.
9. TPT binds standards by an opaque internal numeric node id (read back as `sphinxId`), not by published code, per `docs/research/rethink/tpt-product-model.md`; carrying only the published code would leave us unable to post a selection, so the mapping table from code to TPT node id is the real deliverable.
10. Ingestion is roughly 9-14 engineering days for the four frameworks under the assumptions in "Ingestion design and cost"; TEKS is the cheapest because it is already CASE, and Common Core is the hardest because its only current machine-readable form is somebody else's mirror of a document whose owner has stopped publishing data.

## What we may do with each framework

| Framework | Owner | Grant we can rely on | Attribution we must render | Modification |
|---|---|---|---|---|
| CCSS | NGA Center for Best Practices and CCSSO | Copy, publish, distribute, display, for purposes that support the Initiative | "© Copyright 2010. National Governors Association Center for Best Practices and Council of Chief State School Officers. All rights reserved." | Not granted; the licence never names it |
| NGSS | Trademarks WestEd (from Achieve, May 2020); copyright National Academies Press | Public licence names non-profit education entities, not commercial ones | Asterisk plus the WestEd disclaimer footnote, on the home page and every internal page prominently using the mark | Granted to the named non-profit classes only |
| TEKS | Texas Education Agency (19 TAC, state law) | The underlying rules are Texas Administrative Code; TEA's API declares `licenseURI: null` | None declared in the data | Not addressed in the data |
| VA SOL | Virginia Board of Education / VDOE | Not stated on any page we could reach | Not stated | Not stated |

The TEKS row carries a live tension. The `teks.texasgateway.org` Terms of Service state that "Registered users may download Site Content from the Site only for such user's own personal, noncommercial use or as otherwise permitted by applicable law (such as fair use)" and "You agree not to scrape, or otherwise download in bulk, any Site content." Those terms govern the documentation site and the API domain by their own definition of "the Site" (`teks.texasgateway.org` and `teks-api.texasgateway.org`). The TEKS themselves are Chapter 110-128 of Title 19 of the Texas Administrative Code — state law, whose text carries no copyright the way a private work does. Which of those two facts controls a commercial reseller tool that caches TEKS codes is a legal question, not an engineering one, and it is the first item in "Open questions for the founder".

## Common Core (ELA/Literacy and Mathematics)

### The sites today

Two domains are live and they are not the same thing.

`https://corestandards.org/` (fetched 2026-09-02) is a WordPress installation whose entire link inventory is four PDFs — `ADA-Compliant-ELA-Standards.pdf`, `ADA-Compliant-Math-Standards.pdf`, `ELA_Standards1.pdf`, `Math_Standards1.pdf` — plus the WordPress REST and XML-RPC endpoints.
It has no standards browser and no data.

`https://www.thecorestandards.org/` (fetched 2026-09-02) carries the full browsable standards, the public licence, and the developer documentation.
Its `sitemap.xml` enumerates 2,035 URLs, of which 763 are under `/Math/` and 1,155 under `/ELA-Literacy/` (measured by parsing the sitemap).
The site sits behind Cloudflare bot protection: a request without a full browser User-Agent receives the "Just a moment…" interstitial rather than the document, which we observed on a repeat fetch of `sitemap.xml`.
The site's own footer links its branding guidelines to `web.archive.org`, which is a signal that this deployment is a re-host of an archived site rather than an actively maintained one.

### The public licence

`https://www.thecorestandards.org/public-license/` (fetched 2026-09-02) grants, verbatim:

> The NGA Center for Best Practices (NGA Center) and the Council of Chief State School Officers (CCSSO) hereby grant a limited, non-exclusive, royalty-free license to copy, publish, distribute, and display the Common Core State Standards for purposes that support the Common Core State Standards Initiative. These uses may involve the Common Core State Standards as a whole or selected excerpts or portions.

Four verbs, and modification is not among them.
The attribution clause requires that "NGA Center/CCSSO shall be acknowledged as the sole owners and developers of the Common Core State Standards, and no claims to the contrary shall be made", and that "Any publication or public display shall include the following notice: '© Copyright 2010. National Governors Association Center for Best Practices and Council of Chief State School Officers. All rights reserved.'"
States that adopted CCSS in whole are exempt from the notice requirement; we are not a state.
The licence terminates automatically on breach, is construed under District of Columbia law, and names Washington DC as the exclusive forum.
A separate clause excludes the examples from the grant, because some are third-party copyrighted material licensed from Penguin Group (USA) and McGraw-Hill.

The consequence for us is narrow and specific: we may display the standard's code and statement text verbatim, and we must render the copyright notice wherever we do.
Rewriting a standard's statement — shortening it for a dropdown, paraphrasing it in generated listing copy — is not covered by the four granted verbs.

### Identifier scheme and hierarchy

`https://www.thecorestandards.org/developers-and-publishers/` and `https://www.thecorestandards.org/common-core-state-standards-official-identifiers-and-xml-representation/` (both fetched 2026-09-02) define three parallel canonical identifiers, published simultaneously and revised 2013-05-21:

- Dot notation, for example `CCSS.Math.Content.6.EE.A.1` and `CCSS.ELA-Literacy.RL.2.1`. The `CCSS.` prefix is official; the bare `RL.2.1` seen in the PDFs is conversational.
- Dereferenceable URIs at the corestandards.org domain. The documented 2010-style form (`https://thecorestandards.org/2010/math/content/6/EE/1`) returns the 404 page today; the current-style form (`https://www.thecorestandards.org/Math/Content/6/EE/A/1/`, `https://www.thecorestandards.org/ELA-Literacy/RL/2/1/`) returns HTTP 200. Both probed 2026-09-02.
- GUIDs, for example `A7D3275BC52147618D6CFEE43FB1A47E`, intended for databases.

The two disciplines have deliberately different hierarchies, which the owners state they will not reconcile:

| Level | Math | ELA/Literacy |
|---|---|---|
| 1 | Initiative | Initiative |
| 2 | Framework | Framework |
| 3 | Set (content or practice) | Set (optional; anchor standards) |
| 4 | Grade | Strand+Domain |
| 5 | Domain | Grade |
| 6 | Cluster | Standard |
| 7 | Standard | Component (optional) |
| 8 | Component (optional) | — |

In ELA/Literacy a single level carries strand and domain together, because a domain such as Reading Standards for Literature (RL) already implies the Reading strand.
Math has clusters and no strands; ELA/Literacy has strands folded into domains and no clusters.
Any schema that forces one shape onto both will misrepresent one of them.

### Machine-readable availability

The developer page states that "The XML representation of the standards and the embedded metadata within the HTML pages is available at www.corestandards.org. To access the XML and metadata, append 'XML' to any of the identifier URLs", and that the XML follows the CEDS schema.
That endpoint is dead.
`https://www.thecorestandards.org/Math/Content/6/EE/A/1/XML` and the trailing-slash variant both returned the site's 404 page on 2026-09-02.
Grepping the live HTML for the standard page shows no CEDS microdata, no `itemprop` attributes and no GUID — only ordinary page markup.

So there is today no official machine-readable Common Core distribution.
What exists is the HTML tree, enumerable from `sitemap.xml`, behind bot protection.

### Counts

Measured from 38 Common Core standard sets pulled from the Common Standards Project API on 2026-09-02 (see "The distribution ecosystem"):

- 3,590 distinct standard nodes across all sets.
- 1,812 distinct `statementNotation` values, of which 664 are `CCSS.Math.*` and 1,135 are `CCSS.ELA-Literacy.*`.
- By label: 967 Standard, 569 Component, 201 Cluster, 87 Domain, 1,062 unlabelled container nodes.

That gives roughly 1,536 addressable leaf codes (Standard plus Component) — the granularity a seller actually tags at.
Cross-checked against the live sitemap: 748 URLs under `/Math/Content/`, 9 under `/Math/Practice/` (the eight Standards for Mathematical Practice plus an index), and 1,155 under `/ELA-Literacy/`, distributed across 12 strand roots (L 268, W 258, RI 123, RL 124, SL 120, RF 76, WHST 67, RH 35, RST 35, CCRA 37, plus introductions).

### Revision date

The standards were published in 2010 and have never been revised.
The identifier scheme was revised 2013-05-21.
The owners defined a revision suffix (`CCSS.ELA-Literacy.RF.4.4r2`) for future refinements; no code in the corpus uses it.
Treat CCSS as frozen and plan for zero update cadence, with the caveat that individual states have replaced CCSS with their own successor standards, which is a state-framework problem and not a CCSS one.

## Next Generation Science Standards

### Owner and site

`https://www.nextgenscience.org/` (fetched 2026-09-02) is operated by NextGenScience at WestEd.
The trademarks — the words NEXT GENERATION SCIENCE STANDARDS and the logo — were transferred from Achieve, Inc. to WestEd in May 2020.
Copyright in the standards is held by the National Academies Press.
The standards were developed by 26 lead states in a process managed by Achieve.
The required citation is: NGSS Lead States. 2013. *Next Generation Science Standards: For States, By States*. Washington, DC: The National Academies Press.

### Licence terms

`https://www.nextgenscience.org/trademark-and-copyright/trademark-and-copyright` (fetched 2026-09-02) carries three separate regimes.

The public licence is addressed as "A message for states, districts, schools, teachers and non-profit education entities" and grants that those parties "may copy, reproduce, alter, adapt, edit, delete and rearrange any and all parts of the NGSS as they see fit and without permission".
A commercial reseller tool is not in that list.

The commercial trademark guidelines are considerably more demanding than the non-profit ones:

- The NGSS logo may not be used at all without WestEd's express written consent.
- The ® and ™ symbols may not be used; an asterisk plus a footnote is mandatory instead.
- The mark may not appear in a product's main title, in a company name, in a domain name, in metatags, or as a purchased keyword, and may not be used generically in lower case within a sentence.
- The mark must be smaller and visually subordinate to our own marks.
- The disclaimer footnote is prescribed: "________ is a registered trademark of WestEd. Neither WestEd nor the lead states and partners that developed the Next Generation Science Standards were involved in the production of this product, and do not endorse it."
- For web sites, the footnote "must appear on the web site home page and on all internal web pages that first prominently use the NGSS mark", placed at the bottom of the page, and placing it only in Terms and Conditions "does NOT satisfy these Guidelines".
- Before use, samples of the home page and the page where the mark first appears prominently must be submitted to `nextgenscience@wested.org`, allowing 4-6 weeks.

That last set of bullets is a product requirement, not a footnote: shipping an NGSS picker means a disclaimer in the site chrome and a WestEd review with a six-week lead time.

### Structure

NGSS is built from three dimensions combined into performance expectations.
Performance expectations are the addressable, taggable unit; the dimensions are what a PE is composed of.

- Performance expectations, coded grade-then-discipline-then-number: `K-PS2-1`, `3-PS2-1`, `MS-LS1-1`, `HS-ESS1-1`. Elementary uses the literal grade (`K`, `1`…`5`); middle and high school use `MS` and `HS` grade bands.
- Disciplinary core ideas, coded `1-ESS1`, `MS-LS1`, and the like, in four domains: Physical Sciences (PS), Life Sciences (LS), Earth and Space Sciences (ESS), Engineering, Technology and Applications of Science (ETS).
- Science and engineering practices — 8 of them.
- Crosscutting concepts — 7 of them.

Two published arrangements exist over the same PEs: the DCI arrangement and the topic arrangement.
They are two orderings of one set, not two sets, and a taxonomy that stores them as two trees will double-count every PE.

### Counts

Measured from 64 NGSS standard sets pulled from the Common Standards Project API on 2026-09-02:

- 208 distinct performance-expectation codes.
- 61 distinct disciplinary-core-idea codes.
- 8 practices, 7 crosscutting concepts, 10 categories.
- 3,350 distinct nodes in total across all sets, of which the great majority are the connection and crosscutting statement rows that hang off each PE.

208 is the number that matters: it is small enough that an NGSS picker can be a browsable tree with no search at all.

### Machine-readable availability

None from the owner.
`https://www.nextgenscience.org/search-standards` (fetched 2026-09-02) offers a keyword search plus filters for grade band, practice, disciplinary core idea, discipline and crosscutting concept, paginated across 34 pages, and two downloads — "Download DCI Arrangements (4 MB)" and "Download Topic Arrangements (5 MB)", both PDF.
No CSV, no XML, no JSON, no API.

### Revision

Published 2013, never revised.
As with CCSS, the volatility is in state adoption rather than in the framework.

## Texas Essential Knowledge and Skills

### The source

The TEKS are Chapters 110 through 128 of Title 19, Part 2 of the Texas Administrative Code, adopted by the State Board of Education.
The canonical legal text lives with the Texas Secretary of State; the previously documented host `texreg.sos.state.tx.us` now serves a "This Site Has Moved" redirect page (fetched 2026-09-02), so the TAC URL of record needs re-confirming.
`https://tea.texas.gov/curriculum-and-instruction/texas-essential-knowledge-and-skills-teks` (fetched 2026-09-02) is the agency's human-readable index, offering web and PDF versions.
`https://www.teksguide.org/` (fetched 2026-09-02) is the TEKS Guide, a teacher-facing resource layer over the standards rather than a data source.

### The machine-readable feed

This is the one genuinely good source in the set.

`https://teks.texasgateway.org/` (fetched 2026-09-02) states that "The Texas Education Agency provides access to the Texas Essential Knowledge and Skills (TEKS) in a IMS Global CASE-Certified, machine-readable format" and that consumers "can easily access up-to-date versions of the TEKS … by downloading the provided CSV files, or using the API".
The CSV path requires a login (`https://teks.texasgateway.org/csv` returned HTTP 403 unauthenticated on 2026-09-02).
The API did not.

`https://teks-api.texasgateway.org/ims/case/v1p0/CFDocuments` returned HTTP 200 with 24,618 bytes of JSON and no credentials on 2026-09-02.
It enumerates 19 CFDocuments: the English-language chapters 110 (ELA and Reading), 111 (Mathematics), 112 (Science), 113 (Social Studies), 114 (Languages Other Than English), 115 (Health), 116 (Physical Education), 117 (Fine Arts), 120 (Other TEKS), 126 (Technology Applications), 127 (CTE), 128 (Spanish Language Arts and ESL), the Prekindergarten Guidelines in two versions, and five Spanish-language chapters.
Each document carries `creator: "Texas Education Agency"`, an `officialSourceURL` pointing at the TAC chapter, a `lastChangeDateTime`, version notes naming the SBOE actions that produced each revision, and `licenseURI: null`.

Package sizes, measured by fetching `GET /ims/case/v1p0/CFPackages/{id}` on 2026-09-02:

| Chapter | Bytes | CFItems | CFAssociations | Student Expectations |
|---|---|---|---|---|
| 110 English Language Arts and Reading | 4,470,735 | 2,271 | 2,271 | 1,689 |
| 111 Mathematics | 3,159,233 | 1,515 | 1,515 | 1,081 |
| 112 Science | 2,956,737 | 1,420 | 1,420 | 855 |
| 113 Social Studies | 4,842,985 | 2,408 | 2,408 | 1,375 |

Five thousand Student Expectations across the four core chapters alone.
The remaining 15 documents were not measured.

### Identifier scheme

CASE `humanCodingScheme`, which for TEKS is the TAC citation flattened with dots: `111.2.b.1.A` is Chapter 111, section .2 (Kindergarten), subsection (b), knowledge-and-skills statement (1), student expectation (A).
The intermediate levels carry their own codes: `111.2.b` is the Knowledge and Skills container, `111.2` the Grade/Course.
Item types observed in Chapter 111: Subchapter, Implementation, Grade/Course, Strand, General Requirements, Introduction, Knowledge and Skills, Student Expectation.
Chapter 113 additionally contains 162 items with a null `CFItemType`, which is a data-quality wrinkle worth handling rather than trusting.

Each item also carries a CASE GUID (`identifier`) and a dereferenceable `uri` under `teks-api.texasgateway.org/ims/case/v1p0/CFItems/{guid}`.
The GUIDs appear to be deterministic (UUIDv5-shaped), which matters because it suggests they survive republication.

### Revision cycle

The SBOE reviews and revises the TEKS subject by subject on a published multi-year plan; TEA states an SBOE-approved 10-year schedule running through 2030-31 (`https://tea.texas.gov/academics/curriculum-standards/teks-review/teks-review-and-revision`, via search 2026-09-02).
The `lastChangeDateTime` values in the CFDocuments give the empirical cadence: Chapter 127 (CTE) 2026-08-07, Chapter 113 (Social Studies) 2025-07-16, the 2022 Prekindergarten Guidelines 2026-08-07, Chapter 112 (Science) 2021-08-05, Chapter 126 (Technology Applications) 2021-05-26, Chapter 110 (ELA) 2020-08-24, Chapter 111 (Mathematics) 2019-08-26.
A social studies revision is active right now: SB 24 of the 89th Legislature (2025) required new Grades 4-12 social studies TEKS, the SBOE approved key topics in January 2026 and approved proposed new sections for first reading in April 2026.
The Common Standards Project's subject labels corroborate the churn, carrying separate TEKS vintages such as "Science (2010-2017)", "Science (2017-2020)", "Science (2020-)", "Mathematics (2012-)" and "Mathematics (2025-)".

Practical consequence: TEKS is the only one of the four that genuinely moves, and a listing tagged to a retired TEKS code will need a migration story.

## Virginia Standards of Learning

### The source

VDOE publishes the SOL on `doe.virginia.gov`.
We could not read it.
Every attempt on 2026-09-02 — `www.doe.virginia.gov` and `doe.virginia.gov`, home page and deep links, via WebFetch and via curl with a full browser User-Agent — returned HTTP 403 from Akamai ("Access Denied … Reference #18.cff23717…").
The publication formats are therefore reported from secondary evidence and from the source URLs embedded in mirror data: PDF and Word documents, with no data format.
One such embedded source URL, from a current-cycle Common Standards Project record, is `https://www.doe.virginia.gov/home/showpublisheddocument/48908/638325354847170000` — a numbered document download, which is what a document-management CMS emits, not a data endpoint.

### Revision cycle

Virginia's Standards of Quality require the Board of Education to review the Standards of Learning in every subject at least once every seven years.
Recent adoptions: Mathematics 2023 (approved 2023-08-31, full implementation 2024-25), History and Social Science 2023 (approved 2023-04-20), English 2024, Computer Science 2024 (full implementation by 2025-26), Science 2018 with new high-school science courses in 2025.
This is a seven-year rolling cycle across subjects, which in practice means one or two subjects change every year.

### Identifier scheme

Virginia has no single scheme; each subject has its own, and they are not consistent with one another.
Measured from current-cycle Common Standards Project records on 2026-09-02:

- Mathematics 2023: strand-bearing codes such as `3.PFA.1.e` (grade, strand, standard, bullet) and `1.CE.1.a`. Strand abbreviations include NS (Number and Number Sense), CE (Computation and Estimation), MG (Measurement and Geometry), PS (Probability and Statistics), PFA (Patterns, Functions and Algebra).
- English 2024: `1.C.1.A.i` — grade, strand letter, standard, sub-standard, roman-numeral sub-sub-standard, five levels deep.
- History and Social Science 2023: bare `1.1`, `1.1.a`.
- Science 2018: course-prefixed Roman-numeral codes such as `ENS.V.D` for Environmental Science, alongside grade-numbered codes elsewhere.

A single regex will not parse Virginia.
The hierarchy has to come from the data's parent links rather than from the code string.

### Counts

Measured from Common Standards Project current-cycle sets on 2026-09-02:

| Subject | Sets | Nodes | Distinct codes |
|---|---|---|---|
| Mathematics (2023-) | 20 | 1,763 | 1,660 |
| English (2024-) | 13 | 1,594 | 1,467 |
| History and Social Science (2023-) | 13 | 1,115 | 1,040 |
| Science (2018-) | 18 | not aggregated | not aggregated |

Roughly 4,200 addressable codes across the three core subjects measured, before Science.
Virginia is comparable in size to TEKS and larger than Common Core.

### Machine-readable availability

Not from VDOE.
Virginia's SOL do exist in CASE format, produced and maintained by Common Good Learning Tools rather than by the Commonwealth: the Satchel Rosetta Exchange hosts, for example, "Science Standards of Learning for Virginia Public Schools (2018)" at `https://rosetta.commongoodlt.com/f228b6ac-1313-11eb-80c4-0242c0a84003/5c8d09e6-1e93-11eb-ad73-0242c0a85003/373`.
Common Good Learning Tools' own documentation distinguishes "green" agencies, which publish their own CASE frameworks, from "blue" agencies, whose CASE frameworks "are currently published and maintained by Common Good Learning Tools, and are available only via Rosetta Exchange".
Virginia is blue.
That is a provenance fact we must record, because it means the machine-readable Virginia data is a third party's transcription of a PDF, and its fidelity is that third party's claim rather than the Commonwealth's.

## The distribution ecosystem

### 1EdTech CASE

CASE (Competencies and Academic Standards Exchange) is the 1EdTech specification for encoding competency frameworks as JSON plus a REST API.
The data model is four objects: `CFDocument` (the framework), `CFItem` (a node), `CFAssociation` (an edge, typed — `isChildOf`, `exactMatchOf`, `isRelatedTo` and others), and `CFPackage` (document plus all items and associations, retrievable in one call via `getCFPackage`).
Version 1.0 has a JSON-LD binding and a JSON Schema; version 1.1 adds fields and relaxes consumer certification so a consumer need support only `getCFItem` or `getCFPackage`.
TEA's API is CASE v1p0 with per-document version notes recording v1.1.2 and v2.1.1 content revisions.

This is the shape to normalise into.
It is a directed labelled graph with a tree spine, which is exactly what `tam-taxonomy` already models with `CanonicalTerm.parent` and `ProjectionEdge`.

### CASE Network 2, and its succession

The public CASE Network — browse-free, register-for-API — is in the middle of a transition, and any design that names it will date quickly.
`https://www.1edtech.org/program/casenetwork2` (fetched 2026-09-02) states that "The name 'CASE Network 2' is being phased out" and that Satchel Rosetta Exchange, by Common Good Learning Tools, "will remain available for accessing U.S. K-12 standards and more".
The same page announces a "CASE Global Ecosystem" initiative, described as shared public infrastructure, announced at the 2026 Digital Credentials Summit with a launch planned later in the year.
It lists EdGate, LearningMate and Instructure as CASE providers.

Satchel Rosetta Exchange launched 2026-02-18 with over 1,000 free-to-explore frameworks covering all 50 US states plus other countries and issuing agencies.
Browsing is free; download and API access require registration.
We probed `https://rosetta.commongoodlt.com/ims/case/v1p0/CFDocuments` on 2026-09-02 and received HTTP 403 with body `{"message":"Invalid credentials provided 1"}` — so the CASE API there is credentialed, confirming the registration gate.
Reported commercial terms, which we did not verify against a price sheet: paying subscribers ("#CASEbelievers") contribute "no more than $8,400 per year", and CASE conversion services for an agency's own documents run "typically $500-1500 per PK-12+ subject area".
Google Classroom began sourcing its learning standards from Satchel Rosetta Exchange in May 2026, which is a useful signal about the platform's durability.

### The Common Standards Project

`https://commonstandardsproject.com/` with an API at `https://api.commonstandardsproject.com/` (both fetched 2026-09-02), sponsored by Common Curriculum.
It is alive, it is open, and it needed no API key for anything we did.

- `GET /api/v1/jurisdictions` returned 778 jurisdictions on 2026-09-02: 372 schools, 337 organizations, 65 states, 2 countries, 1 corporation, 1 nation.
- All four of our frameworks are present: "Common Core State Standards" (`67810E9EF6944F9383DCC602A3484C23`), "Next Generation Science Standards" (`71E5AA409D894EB0B43A8CD82F727BFE`), "Texas" (`28903EF2A9F9469C9BF592D4D0BE10F8`), "Virginia" (`27D1AF28D2F54DD3A69C057DDA772BD0`).
- `GET /api/v1/jurisdictions/{id}` returns the jurisdiction's standard sets: 38 for CCSS, 64 for NGSS, 697 for Texas, 534 for Virginia. Texas and Virginia sets are labelled with their vintage ("Science (2020-)", "Mathematics (2023-)"), so both current and superseded cycles are addressable.
- `GET /api/v1/standard_sets/{id}` returns the standards, each carrying `id` (its own GUID), `asnIdentifier` (Achievement Standards Network id), `statementNotation` (the published code), `altStatementNotation`, `statementLabel` (Standard, Component, Cluster, Domain, Performance Expectation, …), `description` (the statement text), `depth`, `parentId`, `ancestorIds`, `listId`, `position` and `exactMatch`.
- Each set declares a licence in the response: `{"title": "CC BY 3.0 US", "rightsHolder": "D2L Corporation"}` for the Common Core set we inspected, `{"title": "CC BY 4.0 US", "rightsHolder": "Common Curriculum, Inc."}` for the Virginia set. The D2L attribution reflects the Achievement Standards Network lineage.

We pulled all 102 CCSS and NGSS sets on 2026-09-02 without hitting a rate limit at roughly 5 requests per second, which is the basis for every CCSS and NGSS count in this document.

The catch is provenance.
CSP's document metadata for CCSS names `sourceURL: http://www.corestandards.org/assets/CCSSI_Math%20Standards.pdf` — the PDF — and its NGSS document titles are a jumble ("NGSS DCI Combined 11.6.13", "NCSS DCI Combined", "Crosscutting Concepts at NSTA") that reads like an accreted import rather than a curated release.
It is a good, free, structurally sound mirror.
It is not an owner.

### OpenSALT

`https://github.com/opensalt/opensalt` — an MIT-licensed CASE server created by Public Consulting Group, PHP running in Docker, last updated 2026-01-27 (via search 2026-09-02), with a reference instance at `opensalt.net`.
It is a tool for publishing and managing CASE frameworks, not a data source.
We would use it only if we decided to host our own CASE endpoint, which nothing in the current product requires.

### Academic Benchmarks (Instructure)

The commercial option, now sold as Elevate Standards Alignment.
It claims more than 4 million learning standards and issues the AB GUID, which it describes as adopted by more than 200 publishers and applications.
Access needs a Partner ID and Partner Key; pricing is not published and requires sales contact; features are tiered (the associations relationship that links to CASE identifiers requires a Professional licence).
There is a free tier covering one state plus CCSS plus NGSS, which is close to but not identical with what we need — it would leave TEKS or VA SOL uncovered depending on which state we picked.

### Best primary source per framework

| Framework | Best source | Why |
|---|---|---|
| CCSS | Common Standards Project API, cross-checked against `www.thecorestandards.org` HTML | The owner publishes no data; CSP is open, keyless, CC BY, and carries the official dot notation. The HTML tree is the authority for statement text and the licence notice. |
| NGSS | Common Standards Project API for the 208 PEs, cross-checked against the nextgenscience.org PDFs | Same reason. 208 rows is small enough to verify by hand against the PDF. |
| TEKS | TEA's own CASE API at `teks-api.texasgateway.org` | It is the owner, it is CASE-certified, it is current, and it carries the SBOE version notes. Subject to the terms-of-service question. |
| VA SOL | Common Standards Project API, with Satchel Rosetta as the second opinion | VDOE publishes documents and blocks automated clients. Both machine-readable options are third-party transcriptions; using two lets us diff them. |

## Cross-walks

No source we examined provides a cross-framework mapping, and one of them says so explicitly.

TEA's CASE packages contain only `isChildOf` associations — 1,515 of 1,515 in Chapter 111, 2,271 of 2,271 in Chapter 110, 1,420 of 1,420 in Chapter 112, 2,408 of 2,408 in Chapter 113, all `isChildOf` (measured 2026-09-02).
CASE has an `exactMatchOf` association type; TEA does not use it.
`CFDefinitions.CFLicenses` is an empty array in all four packages.

The Common Standards Project's `exactMatch` field is not a crosswalk.
On `CCSS.Math.Content.4.NF.C.7` it contains `["http://corestandards.org/Math/Content/4/NF/C/7", "C374A9ECF108450BB9790DDB560BBB27"]` — the standard's own canonical URI and CSP's own id for it.
It asserts identity, not equivalence to a counterpart elsewhere.

TPT does maintain a crosswalk, and its own help documentation defines the limit precisely.
From `https://help.teacherspayteachers.com/hc/en-us/articles/360042194852-Standards-Crosswalking` (fetched 2026-09-02):

> Crosswalking can only occur between CCSS and state-specific standards that are closely aligned with CCSS, allowing for an accurate translation between the two. Unfortunately, since Texas Essential Knowledge and Skills (TEKS) and Virginia Standards of Learning (VA SOL) are not structured similarly to CCSS, we're unable to provide translations from CCSS to TEKS and VA SOL. However, Sellers can still tag their resources with CCSS, TEKS, and VA SOL.

Two further facts from the same article shape our design.
The crosswalk is buyer-facing and display-time — "You'll be able to see crosswalked standards on the product page", driven by the buyer's profile location — so it is not a seller-side tagging aid and it never changes what the seller stored.
And "CCSS are not translated into international standards. If you're a user outside the US, you'll only see the original standards tags that have been added by the Seller."

For a listing crossing to Tes, that settles it.
The repository's own prior work records that Tes has no per-standard field at all, that its `frameworks` field contains "Common Core" as a permitted value, and that the one available bridge is TPT's Common Core jurisdiction mapping to Tes `orientation: American` plus `framework: Common Core` (`docs/notes/mapping/vocab-equivalence.md`, `docs/research/feasibility-report.md`).
So the correct behaviour for standards data on a Tes projection is a `Broader` edge for CCSS-tagged products — preserve the alignment claim, disclose the loss of every specific code — and `Absent` with a `NoCounterpart` record for NGSS, TEKS and VA SOL, since Tes has nowhere to put them.
That is exactly the shape `TermProjection::Broadened { to, dropped }` and `NoCounterpart` already express, so no new machinery is needed for the projection side.

Building our own crosswalk is a separate proposition and should not be bundled into ingestion.
It is a semantic alignment problem across frameworks that their own owners decline to align, TPT paid a third party to do a partial version, and the decision record forbids a model from authoring a taxonomy edge.

## What identifier a reseller tool must carry

The published code is necessary and not sufficient.

The repository's TPT recon (`docs/research/rethink/tpt-product-model.md`, landed 2026-09-02) establishes the wire facts.
Standards post to `data[ItemsCommonCoreStandard][common_core_standard_id][]` as "TPT's own opaque numeric node id", confirmed because `EducationStandardsByIds` in the capture queries exactly the 91 ids the create posted.
The read side returns that id aliased as `sphinxId`, which suggests a search-index artefact rather than a stable public identifier.
The published code appears as the node's `name` (for example `CCRA.L.1`), and the statement prose as `descriptionText` and `descriptionHtml`.
The vocabulary is reachable only by crawling: `EducationStandardsJurisdictionsQuery` yields 166 roots, of which the create form offers four — 3054 CCSS, 3055 NGSS, 3326 TEKS, 5785 VA SOL — and `EducationStandardsQuery($id, $depth)` expands one subtree at a time.
That capture holds 4,162 nodes of `type: standard`, 272 `domain`, 186 `cluster` and 36 `subject`.

So the canonical model must carry the published code as its identity, and the projection edge must carry TPT's numeric node id as `VocabularyPath.native_id`.
That is precisely the split `tam-taxonomy` already enforces: the canonical term is ours, the native id is the marketplace's, and `provenance::check_native_ids` refuses an edge whose native id is not the shape the target issues.
Today that check recognises only TPT's tag slugs, because `taxonomyTags` is TPT's one slug-addressed namespace; a standards axis would bind to `common_core_standard_id` and would need a numeric-shape check rather than the slug check.

The risk, stated plainly: an opaque numeric id aliased as `sphinxId` is a search-index identifier, and search indexes get rebuilt.
If TPT reindexes and the ids move, every projection edge we hold becomes wrong silently — the post will succeed and tag the wrong standard, or fail with a validation error, depending on whether the id survives as something else.
Three mitigations, in ascending cost.
Store the code, the id, and the statement text together, and re-verify the triple on a schedule by re-crawling `EducationStandardsQuery` and diffing.
Treat a code-to-id mismatch as a reconciliation item rather than repairing it automatically, which is what the existing queue is for.
And never post an id we have not verified within the current crawl window, degrading to "standards not projected" rather than to a wrong tag.
The second and third are cheap because the machinery exists; the first is the crawl, and it is the recurring cost of this feature.

## Ingestion design and cost

### Fetch

Four sources, three shapes.

TEKS comes from `GET https://teks-api.texasgateway.org/ims/case/v1p0/CFDocuments` then `GET /CFPackages/{id}` per document — 19 requests, roughly 40 MB total if all documents are taken, four requests and roughly 15 MB for the core four chapters.
CCSS, NGSS and VA SOL come from the Common Standards Project: `GET /api/v1/jurisdictions/{id}` then `GET /api/v1/standard_sets/{id}` per set — 38, 64 and roughly 60 current-cycle requests respectively.
Both are plain JSON over HTTPS with no authentication, no browser and no automation, which keeps this entirely inside the deterministic-and-cron-scheduled rule.
The corestandards.org HTML tree is the verification source for CCSS statement text, and it needs a browser-shaped User-Agent to get past Cloudflare — which argues for verifying against the four PDFs instead, once, by hand, rather than building a crawler for a frozen document.

### Normalisation

One shape for all four, which is the intersection of CASE and the Common Standards Project record:

```
framework        CCSS | NGSS | TEKS | VA_SOL
jurisdiction     US | US | TX | VA
subject          the framework's own subject label, captured verbatim
grade_band       the source's own grade coding, captured verbatim, plus a
                 derived interval for search
hierarchy_path   the ancestor chain of codes, root first
code             the published code (statementNotation / humanCodingScheme)
statement        the standard's prose, verbatim and unmodified
node_type        the source's own type label, captured verbatim
uri              the source's dereferenceable URI where one exists
source_guid      the source's own identifier (ASN id, CASE GUID)
```

Two rules make this survivable.
Capture the source's own vocabulary verbatim rather than mapping it into a house taxonomy at ingest — `tam-taxonomy` is already "pure projection over captured vocabularies with provenance and residue reporting", and the same discipline that keeps the Tes crosswalk honest keeps this one honest.
And derive the hierarchy from the source's parent links, never by parsing the code, because Virginia's four subjects use four incompatible code grammars.

The grade axis is the one place a derivation is genuinely needed, because a standard's grade is what a seller searches by and every framework encodes it differently: CCSS in the URL path and the code, NGSS as a prefix that is sometimes a grade and sometimes a band, TEKS in the section number, Virginia in the leading digit of most but not all codes.
`grades.rs` already owns interval derivation with uncovered-residue reporting, which is the right home.

### Provenance

Every row records its source, its fetch timestamp, and the source's own `lastChangeDateTime` where one exists.
The `Decider::Imported { source }` variant already exists for exactly this and forbids a model from authoring the edge.
The blue-agency fact about Virginia belongs in the provenance string, not in a comment: `csp:virginia:D2985351` is a different trust level from `tea-case:bc997e24`, and a reconciliation item raised against a Virginia row should say so.

### Update strategy

Poll each source on a cron, diff against the stored snapshot by source GUID, and emit changes as reconciliation items rather than applying them.
The cadences differ by an order of magnitude: CCSS and NGSS are frozen, so an annual poll is a liveness check rather than an update mechanism; TEKS moves on the SBOE's schedule, with social studies changing in 2026, so quarterly; Virginia moves on a seven-year rolling cycle across subjects, so quarterly as well.
The event that actually matters is a code disappearing, because some seller's listing points at it.
Retire rather than delete, keep the row with an end date, and let the projection surface it.

### Expected table sizes

| Framework | Addressable codes | All nodes | Statement text |
|---|---|---|---|
| CCSS | ~1,540 | ~3,600 | ~0.9 MB |
| NGSS | 208 PEs | ~3,350 | ~0.8 MB |
| TEKS, four core chapters | ~5,000 SEs | ~7,600 | ~4 MB |
| VA SOL, three core subjects measured | ~4,200 | ~4,500 | ~2.5 MB |

Order 11,000 addressable codes and 19,000 nodes across the four, under 10 MB of text.
This is a small table by any measure; the cost is not storage, it is correctness and currency.

### Licence obligations that must appear in the UI

- The CCSS copyright notice, verbatim, wherever a Common Core standard is published or publicly displayed.
- The NGSS asterisk-and-disclaimer footnote, at the bottom of the home page and of every page that prominently uses the NGSS mark, with the mark visually subordinate to ours, no ® or ™, and no logo.
- Attribution for whichever mirror we ingest from, under its CC BY terms — D2L Corporation or Common Curriculum, Inc. depending on the set.
- No paraphrase of any standard's statement text anywhere, including in generated listing copy, since the CCSS grant does not extend to modification.

### Effort

Assumptions: one engineer; the fetch is a Rust binary in the existing seeder shape rather than a service; no new database technology; verification is a diff against a hand-checked sample per framework; the crosswalk is explicitly out of scope; the TPT node-id crawl is counted separately because it belongs to the marketplace adapter, not to the taxonomy.

| Work | Days |
|---|---|
| CASE fetch and parse, covering TEKS end to end | 2 |
| Common Standards Project fetch and parse, covering CCSS, NGSS and VA SOL | 2 |
| Normalisation into the common shape, with the grade derivation and residue reporting | 2-3 |
| Storage, provenance, retirement semantics, and the poll-and-diff job | 2-3 |
| Verification: hand-check a sample per framework against the owner's PDF, and diff Virginia's two mirrors | 1-2 |
| Licence rendering in the UI | 0.5 |
| **Total** | **9.5-14.5** |

Add 3-5 days for the TPT standards-vocabulary crawl and the code-to-node-id mapping table, which is what actually makes the feature post.

Hardest framework: Common Core, and not for the reason one would guess.
It is the smallest and the most stable, but it is the only one whose owner has stopped publishing machine-readable data while keeping a licence that constrains what we may do with the text, behind bot protection, on a site that links its own documentation to the Wayback Machine.
Every other framework has either a live owner feed (TEKS) or a live owner with a small enough corpus to verify by hand (NGSS, 208 rows).
Common Core has 1,540 codes we must take from a mirror and cannot paraphrase, under a licence whose owner is not visibly maintaining the canonical source.

Runner-up: Virginia, for a different reason — four subjects, four incompatible code grammars, no owner data, a blocked website, and both available mirrors being third-party transcriptions of PDFs.

## Display and search

Eleven thousand codes rules out a flat dropdown categorically, and the per-framework numbers say where the line falls.
NGSS at 208 performance expectations is a browsable tree and needs nothing else.
CCSS at ~1,540 leaves needs search but has a regular code grammar, so code search alone would carry most of the load.
TEKS at ~5,000 student expectations and VA SOL at ~4,200 need everything.

What a teacher actually does, in the order they do it:

- Types a code fragment, because they have it in front of them from their lesson plan. `4.NF.7`, `111.5.b.3`, `MS-LS1-1`, `3.PFA.1`. This must match on the bare form as well as the prefixed form — `RL.2.1` must find `CCSS.ELA-Literacy.RL.2.1`, which is what CSP's `altStatementNotation` field is for, and TEKS users type `5.3B` more often than `111.7.b.3.B`.
- Filters by grade and subject first, then browses, because that is how the standards documents are organised and how the teacher's own year is organised.
- Searches the statement text by keyword when they know the concept but not the code — "equivalent fractions", "author's purpose".
- Expands a tree, because the parent's text carries meaning the child's does not. This is explicit in the Common Core identifier documentation: math cluster headings were given identifiers precisely so that "applications using the system can preserve the meanings that arise from considering the cluster headings and the individual content standards in conjunction with one another." Rendering a bare standard without its cluster loses information the owners deliberately preserved.

Two things follow for the picker.
Show the code and the full statement together, never the code alone, and never a truncated statement, because the CCSS licence does not permit us to shorten it and a truncated NGSS performance expectation is unusable anyway.
And keep the framework selection explicit and first, because a teacher in Texas has no use for the other three and a teacher tagging for a national audience wants CCSS and nothing else.

TPT's own form is the shape to match, and the repository's recon records it: four collapsed sections, each with a "Select …" link opening a picker, each picker drilling one level at a time.

## Open questions for the founder

1. TEKS terms of service versus the Texas Administrative Code. TEA's documentation site restricts registered users to "personal, noncommercial use" and forbids bulk download, while the TEKS themselves are state law. Recommendation: get a written answer from TEA before shipping, and in the meantime ingest TEKS from the Common Standards Project mirror, which carries a CC BY licence and no such restriction.
2. NGSS commercial trademark compliance. Shipping an NGSS picker requires the WestEd disclaimer on the home page and every page prominently using the mark, and sample submission with a 4-6 week turnaround. Recommendation: submit samples now, because six weeks is longer than the rest of this work.
3. Whether to display standard statement text at all, or only codes. Displaying text triggers the CCSS attribution obligation and forecloses paraphrase everywhere including generated listing copy. Recommendation: display it, accept the constraint, and add the notice — a code-only picker is close to unusable.
4. Whether standards alignment is a new `TermKind` on the existing axis machinery or a separate first-class field. The existing axes are Subject, Topic, ResourceType, Phase, Licence, all of which project through the same edge relation. Standards differ in that the canonical unit is the marketplace's own node id rather than a shared concept. Recommendation: a separate field with its own table, reusing the provenance and reconciliation machinery but not the projection-edge relation.
5. Whether to pay for Academic Benchmarks. Its free tier covers one state plus CCSS plus NGSS, which leaves either TEKS or VA SOL uncovered; the paid tier is unpriced publicly. Recommendation: no, for now — the free sources cover all four and the AB GUID buys us nothing that TPT's node id does not, since TPT is the only place we post.
6. Whether we build a CCSS-to-TEKS or CCSS-to-VA-SOL crosswalk. TPT declined to, citing structural dissimilarity, and no owner publishes one. Recommendation: no, and record it as a deliberate non-goal rather than a backlog item.
7. Whether standards data on a Tes projection is a `Broader` edge (CCSS to `framework: Common Core`) or is simply dropped. Recommendation: `Broader` for CCSS, `NoCounterpart` for the other three, so the seller sees the loss in the field diff before publish.

## Unverified

- The Satchel Rosetta Exchange commercial figures (the "no more than $8,400 per year" subscriber contribution and the "$500-1500 per PK-12+ subject area" conversion pricing) come from search-result summaries of Common Good Learning Tools material, not from a price sheet we read directly.
- Academic Benchmarks pricing is not published anywhere we could reach; the free-tier description (one state plus CCSS plus NGSS) comes from search results rather than from Instructure's own pricing page.
- Virginia SOL publication formats and revision dates are reported from secondary sources and from source URLs embedded in mirror data, because `doe.virginia.gov` returned HTTP 403 to every client we tried on 2026-09-02.
- The node count for Virginia Science (2018-) was not aggregated; the ~4,200 figure covers Mathematics, English and History and Social Science only.
- The 15 TEKS CFDocuments other than Chapters 110, 111, 112 and 113 were not downloaded, so the ~40 MB whole-corpus figure is extrapolated from the four measured packages.
- Whether the TEKS CASE GUIDs are stable across republication is inferred from their UUIDv5 shape, not confirmed by observing a republication.
- The canonical Texas Administrative Code URL is unresolved: `texreg.sos.state.tx.us` serves a redirect notice and we did not follow it to the new host.
- 1EdTech's "CASE Global Ecosystem" is announced but unlaunched as of 2026-09-02, so its effect on the CASE Network succession is unknown.
- Whether the Common Standards Project imposes a rate limit is unknown; we completed 102 sequential requests at roughly 5 per second without being throttled, which is evidence of absence at that rate only.
- Whether TPT's `sphinxId` values have ever changed is unknown; the risk is inferred from the alias name and from the general behaviour of search-index identifiers.
