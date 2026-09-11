# Research: cross-marketplace duplicate detection, and "Collections"

Scope: two questions for Teachouse (`listing-sync`). Repo facts cited `file:line`; web facts cited
by URL. Read-only pass; no repo edits.

---

## Question 1 — identifying the same teaching resource across marketplaces

### 1.1 What the repo already gives us

- **Bytes never reach the server.** D27 puts file bytes on the seller's device; a migrated file is
  fetched under the seller's own session and uploaded to the target in the same run
  (`crates/tam-storage/migrations/0052_product_file_source.sql:3-8`). So `product_file` has two
  digest columns with two different meanings: `hash` = "a digest the server verified over bytes it
  holds"; `observed_hash` = "a digest a device asserted over bytes we never held"
  (`0052_product_file_source.sql:10-15`, `:49-50`). **Every content fingerprint below must be
  computed device-side and uploaded as an assertion, in exactly that shape.**
- **The unwrap rule already makes cross-marketplace byte equality possible.** `observed_hash`
  digests *the bytes handed onward after the unwrap decision* — the sole entry where a Tes bundle
  reduces to one file, the bundle otherwise, never the container, because "a marketplace that
  re-zips a bundle with different timestamps changes the container's digest while the entry's is
  unchanged" (`0052_product_file_source.sql:88-95`). For single-file resources a Tes ZIP-of-one and
  a TPT product file therefore already produce **the same digest**. For multi-file Tes bundles the
  container is digested, so byte-level matching needs a set rule (below), not a single equality.
- **Disagreement is already modelled as evidence, not as an error.** `product_file_observation`
  keeps every observation; "a second device reporting a different digest for the same resource is a
  disagreement to surface, not a write to refuse" (`0052_product_file_source.sql:149-158`). A
  duplicate verdict table should follow the same voice.
- **A candidate-set-with-a-human-decision pattern already exists.** `binding_candidate` holds "the
  candidate identifiers an ambiguous create could not decide between"
  (`crates/tam-storage/migrations/0004_mapping.sql:156-174`), and `field_mismatch` records
  per-field, classified evidence (`0004_mapping.sql:176-201`). Duplicate review should be a third
  instance of that pattern, not a new one.
- **Import is already two-phase with per-row breadcrumbs.** "Nothing here creates anything. A parsed
  batch holds rows and refusals; the products, the labels and the jobs are a later, deliberate
  commit" (`crates/tam-storage/migrations/0058_import_batch.sql:9-16`), and each row carries a
  `product_id` breadcrumb so a resumed commit skips what it already created (`:5-13`). Duplicate
  review belongs in the gap between parse and commit — the machinery is there.

### 1.2 Evidence from the literature

- **Shingling + MinHash** (Broder 1997, *On the resemblance and containment of documents*): define
  resemblance as the Jaccard index of a document's w-shingles and estimate it from a fixed-size
  min-wise sample, so two documents' similarity is computed from small sketches rather than from
  their text. https://dblp.org/rec/conf/sequences/Broder97.html
- **SimHash** (Charikar, STOC 2002, *Similarity estimation techniques from rounding algorithms*):
  random-hyperplane LSH whose fingerprints preserve cosine similarity, so near-duplicates differ in
  few bit positions. https://dblp.org/rec/conf/stoc/Charikar02.html
- **SimHash at scale** (Manku, Jain, Das Sarma, WWW 2007): validated that **64-bit fingerprints with
  k = 3 bit differences** are the right operating point over 8 billion pages, and gave the banded
  table technique for finding all fingerprints within Hamming distance k.
  https://research.google.com/pubs/archive/33026.pdf
- **Fusion beats either signal** (Henzinger, SIGIR 2006, *Finding near-duplicate web pages: a
  large-scale evaluation of algorithms*, 1.6 B pages): Charikar's algorithm reached precision 0.50,
  Broder's 0.38; **a combined algorithm reached 0.79** at 79 % of the recall. Also: neither works
  well for near-duplicates *from the same site* — the direct analogue of "two listings by the same
  seller on the same marketplace", which is why the merge verb should be cross-marketplace only.
  https://infoscience.epfl.ch/entities/publication/ca590109-0048-454b-89d9-4cbb9b64b3bc
- **Image hashes.** pHash: grayscale → 32×32 → 2-D DCT → binarise the top-left 8×8 low-frequency
  block, robust to mild JPEG re-compression, resizing and small overlays, sensitive to flips and
  crops (https://phash.org/docs/design.html). dHash: resize to 8×9 and compare adjacent pixels;
  evaluations find it performs almost as well as pHash
  (https://arxiv.org/pdf/2109.00899). Blockhash implements Yang/Gu/Niu's block-mean-value hash
  (IIH-MSP 2006), block-based and therefore more robust to *local* changes such as a watermark or a
  "FREE PREVIEW" band (https://dl.acm.org/doi/abs/10.5555/1193214.1194040,
  https://github.com/commonsmachinery/blockhash-python).
- **Entity resolution practice**: a two-stage blocker-then-matcher pipeline is the standard shape —
  a cheap blocker selects candidate pairs, an expensive matcher decides them (SC-Block,
  https://arxiv.org/pdf/2303.03132). The reference e-commerce benchmark is WDC Products, built by
  clustering Common Crawl offers on schema.org identifiers (https://arxiv.org/html/2301.09521) —
  note the luxury we do **not** have: no GTIN/MPN exists for a teaching resource, which is exactly
  why content fingerprints must carry the weight.
- **Scoring**: Fellegi–Sunter log-odds accumulation over per-field agreement is the standard matcher
  (https://www.norc.org/content/dam/norc-org/pdfs/G-Link_Probabilistic%20Record%20Linkage%20paper_PVERConf_May2011.pdf).
  Its known weakness matters here: plain FS "does not leverage information contained in field values
  and consequently leads to identical classification regardless of whether records agree on rare or
  common values" (https://pmc.ncbi.nlm.nih.gov/articles/PMC9336505/). **Frequency-weighting is not
  optional for us** — a seller's `Terms of Use.pdf` is byte-identical across their entire catalogue,
  so an unweighted exact-digest match would merge their whole store.
- **Marketplace precedent for the interaction, not the algorithm**: eBay flags duplicates partly on
  title/description similarity and penalises by removal or search suppression
  (https://www.ebay.com/help/policies/listing-policies/duplicate-listings-policy?id=4255). Amazon
  surfaces a *Potential Duplicates* page and gives the seller 30 days to respond before suppression,
  plus a self-service "merge duplicate detail pages" tool
  (https://clickfluency.com/what-you-need-to-know-about-amazon-duplicate-listings/,
  https://www.amalytix.com/en/knowledge/strategy/amazon-listing-duplication/). Both large
  marketplaces **surface a queue and let the seller decide** rather than merging silently.

### 1.3 The layered matcher

Run **within one org only** (cross-org matching would leak one seller's catalogue into another's) and
propose merges **only across different marketplaces**.

| # | Layer | Signal | Strength |
|---|---|---|---|
| L1 | Exact payload digest | any payload entry digest + byte length equal, **and that digest occurs in ≤ 2 products org-wide**, **and** the file is ≥ 50 KB | decisive |
| L1b | Payload set overlap | Jaccard over the set of per-entry digests ≥ 0.6 (handles Tes multi-file ZIP vs TPT single file) | strong |
| L2 | PDF text fingerprint | extracted text → normalised → 5-word shingles → MinHash sketch; estimated Jaccard ≥ 0.9 strong, 0.6–0.9 moderate. 64-bit SimHash Hamming ≤ 3 used as the *blocker* | strong (survives re-export, which is the whole point: re-export changes bytes, not words) |
| L2b | Rendered-page hashes | per-page dHash of pages rendered at ~72 dpi, for scanned/image-only PDFs where L2 extracts nothing; ≥ 80 % of pages within Hamming 6 | strong, expensive |
| L3 | Cover/preview pHash | Hamming ≤ 6 / 64 | moderate, **never sufficient alone** — covers are frequently marketplace-generated or reused across a seller's whole range |
| L4 | Title similarity | normalise (lowercase, strip punctuation, strip marketplace boilerplate — "| TPT", "distance learning", "printable & digital", the seller's own store name), then token Jaccard ≥ 0.6 or trigram similarity ≥ 0.5 | weak |
| L5 | Metadata corroboration | grade-band overlap; same subject; **page count equal**; price ratio within ×1.5 after currency conversion; creation dates within 90 days | weak positives; page count differing > 20 % is a **strong negative** |

**Fusion.** Sum per-signal log-odds weights (Fellegi–Sunter), frequency-weighted at L1/L3/L4 so that
agreement on a value that is common inside this seller's catalogue earns little. Three outcomes:

- **auto-merge** only on `L1` (rare, large, exact) **or** `L2 ≥ 0.9 ∧ page count equal`, and only
  when no strong negative fires;
- **ask the seller** for everything above a review floor (roughly: any two independent moderate
  signals, e.g. L1b + L4, or L2 in 0.6–0.9 + L5);
- **distinct** by default — silence is not a merge.

The asymmetry is deliberate. A missed duplicate costs the seller one review click later; a false
merge collapses two real products, and the next publish overwrites a live listing with the wrong
resource. Henzinger's 0.79 precision was the *best fused* number on web pages; at that precision an
unattended merge would be wrong one time in five.

### 1.4 What to store

A `product_fingerprint` row per resource (device-asserted, versioned), plus per-file digests already
in `product_file`:

```
product_fingerprint(org_id, product_id,
  fingerprint_version smallint,      -- freeze the sketch format; a bump re-computes, never compares across
  text_simhash        bigint,        -- 64-bit Charikar
  text_minhash        bytea,         -- k=128 × u32 = 512 bytes, fixed width
  shingle_count       int,
  extracted_chars     int,           -- 0 ⇒ image-only PDF ⇒ L2 unavailable, fall to L2b
  page_count          int,
  page_dhash          bytea,         -- u64 per page, first 8 + last page
  cover_phash         bigint,
  title_norm          text,          -- pg_trgm GIN index
  observed_by_device  text, observed_at timestamptz, recorded_at timestamptz)
```

Naming follows `0052_product_file_source.sql:168-175`: every device-asserted column keeps a name
that says so, because "one word meaning both [a server verification and a device assertion] across
two tables is the conflation this whole design exists to prevent".

Blocking indexes: four 16-bit band columns over `text_simhash` (Manku's banded technique), a MinHash
LSH band table, `pg_trgm` on `title_norm`, and the existing digest index on `product_file`.

Verdicts get their own table keyed on the **unordered** product pair, storing `same | different` plus
the evidence snapshot — a "different" verdict must be recorded too, or the next import re-asks a
question the seller already answered.

### 1.5 Device vs server

| Device (Tauri/Android, where the bytes are) | Server |
|---|---|
| blake3 of payload entries (already: `observed_hash`) | blocking (SimHash bands, LSH, trigram) |
| PDF text extraction → shingles → MinHash + SimHash | pair scoring and fusion |
| page count, page dHashes, cover pHash | persisting verdicts, driving the review UI |
| upload fixed-width sketches (≈ 600 B/resource) | never sees text, never sees bytes |

A 512-byte MinHash sketch is not the seller's content, so shipping it upward does not violate D27 —
and that is precisely why a *sketch* is the right transport rather than extracted text.

### 1.6 Rust crates (status checked 2026)

| Need | Crate | Status | Verdict |
|---|---|---|---|
| image pHash/dHash/blockhash | **`image_hasher`** (fork of `img_hash`, adds Blockhash) https://crates.io/crates/image_hasher | updated Feb 2026; maintainer states he is "not familiar too much with this library" and is looking for co-maintainers (https://lib.rs/crates/image_hasher) | **use**, but pin and vendor-audit; the algorithms are small enough to re-implement if it dies |
| — | `img_hash` 3.2.0 | last release ~2021 (https://crates.io/crates/img_hash) | **avoid**, superseded |
| PDF text | **`pdf-extract`** https://crates.io/crates/pdf-extract | ~880 k downloads/month, 432 dependents, updated Jun 2026 | **use** for L2 |
| PDF structure / page count | **`lopdf`** https://crates.io/crates/lopdf | 0.41 (Jun 2026), 0.40, 0.39 — active; needs Rust ≥ 1.85 | **use** |
| page rendering (L2b) | `pdfium-render` https://crates.io/crates/pdfium-render | 0.9.2 (Jun 2026), active; run-time binding to Pdfium (so WASM-capable) | **phase 2** — it means shipping the Pdfium binary in the desktop bundle; do not pay that on day one |
| SimHash | `simhash` 0.3 https://lib.rs/crates/simhash | ~1.5 M all-time downloads, last update ~12 months, single-author | **implement inline** (~30 lines) — the bit layout must be frozen in the schema anyway |
| MinHash | `probminhash` (Ertl's ProbMinHash2/3, SuperMinHash, updated 2025) https://crates.io/crates/probminhash; `gaoya` (full LSH index + 8/16/32/64-bit minhash and 64/128-bit simhash) https://lib.rs/crates/gaoya | probminhash active; gaoya last release > 2 years ago | **implement classic k-permutation MinHash inline** over blake3; reach for `probminhash` only if weighted Jaccard is later wanted. Do not adopt `gaoya` — an unmaintained crate that owns your *stored* sketch format is a migration you cannot run |
| content digest | `blake3` | already in use | keep |

### 1.7 UI pattern for "possible duplicate — keep one"

Place it between parse and commit, where import already pauses
(`0058_import_batch.sql:9-16`). For each flagged pair, one card:

1. **Two listings side by side**, each with its marketplace badge, cover, title, price, grades.
2. **One plain-English evidence line**, generated from the winning layer — "the same file, byte for
   byte (worksheet-pack.pdf, 2.4 MB)" / "the text of both PDFs is 94 % the same" / "same page count,
   same grade band, near-identical title". Never a score. A percentage the seller cannot act on is
   worse than a sentence they can.
3. **Three buttons**: *Same resource — keep one* · *Different resources* · *Decide later*.
4. On "same", a **which-side-wins** step per field (title, description, cover, price) with the two
   candidate values shown; per-marketplace values are *kept*, not discarded — the merged resource
   holds one canonical record plus the per-marketplace overrides that already live on `mapping`.
5. The merge stays **reversible for 30 days** (keep the losing side's mapping rows, tombstoned).
   Amazon's 30-day duplicate window is the precedent for the duration
   (https://clickfluency.com/what-you-need-to-know-about-amazon-duplicate-listings/).
6. "Decide later" parks the pair in a *Possible duplicates* list — Amazon's *Potential Duplicates*
   page, scaled down — so nothing blocks the import.

### Recommendations — duplicate detection

1. **Make the exact-digest layer frequency-weighted before shipping it.** Unweighted, it merges every
   product carrying the seller's shared `Terms of Use.pdf`. Rule: digest must occur in ≤ 2 products
   org-wide and cover ≥ 50 KB to count.
2. **Compute every fingerprint on the device and upload fixed-width sketches**, in the same
   assertion shape `observed_hash`/`asserted_scan_state` already use
   (`0052_product_file_source.sql:10-15`, `:51-58`). The server does blocking and scoring only.
3. **Ship L1 + L1b + L2 + L4 + L5 first; defer L2b (page rendering).** Text extraction with
   `pdf-extract` covers the re-export case, which is the actual founder scenario; Pdfium in the
   bundle is a packaging cost that buys only scanned PDFs.
4. **Auto-merge only on decisive evidence** (rare exact digest, or text Jaccard ≥ 0.9 with equal page
   count, and no strong negative). Everything else asks. Henzinger's fused precision of 0.79 is the
   ceiling for "clever scoring", and 0.79 is not good enough to act unattended on destructive merges.
5. **Restrict merge proposals to cross-marketplace pairs**; Henzinger found same-site pairs are
   exactly where these algorithms fail, and two listings on one marketplace are a policy question
   (eBay, Amazon), not an identity one.
6. **Store a verdict per unordered pair, including "different".** Otherwise re-import re-asks.
7. **Model the candidate set on `binding_candidate` + `field_mismatch`** (`0004_mapping.sql:156-201`)
   rather than inventing a shape: positioned candidates plus classified per-field evidence is already
   this codebase's idiom for "we could not decide, here is why".
8. **Freeze the sketch format behind `fingerprint_version`** and never compare across versions;
   re-compute on the device instead.
9. **Crates: `image_hasher`, `pdf-extract`, `lopdf`, `blake3`; hand-roll SimHash and MinHash.** Any
   crate whose output is persisted in the database must be one you can survive losing — `gaoya`
   (dormant > 2 years) fails that test, and `simhash`/`minhash` are too small to justify the risk.
10. **Review UI: sentence, not score; three buttons; reversible for 30 days; parked pairs never
    block the import.**

---

## Question 2 — Collections

### 2.1 Repo starting point

- `label` is org-scoped, name-unique case-insensitively, with a **closed colour set** the console
  maps to theme tokens (`0046_product_label.sql:20-40`); `product_label` is a plain many-to-many with
  `ON DELETE CASCADE` from the label side, justified as "a filter offering a label no item can be
  found by" being the worse alternative (`:42-55`).
- The "20-label cap" is **per product, not per vocabulary**:
  `LABELS_PER_PRODUCT_MAX: usize = 20` (`crates/tam-api/src/resources.rs:1447`), with
  `LABEL_MAX_CHARS = 60` (`:1452`) mirroring the SQL bound (`0046_product_label.sql:29`).
- The uniqueness rule already has a database enforcer:
  `mapping_one_per_inventory UNIQUE (org_id, product_id, inventory)`
  (`crates/tam-storage/migrations/0004_mapping.sql:63`), plus one bound URL and one bound numeric id
  per inventory (`0018_mapping_bind.sql:20-24`). Per-marketplace behaviour (price rule, publish mode,
  lifecycle) lives on `mapping` (`0004_mapping.sql:120-147`), **not on the product**.
- Idempotency is an established idiom: `import_batch.id` is "the submit's own idempotency key, so a
  double-clicked upload is one batch rather than two, following `job` and `sync_request`"
  (`0058_import_batch.sql:19-22`).

### 2.2 How comparable products model this

| Product | Model | Cardinality | Lesson |
|---|---|---|---|
| **Etsy shop sections** (https://help.etsy.com/hc/en-us/articles/360000345048) | storefront navigation | **one section per listing**, ≤ 20 sections, ≤ 24-char names; empty sections hidden; **deleting a section does not deactivate its listings** | one-to-many is a *storefront* constraint, not an identity one; the delete semantics are the right default |
| **Shopify collections** (https://help.shopify.com/en/manual/products/collections, https://shopify.dev/docs/api/admin-graphql/latest/objects/Collection) | many-to-many, manual **and** rule-based, now unified with multiple sources, inclusion **and exclusion** conditions, and nesting; collections are **unpublished by default** and made visible per sales channel via `publishablePublish` | many-to-many | the closest analogue, and the key structural idea: **publication is a separate, per-channel act on the collection**, not a property of membership. Also a warning: their current model is the product of years of iteration |
| **TPT bundles** (https://help.teacherspayteachers.com/hc/en-us/articles/360042429532) | a bundle **is a product**: 2–500 members, a resource may sit in ≤ 15 bundles, auto-updates when a member changes | many-to-many | bundles are **not** collections — they have their own listing, price and buyer entitlement |
| **Tes bundles** (https://www.tes.com/author-academy/getting-started/how-create-resource-bundles) | 2–20 resources, ≥ 2 paid, own title/description/cover/price; **age and subject tags are derived from members**; buyers keep the snapshot they bought | many-to-many | same: a saleable composite with snapshot semantics |
| **Vendoo** (https://blog.vendoo.co/7-vendoo-features-every-reseller-should-be-using) | "Custom Labels … electronic colour-coded stickers" for inventory | many-to-many | the crosslisting incumbent ships **labels**, not collections |
| **List Perfectly** (https://listperfectly.com/faq/, https://listperfectly.com/selling/list-perfectly-pro-plus-auto-delist/) | bulk verbs: Import & Crosslist, Delist/Relist, Update, Mark Sold | — | the shipped verb set is small and action-shaped |
| **Notion relations** (https://www.notion.com/help/relations-and-rollups) | relation property, many-to-many by default, with an explicit "limit to **1 page**" option; rollups aggregate over the relation | both, by choice | many-to-many is the default even in a general-purpose tool; one-to-one is the special case you opt into |

### 2.3 Is many-to-many a problem for the uniqueness rule?

**No.** Membership is a reference, not a copy. The uniqueness rule — one canonical resource per real
product — is a statement about *identity* and about *the mapping to each marketplace*, and the
database already enforces the latter at `0004_mapping.sql:63`: one mapping per `(org, product,
inventory)`. A membership row `(collection_id, product_id)` creates no `product` row and no `mapping`
row, so there is no cardinality it could violate. Shopify's entire catalogue model is one product in
many collections; Etsy's one-section rule is a storefront navigation choice, not an identity one.

The rule is only threatened by two adjacent features, both of which must be refused:

- **"Duplicate into collection"** — would mint a second `product`. Never.
- **Per-collection overrides** (a price, a description, a template stored *on the membership*) —
  would give a resource in two collections two conflicting answers for the same marketplace. Those
  overrides already have a home on `mapping` and in resource templates
  (`0056_resource_template.sql`); a second home is the bug.

### 2.4 Pitfalls of bulk actions on overlapping collections, and the fix

1. **Double-publish.** A resource in two selected collections yields two publish commands.
   *Fix, three layers deep:* (a) the verb resolves the **distinct union** of product ids before
   anything is enqueued and the preview shows that union; (b) enqueue is idempotent on
   `(org, product, marketplace, intent)` using the key idiom already in the codebase
   (`0058_import_batch.sql:19-22`); (c) `mapping_one_per_inventory` means even a leaked second
   create cannot produce a second listing. Stripe's semantics are the model to copy — store the
   status and body of the first request per key, replay it on retry, and **error when the same key
   arrives with different parameters** (https://docs.stripe.com/api/idempotent_requests).
2. **Conflicting per-marketplace overrides.** Prevented structurally: collections carry no
   per-marketplace state at all. A collection may *offer* a template as an argument to a verb ("use
   template X for this run"), shown in the preview and confirmed by the seller — an argument, never
   stored state.
3. **Partial failure and re-run.** The preview must be per-resource ("will create / already live /
   blocked: missing grade"), and the re-run must skip what succeeded — exactly the hazard
   `import_batch` was built to solve, with the per-row `product_id` breadcrumb
   (`0058_import_batch.sql:5-13`). Reuse it.
4. **Deleting a collection.** Must cascade to membership and touch nothing else — Etsy's
   "deleting a section doesn't deactivate the listings" is the right behaviour and the seller's
   expectation.
5. **Rule drift** (if smart collections ever exist). A rule edited between preview and execution
   changes what "publish this collection" means. Any action must **freeze the resolved member ids
   into the operation**.

### 2.5 Smart vs manual; collections vs labels

**Manual only, for now.** Shopify's rule engine now spans multiple sources, inclusion *and*
exclusion conditions, and nesting — that is a mature product's surface, not a v1's. Teachouse already
has the cheap 80 % of "smart": labels plus the existing filter. A saved filter view answers "label =
Autumn ∧ grade = 3" without inventing membership that changes underfoot. Keep the door open by
recording resolved ids in every operation from day one; a `collection.rule` column and a resolver can
be added later without touching the action path.

**Beside labels, not instead of them.** They are different parts of speech:

- a **label** is an adjective — many per resource (cap 20, `resources.rs:1447`), unordered, coloured,
  a filter dimension, and cheap to apply in bulk;
- a **collection** is a noun — a named, described, *ordered* list that you can *act on*.

Vendoo ships only labels; Shopify ships only collections; neither is a substitute for the other.
Subsuming labels into collections would force a description and an ordering onto every "Autumn term"
tag, discard the closed colour set the console maps to theme tokens
(`0046_product_label.sql:13-19`), and break the cascade semantics already reasoned through at
`0046_product_label.sql:51-54`.

**And keep "collection" away from "bundle".** TPT and Tes bundles are *products*: own title, cover,
price, member caps (≤ 500 / ≤ 20), derived tags, and buyer-snapshot entitlement. If the founder later
wants bundles, that is a product-composed-of-products change, not a rename of collections.

### 2.6 Minimal model

```sql
CREATE TABLE collection (
    org_id      uuid        NOT NULL REFERENCES organisation (id),
    id          uuid        NOT NULL,
    name        text        NOT NULL,          -- 1..80
    description text,                          -- nullable, ..1000; a list without prose is normal
    created_at  timestamptz NOT NULL,
    PRIMARY KEY (org_id, id),
    CONSTRAINT collection_name_bounded CHECK (char_length(name) BETWEEN 1 AND 80),
    CONSTRAINT collection_description_bounded CHECK (description IS NULL
        OR char_length(description) BETWEEN 1 AND 1000)
);
-- Same rule, same reason, as label_one_per_name (0046:35-40).
CREATE UNIQUE INDEX collection_one_per_name ON collection (org_id, lower(name));

CREATE TABLE collection_member (
    org_id        uuid        NOT NULL REFERENCES organisation (id),
    collection_id uuid        NOT NULL,
    product_id    uuid        NOT NULL,
    position      int         NOT NULL,        -- collections are ordered; labels are not
    added_at      timestamptz NOT NULL,
    PRIMARY KEY (org_id, collection_id, product_id),
    FOREIGN KEY (org_id, collection_id) REFERENCES collection (org_id, id) ON DELETE CASCADE,
    FOREIGN KEY (org_id, product_id)    REFERENCES product (org_id, id)
);
CREATE INDEX collection_member_by_product ON collection_member (org_id, product_id);
-- RLS exactly as label/product_label (0046:62-74).
```

No third table. A bulk verb expands into the existing per-resource jobs with the existing idempotency
keys; the collection is recorded as the request's `source` (`0028_sync_request_source.sql`).

**Invariants**

- **I1** Membership never creates, copies or clones a `product`; the only write is `collection_member`.
- **I2** No per-marketplace column ever appears on `collection` or `collection_member`. Overrides live
  on `mapping` (`0004_mapping.sql:120-147`) and in resource templates.
- **I3** Deleting a collection cascades to membership and to nothing else.
- **I4** A bulk verb acts on the **distinct union** of resolved product ids; enqueue is idempotent on
  `(org, product, marketplace, intent)`.
- **I5** A resource may belong to unlimited collections (unlike labels, which are capped per product);
  membership *within* a collection may be capped generously (e.g. 500) only if pagination demands it.
- **I6** Every action records the resolved member ids, so a later membership change cannot
  retroactively alter what an operation meant.

**The three bulk verbs worth shipping first**

1. **Publish to a marketplace** — the founder's actual ask. Preview → per-resource rows (*will create*
   / *already live* / *blocked: reason*) → confirm → enqueue. Shopify's precedent that publication is
   a distinct, per-channel act applies directly.
2. **Apply template / add-remove labels across the union** — cheap, reversible, no marketplace call,
   and the verb sellers reach for most. It is also the safest place to prove the union-and-preview
   machinery before pointing it at a live marketplace.
3. **Export the collection** to the spreadsheet the import path already speaks
   (`crates/tam-api/src/export.rs`, `0058_import_batch.sql`) — the collection is the natural
   selection for the round trip, and it costs almost nothing.

Deliberately *fourth*: **delist/withdraw**. List Perfectly's verb set says it is the next most
valuable, but it is destructive against live listings; it waits until publish's preview is trusted.

### Recommendations — collections

1. **Ship collections as a plain many-to-many beside labels** — two tables, no new operation table,
   RLS copied from `0046_product_label.sql:62-74`.
2. **State in the schema comment that membership is a reference, never a copy**, and that the
   uniqueness rule is enforced by `mapping_one_per_inventory` (`0004_mapping.sql:63`) — so a future
   reader does not "fix" many-to-many by cloning products.
3. **Forbid per-marketplace state on collections outright.** Overrides belong on `mapping` and on
   resource templates; a second home is how the double-override bug is born.
4. **Every bulk verb: resolve → de-duplicate the union → preview per resource → confirm → enqueue
   idempotently** on `(org, product, marketplace, intent)`, replaying the stored result on retry and
   erroring on a key reused with different parameters (Stripe's semantics).
5. **Record resolved member ids in the operation**, so membership changes cannot retroactively change
   what a seller confirmed.
6. **Manual collections only.** No rules, no nesting, no exclusions. The saved label filter is the
   smart collection, and it already exists.
7. **Do not merge labels into collections.** Adjective vs noun; Vendoo and Shopify each ship exactly
   one of the two, and the colour/cascade reasoning at `0046_product_label.sql:13-19` and `:51-54`
   would be thrown away.
8. **Do not build bundles under the name "collections".** TPT (≤ 500 members, ≤ 15 bundles per
   resource) and Tes (≤ 20, ≥ 2 paid, derived tags, buyer snapshot) show a bundle is a *product*;
   that is a separate change.
9. **Delete semantics: Etsy's.** Deleting a collection removes membership and deactivates nothing.
10. **Ship three verbs — publish, apply-template/label, export — and keep delist for after the
    preview path has earned trust.**

---

## Sources

1. Broder, *On the Resemblance and Containment of Documents*, SEQUENCES 1997 — https://dblp.org/rec/conf/sequences/Broder97.html
2. Charikar, *Similarity Estimation Techniques from Rounding Algorithms*, STOC 2002 — https://dblp.org/rec/conf/stoc/Charikar02.html
3. Manku, Jain, Das Sarma, *Detecting Near-Duplicates for Web Crawling*, WWW 2007 — https://research.google.com/pubs/archive/33026.pdf
4. Henzinger, *Finding near-duplicate web pages: a large-scale evaluation of algorithms*, SIGIR 2006 — https://infoscience.epfl.ch/entities/publication/ca590109-0048-454b-89d9-4cbb9b64b3bc
5. Yang, Gu, Niu, *Block Mean Value Based Image Perceptual Hashing*, IIH-MSP 2006 — https://dl.acm.org/doi/abs/10.5555/1193214.1194040 ; implementation: https://github.com/commonsmachinery/blockhash-python
6. pHash design notes — https://phash.org/docs/design.html
7. dHash description and pHash/dHash comparison — https://arxiv.org/pdf/2109.00899 , https://arxiv.org/pdf/2211.02115
8. Brinkmann et al., *SC-Block: Supervised Contrastive Blocking within Entity Resolution Pipelines* — https://arxiv.org/pdf/2303.03132
9. Peeters et al., *WDC Products: A Multi-Dimensional Entity Matching Benchmark* — https://arxiv.org/html/2301.09521
10. Fellegi–Sunter in practice (G-LINK) — https://www.norc.org/content/dam/norc-org/pdfs/G-Link_Probabilistic%20Record%20Linkage%20paper_PVERConf_May2011.pdf ; frequency-based refinement — https://pmc.ncbi.nlm.nih.gov/articles/PMC9336505/
11. eBay duplicate listings policy — https://www.ebay.com/help/policies/listing-policies/duplicate-listings-policy?id=4255
12. Amazon duplicate detail pages / Potential Duplicates — https://www.amalytix.com/en/knowledge/strategy/amazon-listing-duplication/ , https://clickfluency.com/what-you-need-to-know-about-amazon-duplicate-listings/
13. Rust crates — https://crates.io/crates/image_hasher , https://lib.rs/crates/image_hasher , https://crates.io/crates/img_hash , https://crates.io/crates/pdf-extract , https://crates.io/crates/lopdf , https://crates.io/crates/pdfium-render , https://lib.rs/crates/simhash , https://crates.io/crates/probminhash , https://lib.rs/crates/gaoya
14. Shopify collections — https://help.shopify.com/en/manual/products/collections ; Admin GraphQL `Collection` — https://shopify.dev/docs/api/admin-graphql/latest/objects/Collection
15. Etsy shop sections — https://help.etsy.com/hc/en-us/articles/360000345048
16. TPT bundles — https://help.teacherspayteachers.com/hc/en-us/articles/360042429532
17. Tes resource bundles — https://www.tes.com/author-academy/getting-started/how-create-resource-bundles
18. Vendoo custom labels — https://blog.vendoo.co/7-vendoo-features-every-reseller-should-be-using
19. List Perfectly feature set / auto-delist — https://listperfectly.com/faq/ , https://listperfectly.com/selling/list-perfectly-pro-plus-auto-delist/
20. Notion relations & rollups — https://www.notion.com/help/relations-and-rollups
21. Stripe idempotent requests — https://docs.stripe.com/api/idempotent_requests
