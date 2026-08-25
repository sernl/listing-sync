# M1e credential broker implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Credentials become ciphertext only the broker can open: `tam-secrets` implements the design's XChaCha20-Poly1305 envelope with tenant-bound AAD; the broker gets its own database role, the KEK, and the one narrow unix-socket surface (link, lease, revoke, health); a lease is an authenticating gateway endpoint so a worker can use a connection and cannot read one; revocation is drilled with its wall-clock recorded; and the key-escrow recovery is a passing test before any customer data exists. This is the breach-surface floor.

**Architecture:** The custody seam stays in `tam-marketplace` where the compile forced it (M1a). `tam-secrets` is pure crypto (chacha20poly1305 0.11, zeroize; no I/O). The broker's protocol and gateway live INSIDE `tam-session-broker` deliberately — the privilege boundary's logic must not be linkable by other processes. The worker-side socket client joins `tam-engine`. The design's browser-era lease ("a driver endpoint for a browser the broker primed") adapts to the JSON-API era as a local authenticating gateway: the broker proxies allow-listed `tes.com` routes with the cookie injected server-side, which also makes the gateway the enforcement point for the design's route allow-list — a first-milestone control, not a hardening pass.

## Global constraints

- XChaCha20-Poly1305 from `chacha20poly1305` 0.11 at both envelope layers; AAD bound to `org_id || marketplace || connection_id || key_version`, so a row replayed into another tenant fails authentication rather than decrypting into the wrong context (design, custody section).
- The KEK is 32 bytes read from a file path argument — the same code path serves `LoadCredentialEncrypted=` in production and a dev file locally. A tested escrow-and-recovery round trip is a REQUIRED test, because TPM binding means the first motherboard failure otherwise destroys every stored session.
- `Secret<String>`: redacting `Debug`, zeroize on drop, consumed by value at `link`. The design's disallowed-types entry against raw strings is not mechanisable as stated (String cannot be banned); the Secret type and by-value consumption are the enforcement, recorded here.
- Database custody: `tam_broker` role (BYPASSRLS like the engine's, cluster-created by the init script) holds SELECT/INSERT/UPDATE on `connection_secret` and SELECT/UPDATE on `connection`; `tam_app` is REVOKED from `connection_secret` entirely — the API process never touches ciphertext, it forwards the link offer over the socket. A test proves `tam_app`'s read is denied.
- The gateway allow-list admits exactly the write-path prefixes (`/api/v2/resources`, `/api/resources/v3/draft`) and refuses everything else with a test; roster, account-administration and payout routes are thereby structurally unreachable.
- Revocation clears ciphertext, revokes connections and aborts live gateways; scheduling halts automatically because the M1d lease scan already refuses any org without a `linked` connection — no new grants needed for the stop. The drill records wall-clock.
- No DELETE anywhere; revocation tombstones by UPDATE.
- Commit recipe (jj, signed): `jj describe -m "<msg>"`, `jj bookmark set main -r @`, `jj new`, `git push origin main`.
- Also in this milestone, as promised in the M1d gaps record: the pre-settle read-back capability fix (Task 1), since it touches the same seam.

---

### Task 1: the pre-settle read-back capability fix (M1d gap 1)

**Files:** `crates/tam-marketplace/src/lib.rs`, `crates/tam-domain/src/lib.rs`, `crates/tam-marketplace-tes/src/flows.rs`, `crates/tam-engine/tests/driver.rs`

- `SubmitEvidence` gains `landed: Option<RemoteListingId>` — the durable identifier the submit landed on, when the adapter can state it (Tes can: the create returned the id). `landed_on_route` stays as raw evidence.
- `FetchReason` gains `VerifyAttempt { attempt: WriteAttemptId }` — the pre-settle verification read, justified by the open `write_attempt` fencing row, which IS the authorisation trail.
- The machine's `SubmitResult(Ok(evidence))` row: `ReadBack { locator: Durable(landed), reason: VerifyAttempt }` when evidence carries the id; when it does not, fall back by strategy — marker locator for a marker strategy, else the halting ambiguity (`NoDurableIdentifier`), because a submit that cannot be verified must stall, not guess.
- The Tes adapter fills `landed`; the driver test asserts the durable path.
- [ ] Gate, `just db-test`, commit `fix(m1e): pre-settle read-back rides the write-attempt capability`.

---

### Task 2: tam-secrets — the envelope

**Files:** `crates/tam-secrets/{Cargo.toml,src/lib.rs}`; workspace member.

- `Secret<String>` (zeroize, redacting Debug, `expose()` borrow + `into_inner` by value), `Kek::from_file(path)` (32 bytes, zeroizing), `AadContext { org, marketplace, connection, key_version }` with a fixed-width canonical encoding.
- `seal(kek, context, secret) -> Sealed { key_version, wrapped_dek, nonce, ciphertext }` — fresh 32-byte DEK per seal, DEK wrapped under the KEK (its own XNonce prepended inside `wrapped_dek`), payload under the DEK with the context as AAD at both layers.
- `open(kek, context, sealed) -> Secret<String>` — total errors: wrong KEK, wrong context (the replay test), truncated material.
- Tests: round trip; escrow recovery (seal, reload the KEK from a copied file, open); wrong-KEK refusal; cross-tenant replay refusal (same row, different org in the context); Debug redaction.
- [ ] Gate, `cargo deny check` (new crypto deps), commit `feat(m1e): tam-secrets envelope with tenant-bound AAD and escrowable KEK`.

---

### Task 3: the broker role and custody grants

**Files:** `db/init/01-app-role.sql`, `crates/tam-storage/migrations/0010_broker_custody.sql`, `crates/tam-storage/tests/custody.rs`

- Init script: `tam_broker` LOGIN BYPASSRLS (cluster-level, superuser-created), applied to the running dev database.
- Migration 0010: REVOKE ALL on `connection_secret` FROM `tam_app`; GRANT SELECT, INSERT, UPDATE ON `connection_secret` TO `tam_broker`; GRANT SELECT, UPDATE ON `connection` TO `tam_broker`.
- Test: `tam_app` SELECT on `connection_secret` is DENIED (the severity test for "only the broker's role may select"); `tam_broker` can read it.
- [ ] `just db-migrate`, `just db-test`, commit `feat(m1e): the broker database role; the app role can no longer read ciphertext`.

---

### Task 4: the broker — protocol, gateway, revocation

**Files:** `crates/tam-session-broker/src/{main.rs,protocol.rs,vault.rs,gateway.rs,service.rs}`, manifest deps (tam-secrets, tam-storage? no — direct sqlx over its own narrow queries, tam-types, serde, axum, reqwest, tokio)

- Line-delimited JSON over the unix socket: `link { org, connection, marketplace, cookie_header }` (offer consumed, sealed, inserted, connection → linked), `lease { org, connection, purpose, grant }` → `{ endpoint, expires_ms }`, `revoke { org, connection } | revoke_all {}` → counts + elapsed ms, `health {}`.
- The vault: broker-role pool, seal/open via tam-secrets, key_version 1.
- The gateway: per lease, an ephemeral 127.0.0.1 listener proxying method, path, query, body and content-type to the upstream base (tes.com in the binary; injectable for tests) with the Cookie header injected; the route allow-list refuses everything outside the two write-path prefixes with 403; leases expire and abort; revocation aborts immediately.
- `main`: args `<socket-path> <broker-db-url> <kek-path>`; the M1a stale-socket handling stays; `revoke-all` also available as a one-shot subcommand for the operator drill.
- Worker side: `tam-engine::custody` — a socket-client `ConnectionProvider` impl; `ReqwestTransport` gains a variant targeting a lease endpoint (tes.com URLs rewritten to the gateway, S3 URLs direct — S3 posts are presigned and carry no cookie).
- [ ] Gate, commit `feat(m1e): the broker — sealed link, gateway leases, drilled revocation`.

---

### Task 5: the drill and the gateway tests

**Files:** `crates/tam-session-broker/tests/broker.rs`

- Against the live database and an in-process fake upstream: link seals and flips the connection; a leased gateway forwards an allow-listed route WITH the cookie while the client never saw it; a non-listed route is refused 403; revoke_all clears ciphertext (row tombstoned), revokes the connection, kills the gateway (connection refused), and prints its wall-clock — the drill, as a test that always runs.
- [ ] `just db-test` extended to the broker crate, full gates, `nix flake check` on the committed tree, commit `feat(m1e): the revocation drill and gateway allow-list, proven`.

---

## Deferred, recorded

- The systemd unit hardening (`LoadCredentialEncrypted=`, `ProtectSystem=strict`, syscall filter) ships as reference config with the deploy milestone; the KEK-from-file path is the same code either way.
- `SellerDrivenSession` and the courier stay future `CustodyModel` variants behind the existing seam, per the design's kill-gate wording.
- pg_notify lease-kill fan-out to workers lands with the M1j pump wiring; today revocation kills gateways, and the lease scan's linked-gate stops new work.
