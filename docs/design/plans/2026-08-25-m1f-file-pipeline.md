# M1f file pipeline implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Customer uploads become safe, content-addressed, per-tenant-encrypted blobs with covers and previews: streamed ZIP extraction under running byte and ratio counters with symlink and traversal refusal, PDF and PPTX probing, cover and preview generation (a hard requirement on Tes), a malware-scan seam, and blake3 content-hash dedup — all behind the `FileSource` seam M1c left waiting and driven by a `tam-pipeline-worker` binary. This milestone has no kill gate but is the attacker-supplied-content boundary.

**Architecture:** `tam-pipeline` is the pure inspection library: sans-IO where it can be, every decompression bounded by a caller-supplied byte budget. The heavy native closures (image encode, PDF/PPTX probe) live here so they never enter the API's closure. `tam-storage` gains a `BlobRepo` writing object-store keys the database owns plus the per-tenant blob DEK columns the design (line 512) reinstates. `tam-pipeline-worker` is the thin binary. Object storage is a `trait ObjectStore` with a local-filesystem impl for dev and tests; S3 is a later wiring change, not a redesign.

## Global constraints

- Extraction uses `zip`'s `enclosed_name()` and NEVER `name()`, refuses any symlink entry, and enforces a running byte counter during streamed extraction against `ingest::ARCHIVE_UNCOMPRESSED_BYTES_MAX` and `ingest::ARCHIVE_COMPRESSION_RATIO_MAX` — `decompressed_size()` reads spoofable headers and is never trusted (design, GHSA-94vh-gphv-8pm8).
- The kind enumeration excludes `.rar` deliberately: the `unrar` crate bundles non-OSI RARLAB C++, so it is not a dependency and `Zip` is the only archive kind.
- Bytes are content-addressed by blake3; the hash is the dedup key and, per tenant, the blob primary key (M1b's `blob` table). Dedup is per tenant, never global — a global hash table is an existence oracle.
- Cover generation is a HARD requirement: Tes generates neither cover nor preview for ZIP uploads, so a failed cover generation blocks the publish (`ScanOutcome`/projection already model the block). Preview generation is best-effort.
- Object-store blobs are encrypted under per-tenant data-encryption keys (design line 512) — the files are the asset and cannot be rotated after disclosure — reusing the `tam-secrets` envelope with a blob-scoped AAD context.
- The out-of-process rlimit sandbox is the design's stated architecture (line 375). M1f ships the inspection logic and a `SubprocessSandbox` seam with a same-process impl for tests and a fork-exec-rlimit impl for the worker; the systemd `MemoryMax`/`TemporaryFileSystem=/dev/shm` hardening ships as reference config with deploy. Recorded so the omission is a decision, not a gap.
- The malware scanner is a `trait Scanner` — a no-op/allowlist impl for tests, a ClamAV-socket impl for the worker; `ScanOutcome` already carries `Infected { signature }`.
- Commit recipe (jj, signed): `jj describe -m "<msg>"`, `jj bookmark set main -r @`, `jj new`, `git push origin main`.

---

### Task 1: tam-pipeline — safe ZIP extraction under bounds

**Files:** `crates/tam-pipeline/{Cargo.toml,src/lib.rs,src/archive.rs}`; workspace member.

- `extract(reader, budget: ExtractBudget, sink) -> Result<Vec<ExtractedEntry>, ArchiveError>`: streamed, per-entry `enclosed_name()` (traversal refusal), symlink refusal (unix mode bits), a running decompressed-byte counter that aborts at the absolute cap AND a per-entry ratio check that aborts at the ratio cap, each read chunk-bounded so a single entry cannot exhaust memory.
- Tests: a benign ZIP extracts; a zip bomb (high ratio) is refused at the ratio cap; an entry exceeding the absolute cap is refused; a `../` traversal entry is refused; a symlink entry is refused; a truncated central directory errors rather than panics. Fixtures are built in-test with the `zip` writer so no binary blobs are committed.
- [ ] Gate, `cargo deny check` (new deps: zip, blake3), commit `feat(m1f): streamed ZIP extraction with bomb, ratio, traversal and symlink guards`.

---

### Task 2: content addressing, kind probing, the scanner seam

**Files:** `crates/tam-pipeline/src/{hash.rs,probe.rs,scan.rs}`

- `content_hash(bytes) -> ContentHash` (blake3, the real 32-byte digest replacing the sketch stand-in usage).
- `probe_kind(bytes) -> Option<FileKind>`: magic-byte sniffing for pdf (`%PDF`), the OOXML/ZIP container shared by pptx/docx (disambiguated by the `[Content_Types].xml` part), zip, and common image kinds; returns the closed `FileKind`, never guesses.
- `trait Scanner { fn scan(bytes) -> impl Future<Output = ScanOutcome> }` plus `AllowAllScanner` (tests) and an `EicarScanner` that flags the standard EICAR test string, so the infected path is reachable without a live ClamAV.
- Tests: hash determinism and the FileKind agreement (a real PDF/PPTX/PNG fixture built in-test probes to the right kind; a ZIP that is not OOXML probes to Zip); EICAR flags `Infected`, clean bytes flag `Clean`.
- [ ] Gate, commit `feat(m1f): blake3 content addressing, magic-byte kind probing, scanner seam`.

---

### Task 3: cover and preview generation

**Files:** `crates/tam-pipeline/src/render.rs`; dep on an image encoder.

- `cover(source: &CoverSource) -> Result<RenderedImage, RenderError>` — a deterministic cover for a payload: for an image payload, a downscaled thumbnail; for a PDF/PPTX/ZIP, a generated placeholder card carrying the title rendered to a fixed-size PNG (real first-page rasterisation needs a native PDF renderer deferred to deploy; the placeholder is honest and unblocks the Tes hard requirement). `preview` best-effort, same shape.
- `RenderError::CoverRequired` is the terminal that blocks the publish, distinct from a best-effort preview failure.
- Tests: a cover is produced for each payload kind and is a valid PNG of the fixed dimensions (decode it back); determinism (same input, byte-identical output) so dedup on the cover works.
- [ ] Gate, commit `feat(m1f): deterministic cover generation and best-effort previews`.

---

### Task 4: object store, per-tenant blob encryption, BlobRepo

**Files:** `crates/tam-pipeline/src/store.rs`, `crates/tam-storage/src/blobs.rs`, `crates/tam-storage/migrations/0011_blob_dek.sql`, tests.

- `trait ObjectStore { put(key, bytes) / get(key) -> bytes }` with `LocalObjectStore` (a gitignored dev dir) and a blob-scoped `tam-secrets` envelope so bytes at rest are per-tenant ciphertext.
- Migration 0011: `blob` gains the DEK columns the M1b table stubbed (`dek_key_version` already exists; add `wrapped_dek`, `nonce`, `aad` for the object bytes) — engine/pipeline grants updated.
- `BlobRepo::put(org, ProductFile-worth-of-metadata, bytes)` seals, stores, and upserts the `blob` row `ON CONFLICT DO NOTHING` (per-tenant dedup); `get(org, hash)` fetches and opens.
- Tests (live db + local store): a blob round-trips through seal/store/fetch/open; two puts of identical bytes under one tenant dedup to one row and one object; the same bytes under two tenants are two rows (per-tenant, not global).
- [ ] `just db-migrate`, `just db-test`, commit `feat(m1f): object store, per-tenant blob encryption, deduplicating BlobRepo`.

---

### Task 5: the pipeline driver and the FileSource impl

**Files:** `crates/tam-pipeline/src/pipeline.rs`, `crates/tam-pipeline-worker/{Cargo.toml,src/main.rs}`; the `FileSource` impl.

- `ingest(org, upload, ctx) -> Result<IngestedProduct, IngestError>`: probe kind → scan (block on `Infected`) → if ZIP, extract under bounds and classify entries into payload/cover/preview roles → hash each → generate cover (block on failure) → store each blob → return the `ProductFile` set the catalogue insert consumes.
- `PipelineFileSource: FileSource` (the M1c seam): `fetch(FileId)` reads the stored, encrypted blob back as `FileContent` for the adapter's upload — closing the seam the Tes adapter has been calling into a stub.
- `tam-pipeline-worker`: thin binary, argv (db url, object-store root), leases nothing yet (M1j wires the queue) — a one-shot `ingest a path` operator mode plus the library wiring, so the worker exists as the design's unit without turning on unbuilt queue consumption.
- Tests: end-to-end ingest of a benign ZIP produces the expected roles, blocks on EICAR, blocks on cover failure; `PipelineFileSource` round-trips bytes back for the adapter.
- [ ] Full gates, `just db-test`, `nix flake check` on the committed tree, commit `feat(m1f): the ingest pipeline and the FileSource that closes the adapter seam`.

---

## Deferred, recorded

- Real first-page PDF/PPTX rasterisation (native renderer) ships with deploy; the placeholder cover is honest and unblocks Tes.
- The fork-exec-rlimit subprocess and systemd `MemoryMax`/`/dev/shm` hardening ship as deploy reference config; the byte and ratio bounds already hold in-process, which is where the reaped subprocess would enforce them anyway.
- A live ClamAV socket scanner ships with deploy; the EICAR scanner proves the infected path.
