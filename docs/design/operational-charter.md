# Engineering charter: operating the system

This file carries sections 9 through 18 of the engineering charter and is called the operations file throughout the set.
Sections 1 through 8 are in [`engineering-charter.md`](engineering-charter.md), called the rules file below: what the charter is for, the rules adopted, modified and rejected, the assertion discipline, the limits crate, and the testing pyramid.
The lint table, `clippy.toml`, `deny.toml`, release profile, limits module and flake checks are in [`enforcement-toolchain.md`](enforcement-toolchain.md), called the toolchain file below.
Section numbering runs continuously across this file and the rules file, so a reference to a section number resolves the same way in either.

Everything here is the half of the charter that governs a running system rather than the code being written: where secrets rest, what happens when the disk dies, how a schema changes without taking the site down, what is recorded of an automation run and how long it may be kept, how a marketplace redesign is contained, what an attacker can spend, what an injected document can reach, how a deploy comes back off, what the law requires of the design, and what the whole thing costs.
The split is by responsibility rather than by importance, and the second half is where the failures that end a business live.
Automation runs on infrastructure this project operates, so several of these sections were rewritten rather than adjusted when that was settled, and each of those says so where it matters.

## 9. Secret handling

Neither source document has a secret in it, because TigerBeetle's adversary is a corrupted disk sector and Holzmann's is a cosmic ray, so both are silent on the class of bug where the program is entirely correct and simply writes the wrong string somewhere durable.
That class is the one this system has, and it gets the same treatment this charter gives to bounds: a type the compiler enforces and a lint an agent cannot satisfy by being careful.

| Class | Examples | Where it rests | Worst outcome if leaked |
|---|---|---|---|
| Money | Stripe secret key, Stripe webhook signing secret | `/run/secrets` on the server | Charges and refunds issued as the vendor |
| Spend | LLM provider API key | `/run/secrets` on the server | An unbounded third-party bill |
| Identity | JWT signing key, client and worker API tokens | Signing key in `/run/secrets`; token hashes in Postgres | Impersonation of any tenant |
| Access | Marketplace credentials, and the session cookies minted from them | Envelope-encrypted in Postgres, one data key per tenant | Account takeover on every connected storefront |

The fourth row is the one an earlier draft claimed the architecture had already solved, and it has not.
Sellers supply their marketplace credentials and this system stores them, so the most damaging secret in the product is one this codebase does possess, and the section that follows is about custody rather than about absence.
The strongest control available for any secret is still not holding it, and the honest statement is that this design does not yet apply that control where it matters most.
The founder recorded the arrangement as interim — "until we find a better way to have their creds" — which is why credential acquisition sits behind a seam from the first commit rather than being written directly into the automation worker.

### The `Secret<P>` type

The `secrecy` crate (0.10.3, verified on crates.io 2026-08-24) is the obvious dependency and is worth reading before citing, since from 0.10 its wrapper is `SecretBox<T>` with `SecretString` and `SecretSlice` aliases and the traits are `ExposeSecret` and `ExposeSecretMut`, so a charter instructing an agent to use `secrecy::Secret` produces code that does not compile.
Define the type locally instead, because a secret should carry its purpose in the type so a Stripe key cannot be passed to the function that calls the LLM provider, which is making illegal states unrepresentable applied to credentials.
The skeleton below is illustrative and uncompiled.

```rust
// crates/secrets — no dependencies beyond zeroize.
pub trait Purpose: 'static { const NAME: &'static str; }

/// Never `Display`, `Serialize`, `Deserialize`, `Clone` or `PartialEq`.
/// `Debug` prints `Secret<{P::NAME}>(redacted, N bytes)`.
pub struct Secret<P: Purpose> { bytes: Zeroizing<Vec<u8>>, purpose: PhantomData<fn() -> P> }

impl<P: Purpose> ZeroizeOnDrop for Secret<P> {}

impl<P: Purpose> Secret<P> {
    /// Reads the file directly into the wrapper. There is no constructor
    /// taking a `String`, by design.
    pub fn read_from(path: &std::path::Path) -> Result<Self, SecretError> { /* .. */ }
    /// The single greppable exposure point in the workspace.
    pub fn expose_secret(&self) -> &[u8] { &self.bytes }
}
```

Five details carry the weight, beginning with drop behaviour.
`Zeroizing<Vec<u8>>` supplies the drop behaviour without a hand-written `Drop`, which is what zeroize's documentation recommends for types with invariants, and zeroize's `alloc` feature carrying the `Vec` implementation is on by default (`default = ["alloc"]`, read from the crate's `Cargo.toml`).
The absence of `Display` matters more than the redacting `Debug`, because `format!("{}", key)` is what an agent writes when building an `Authorization` header and it must not compile; the absence of `Clone` means the number of copies in memory is the number in the source, which is what makes zeroizing meaningful at all.
There is no constructor from `String`, because zeroize makes no promise about a buffer already moved, reallocated or copied before it was wrapped, and `Deserialize` is prohibited rather than merely omitted, because secrets arrive by being read from a path rather than parsed out of a structure something else might also serialise.
`expose_secret` is deliberately the one name in the workspace that hands out plaintext, so the exposure inventory is an `rg` invocation: a CI check counts its call sites and fails when the count changes without the committed inventory file changing alongside, which is the greppable-inventory pattern already used for `#[expect(clippy::infinite_loop, ...)]`.
An earlier draft put a crate-scoped `clippy.toml` behind this, banning `String`, `std::env::var` and `std::fs::read_to_string` in the secrets and configuration crates, and that mechanism does not exist: a crate-local file replaces the workspace-root one rather than adding to it, which the toolchain file measures.
What holds instead is that `std::env::var`, `std::env::vars` and `std::fs::read_to_string` are denied in the one workspace-root list, so the configuration crate's single legitimate read carries an `#[expect]` and is greppable, while a per-crate ban on `String` has no configuration form at all and is carried by the type: `Secret<P>` has no constructor taking a `String`, so a credential field of that type cannot be built from one.

### Deployment, rotation, and encryption at rest

Use sops-nix, whose module (read at `modules/sops/default.nix` on the upstream default branch, 2026-08-24) gives `sops.secrets.<name>` with `path` defaulting to `/run/secrets/$name`, plus `mode`, `owner`, `group`, `neededForUsers`, `restartUnits` and `reloadUnits`, and a `sops.defaultSopsFile`.
agenix is an acceptable substitute and is simpler, but `restartUnits` is the deciding property, because it makes rotation a declarative consequence of changing an encrypted value rather than a checklist someone follows, which is the difference between a rotation story and a rotation intention.
Secrets are decrypted at activation, so a rotation is a deploy and a deploy is the minutes-long operation of section 16, which removes the usual reason rotations do not happen.

The order of preference is not to hold the value, then to hold a hash, then to hold ciphertext.
Enrolment tokens and any API token this system issues are stored as an Argon2id hash (`argon2` 0.5.3) rather than as recoverable text, which makes encryption at rest moot for the largest population of secrets in the database, and card data is never held at all, since Stripe's hosted surfaces mean this system holds only a customer identifier.
Where a value must genuinely be recoverable, use envelope encryption: a per-row data key under an XChaCha20-Poly1305 AEAD (`chacha20poly1305` 0.11.0), that key wrapped by a key-encryption key read from `/run/secrets`, and a `key_version` column on the row, so rotation is a re-wrap of data keys rather than a re-encrypt of content — a bounded job rather than a migration.
Every secret has a written rotation procedure with the same four phases, which are what make rotation non-disruptive rather than an outage: introduce the new version alongside the old, deploy code accepting both, re-sign or re-wrap or re-issue against the new version, and remove the old version in a later deploy.
For JWTs this means a key identifier in the token header and a verification key set rather than a single key, so rotation never invalidates a live session; for the Stripe key it means rolling the key in Stripe's dashboard and deploying the new value, and while Stripe provides a grace window on a rolled key the exact duration is unverified here and should be read from Stripe's documentation at rotation time.
Rotation cadence is a scheduled job that opens an issue rather than a check that fails the build, because a build that breaks on a date is a build an agent will route around, which is this charter's own criterion for a rule not worth having.

### Credential custody, and the seam that admits a better model

Because this system holds the credential, the design question is not whether to hold a secret but how narrow the window of exposure can be made while it is held.
Encrypt each connection under its own data-encryption key with an authenticated cipher, binding the additional authenticated data to the tenant, the marketplace, the connection and the key version, so a ciphertext row replayed into another tenant fails authentication rather than decrypting into the wrong storefront.
Persist a minimal enumerated cookie allow-list per marketplace origin rather than a whole browser profile, because a profile accumulates autofill and third-party state nobody asked for and nobody can audit.
The data key exists in the database only wrapped; the key-encryption key that unwraps it is delivered as a systemd credential encrypted against the machine's TPM, so a stolen database dump or a disk image without that TPM yields ciphertext and nothing else.
TPM binding ties the deployment to one machine, so a tested key-escrow and recovery procedure has to exist before any customer data does, or the first motherboard failure destroys every stored connection; that procedure belongs in the restore drill below rather than in a separate intention.

Decryption is not ambient, which is what bounds a compromise of the automation worker.
A session-broker unit is the only component that reaches the key-encryption key: it runs as its own user under a strict system-protection profile with no new privileges and a syscall filter, and it exposes one narrow call that leases an already-authenticated browser endpoint for a named connection and purpose.
It exposes no call that returns a credential.
A compromised worker can therefore misuse the connections it has been leased and cannot exfiltrate the store, and the ceiling is worth stating plainly: root on the box, or a bug in the broker, gets everything currently unsealed.
The structural enforcement is a `cargo-deny` `[bans]` entry giving the browser-driver crate exactly one permitted parent, the automation worker, so no axum, telemetry or client-facing crate can reach a live session; that entry checks direct parents only, which the toolchain file measures, and where transitive reach must be excluded the mechanism is the dependency-closure check rather than the ban.

The seam exists because that ceiling is the reason to stop holding credentials at all, and because it is cheap now and expensive to retrofit.
Credential acquisition is one trait with one implementation today and room for the better ones, two of which the decision register names: an interactive remote browser the seller logs into themselves, and an official partner or delegated-access integration where a marketplace grants one.
The research adds a third, acquisition on the seller's own device by a small helper that mints the session there and posts only the cookies, and the three are not equally good.
Order them by exculpability rather than by cryptography: a delegated-access grant is best, because the password never exists on this side and the marketplace itself holds the record of the grant; acquisition on the seller's own device is next, because the password provably never transits this infrastructure; and an interactive remote browser is last, because the keystrokes do transit it and no log or policy this project holds can afterwards demonstrate that they were not captured.
The seam is a real boundary rather than a comment: nothing outside the credential crate constructs a credential, nothing outside the broker decrypts one, and adding an implementation is a new type in that crate rather than an edit to any caller.

Retired 2026-09-04 by founder decision D1, and this section is kept as the record of what was built rather than as a description of what runs.
There is no session-broker unit and no component on the serving host that reaches a key-encryption key, because there is no longer a server-side seller session for a no-API marketplace to hold a credential for: every TeachersPayTeachers and Tes request originates on the seller's own device under the seller's own session.
What the section decided still stands and is what D1 chose. It ordered the three acquisition options by exculpability and put acquisition on the seller's own device second only to a delegated-access grant; the architecture went to that option outright rather than to the interactive remote browser it ranked last.
The systemd unit, the TPM-bound credential delivery and the escrow drill they require apply to whatever server-side secret the sanctioned-token branch comes to hold, and to nothing running today. `connection_secret` keeps its rows, which are the only copy of what was sealed before this, and disposing of them is a later founder call.

## 10. Backup, restore, and a rehearsed restore drill

One box, no failover, and paying customers whose income-producing files are on it: every previous section is about not shipping a bug, and this one is about the case where the bug has already shipped, or the disk has already failed, and the only question left is how much is gone and how long until it is back.
The listing catalogue, the job ledger, the tenant records and the billing state exist only in Postgres on this machine, while the uploaded resources exist here, on the seller's own machine, and — once published — on the marketplace, so the database is the only single copy in the system and gets the tighter numbers.

| Data | RPO | RTO | Mechanism |
|---|---|---|---|
| Postgres | 15 minutes | 1 hour | pgBackRest, WAL archiving, PITR |
| Object storage | 1 hour | 4 hours | restic, hourly snapshot |
| Deployment configuration | 0 | minutes | The flake repository, mirrored offsite |
| Secrets | 0 | minutes | sops-encrypted in the repository; the age key held offline |

The Postgres number is a commitment rather than a measurement of the mechanism: `archive_timeout` set to 60 seconds forces a WAL segment at least every minute, so the achieved figure should be far better than fifteen minutes, and committing to the worse number leaves headroom for the case where the archive push is the thing that failed.
`services.pgbackrest` is in nixpkgs and present on the current stable channel (verified: the module exists on `nixos-25.05`, `nixos-25.11` and `nixos-26.05`), exposing `repos`, `stanzas`, and per-stanza `jobs` with `schedule` and `type`.
Configure two repositories, because the module indexes them (`repo1-`, `repo2-`) and pgBackRest searches them in order: a local repository on a filesystem separate from the data directory, and an offsite one.
Point-in-time recovery is `pgbackrest restore --type=time --target='<timestamp>' --target-action=promote`, with `--set` to pin a specific backup; pgBackRest's command reference confirms that for `--type=time` the target must be given with `--target` and that it searches the configured repositories in order for a backup containing the requested time.
`services.restic.backups.<name>` covers the object store, with `repository`, `paths`, `passwordFile`, `timerConfig`, `pruneOpts`, `runCheck` and `checkOpts` all present as module options, and `rcloneConfigFile` for an offsite target restic does not speak natively.
One default is a trap of exactly the kind this charter records elsewhere: `runCheck` defaults to `builtins.length checkOpts > 0`, so a backup configured without `checkOpts` runs no integrity check at all and reports success either way, and the fix is to set `checkOpts` explicitly and rotate `restic check --read-data-subset=n/t` across the schedule so every pack file is actually read over `t` runs rather than checking only the metadata forever.
Retention is bounded and stated rather than left to grow, which is partly a storage decision and mostly a data-protection one, for the reason in section 17.
One circular dependency will otherwise be discovered during the incident rather than before it: restic encrypts with a repository password and sops-nix decrypts with an age key, so if the only copies of either live on the machine being restored then there is no backup, only an archive nobody can open, which is why the recovery key material lives offline and outside the deployment and why the drill is what proves it does.

### The drill

A backup that has never been restored is a hypothesis; the drill is the experiment, it is scheduled, and it has a pass criterion a program evaluates.
Run it quarterly, and monthly for the first two quarters while the procedure is still wrong in ways nobody has found: provision a scratch NixOS virtual machine from the same flake, restore Postgres to a chosen timestamp, restore the object store, boot the application, and run the verification script, recording the result under `docs/ops/restore-drills/` so a drill that did not happen is visible as a gap in a directory listing rather than invisible as an absence of memory.

1. The restored database's applied migration set, per `sqlx migrate info`, matches the set the restored binary embeds.
2. For a tenant chosen before the drill, the listing count, the most recent job-ledger row and the Stripe subscription state match values recorded before the drill began.
3. Every object referenced by a `files` row is present in the restored object store and its content hash matches the stored hash.
4. A synthetic sync job runs to completion against the fake adapter on the restored system.
5. Wall-clock time from the start of the restore to assertion four passing is inside the stated RTO, measured rather than estimated.

A failed drill is an incident with a written follow-up, and a skipped drill counts as a failed one, which is the rule that determines whether this section survives contact with a busy quarter.
The drill does double duty, which is what makes it affordable for one person: a restored production-shaped database is precisely the environment section 11 needs for timing pending migrations, so the drill and the migration rehearsal are one activity, and it exercises the secret path as well, because a freshly provisioned box has no secrets until sops-nix decrypts them with a key that has to exist somewhere other than the box that died.

### Bus factor and wind-down

The question everyone defers is what happens to customer data if the founder stops, and it has an engineering answer cheaper to build early than to improvise later.
Tenant export is self-service and does not route through a human, which is required anyway by the data-protection duties in section 17, so building it once discharges both obligations and the answer to "the founder stopped" is never "email support".
A dead-man's switch — a scheduled job requiring periodic human acknowledgement — triggers a defined sequence on lapse: stop billing first, because a subscription that keeps charging while nobody operates the service is worse than the service being down; notify a named second party who holds the recovery material; and after a further interval place the service into read-only export mode rather than letting certificates expire and the box become unreachable with the data still inside it.
The recovery bundle is the flake repository, the sops age key, the restic repository password, the escrowed key-encryption key without which every stored marketplace connection is unrecoverable ciphertext, and the account-recovery paths for the domain registrar and Stripe, held by that named second party, who runs at least one restore drill themselves, because a drill only the founder can pass measures the founder rather than the system.
The tension in that bundle is real and should not be smoothed over: the material that makes the business survivable also makes one other person able to decrypt every seller's connection, which is an argument for the credential seam's better implementations rather than an argument for skipping the escrow.
The wind-down mechanics are then ordinary — a stated notice period, an export window with the API read-only, and a scheduled deletion after it — and the shape of the product makes this survivable, since sellers uploaded their own source files and still hold them and their published listings are already live on marketplaces this platform does not control, so the platform disappearing costs them their automation and their catalogue view rather than their income.
One step is specific to holding credentials and belongs in the sequence rather than in a policy: revoke and destroy every stored connection at the end of the export window, and tell sellers to rotate the marketplace passwords they gave this system, because a wind-down that leaves ciphertext in a backup nobody operates is worse than one that does not.

## 11. Schema and data migration discipline

The rules file's section 5 carries a job-ledger example that treats a row whose `state` column holds a value this build's enum does not recognise as a live case, written by an older or newer deployment, and that case is live only because two versions of the code touch one schema during a deploy and during a rollback.
Everything here is about keeping that window short, keeping it safe, and making the unsafe deploy the one you can point at.

sqlx makes the reversibility decision sticky, because creating the first migration with `sqlx migrate add -r` puts the project in reversible mode and, per the sqlx-cli documentation, all subsequent migrations will be reversible as well, so this is a choice made at the first migration and should be made deliberately rather than by whichever flag an agent typed first.
Choose forward-only: a `down` script is code that never runs in production and is therefore never tested, and it cannot restore data its corresponding `up` destroyed, so the rollback of a migration is a new forward migration and the rollback of lost data is a restore.
sqlx's checksum enforcement is already the machine check for the most common agent mistake, since editing an applied migration produces `MigrateError::VersionMismatch` — "migration {0} was previously applied but has been modified" — and a migration timestamped before the latest applied one produces `VersionTooOld`, which catches the merge-order hazard; state both here so an agent hitting them fixes forward rather than deleting the row from `_sqlx_migrations`.

### Expand and contract

Every schema change is decomposed into phases, and the phase table is the whole rule.

| Phase | Schema change | Code | Previous generation still runs |
|---|---|---|---|
| Expand | Add nullable column, new table, or index concurrently | Ignores the new shape | Yes |
| Backfill | None | Bounded, resumable, idempotent job | Yes |
| Read | None | Reads new, falls back to old | Yes |
| Write | None | Writes both | Yes |
| Contract | Drop the old column or constraint | New only | No |

Exactly one deploy in that sequence is not rollback-safe and it is the last one, which ships alone, after the new code has soaked in production for a stated period, and is the only deploy whose rollback plan is something other than switching generations.
The machine check is a migration lint: a CI step parses each new file under `migrations/` for the destructive verbs — `DROP TABLE`, `DROP COLUMN`, `ALTER COLUMN ... TYPE`, `ALTER COLUMN ... SET NOT NULL`, `RENAME` — and fails unless the file carries a marker comment naming the expand migration it contracts and the soak period that has elapsed.
That comment is load-bearing and belongs in the carve-out list alongside linter pragmas and code-generation markers, because tooling parses it rather than a person reading it, and it is the same pattern as `#[expect(clippy::infinite_loop, reason = "...")]`: the hazard is permitted, flagged by a checker, and justified in the file.

### Locks are how a safe migration takes the site down

The failure mode is rarely a wrong schema and usually a correct schema acquired badly: `SET NOT NULL` takes an access-exclusive lock and scans the table, a non-concurrent `CREATE INDEX` blocks writers for the duration, and every one of these queues behind and ahead of ordinary traffic, so a migration that would have taken ninety seconds takes the API down for ninety seconds plus however long the blocked connections take to time out.
Set `lock_timeout` and `statement_timeout` on the migration session so a blocked migration fails fast instead of becoming an outage, and admit that lock timeout to `crates/limits` under the admission rule, since a migration holding an access-exclusive lock is a shared-resource incident by definition.
A migration that could not get its lock is a migration to retry at a quieter moment, which is a far better outcome than one that got it and held it.
`CREATE INDEX CONCURRENTLY` is the specific exception needing its own handling, and PostgreSQL's documentation states both halves plainly: it cannot be performed within a transaction block, and if it fails it leaves behind an index marked `INVALID`, whose recommended recovery is to drop the index and try again or to rebuild it with `REINDEX INDEX CONCURRENTLY`.
sqlx supports this through a directive whose exact form matters, because the source resolver tests `sql.starts_with("-- no-transaction")` (`sqlx-core/src/migrate/source.rs:234`), so the comment must be the very first bytes of the file and a licence header, a blank line, or a descriptive comment above it silently disables the directive and the migration runs inside a transaction, where it fails.
Concurrent index builds therefore live alone in their own migration file, and the CI migration lint asserts that any file containing `CONCURRENTLY` begins with that exact directive.

### Testing and backing out

Two mechanisms, and the first is free: `cargo sqlx prepare --workspace` writes query metadata to `.sqlx`, which is committed, so compile-time-checked queries are pinned against a schema snapshot and a migration that invalidates a query fails the build.
That is a genuine machine check that the new code and the new schema agree, at no ongoing cost, and it is the strongest single item in this section — but it proves nothing about the old code against the new schema, which is the case the rollback depends on, and nothing mechanical covers that gap, so the expand-and-contract phase table closes it by discipline and this document says so rather than implying the check is broader than it is.
The second mechanism is the restore drill's virtual machine: run the pending migrations there against real row counts, real index sizes and real bloat, and record total migration duration and the longest lock held in the drill file, because a migration whose rehearsed duration exceeds the acceptable API interruption is not ready to deploy, whatever it does.

Backing out has three cases and the distinction between them is the content.
The migration failed: PostgreSQL has transactional DDL so it rolled back and `_sqlx_migrations` has no row for it — sqlx's own `MigrateError::Dirty` variant is annotated in its source as only occurring on databases without transactional DDL — so fix and redeploy, the exception being `CREATE INDEX CONCURRENTLY`, which leaves the invalid index described above and must be dropped explicitly before retrying.
The migration succeeded and the code is wrong: roll back the generation per section 16, and because the migration was an expand-phase change the previous generation runs against the migrated schema unchanged, which is the ordinary case the phase table exists to make ordinary.
The migration succeeded and destroyed data: no generation rollback helps, so restore by point-in-time recovery to a timestamp before the migration into a scratch database and copy the lost rows forward into production, and never restore over production, which would discard every transaction since the migration including every sync every tenant ran in the meantime, turning a data-loss incident into a larger one.

## 12. Observability

An earlier draft opened this section by saying the founder cannot see or reproduce a customer's failure, and the settled architecture inverts that premise exactly.
Every part of this system, including its most failure-prone part, executes on infrastructure the founder operates, so server-side tracing, page-state capture at the moment of failure, and replay of a recorded trace are all available, and observability becomes a question of what to record rather than of reaching a component that cannot be reached.
The live constraint is retention, not visibility.
A captured marketplace page is someone else's interface and may carry a seller's account details, their sales figures and their buyers' personal data, so capture is redacted where it is taken and short-lived by construction, and the subsection below states that as three rules rather than as a preference about log volume.

Use `tracing`, and make one attribute form mandatory; the block below is illustrative and uncompiled.

```rust
#[tracing::instrument(
    skip_all,
    fields(tenant_id = %tenant, job_id = %job.id, marketplace = %target.marketplace),
    err,
)]
async fn publish(&self, ...) -> Result<PublishReceipt, AdapterError> { /* .. */ }
```

`skip_all` is not optional and not a default to be relied on, because without it `#[instrument]` records every argument through its `Debug` implementation, which is exactly how a payload containing a tenant's listing text ends up in a log line; the `Secret<P>` type closes that hole for credentials and nothing closes it for tenant content except recording fields by name.
Enforce with a grep rejecting any `#[instrument` attribute that does not contain `skip_all`, which has the same zero-false-positive shape as the grep against `&&` inside `assert!(`.
Field names come from one module of constants, in the same spirit as `crates/limits`, so a query written once keeps working: `tenant_id`, `job_id`, `listing_id`, `marketplace`, `adapter`, `adapter_version`, `attempt`, `correlation_id`, `outcome`, `failure_code`.
Values are identifiers, enums and numbers, never free text and never anything derived from a document, a page or a form field, which stated positively is a stronger rule than a deny-list of forbidden field names, because a deny-list is a list you forget to extend.
The `err` argument records the `Err` at error level through its `Display`, which makes error types part of the redaction surface, and the existing `AdapterError` design already satisfies this — `SelectorMissing { selector: SelectorId }` carries an identifier rather than a scraped selector string, and `UnexpectedNavigation { to: Url }` carries a URL that must be reduced to a route pattern before it is recorded — so the rule is a consequence of a taxonomy designed that way for other reasons rather than new ceremony.

One correlation identifier is minted at the edge and survives the whole causal chain: HTTP request, job ledger row, worker lease, browser session, adapter attempt, and the response the seller eventually sees.
The load-bearing detail is that it is a column on the job row rather than merely a field in a log line, because a job outlives the process that started it: a worker restart, a deploy, or an item parked waiting on a seller's re-authentication each break the chain a log-only identifier depends on, and those are ordinary events rather than incidents.
Writing it durably means the ledger can answer "what happened to this request" from the database alone, with the trace and any capture as corroboration rather than as the sole record.
On the wire use W3C Trace Context, the `traceparent` header with `tracestate` if a vendor ever needs it, whose specification gives the example `traceparent: 00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01`; adopting the standard format costs nothing now and means `tracing-opentelemetry` (0.33.0) can carry the same identifiers unchanged if a collector is ever added.
Do not add a collector on day one, because on one box structured JSON to the journal plus `journalctl` is adequate and the field discipline is what makes a later migration cheap, whereas building an OpenTelemetry pipeline for a single-process deployment is effort spent where the restore drill is not yet written.
Verify propagation rather than hoping for it, using this charter's own three-question test: the job-claim handler asserts that a leased job carries a non-empty correlation identifier, because the enqueue path owns that field and its absence is a bug in code this project controls, while the client-facing progress endpoint returns `Err` on a missing or unrecognised one, because that value arrived over the network from a client.

### Health, readiness, and the metric that matters

Two endpoints with genuinely different jobs, and the distinction is the point.
`/livez` reports that the process is up and the runtime is not wedged and touches nothing external, because a liveness probe that queries the database will fail when the database is slow, restarting a healthy process and converting a degradation into an outage.
`/readyz` reports that this process can serve traffic, and checks a `SELECT 1` inside the pool's acquire timeout, that the applied migration set matches the set the binary embeds, and that the object store is writable, returning a typed JSON body naming each check and its result so a failure is legible without a log dive.
Both are unauthenticated and rate-limited, emit a build identifier, and emit nothing else: no dependency hostnames, no versions of anything downstream, no counts.
The machine check is the one everybody skips — an integration test that stops the database and asserts `/livez` returns 200 while `/readyz` returns 503 — and that test is what proves the distinction was implemented rather than merely described.

One metric matters more than the rest of this section combined: `adapter_attempts_total{marketplace, operation, outcome}`, where `outcome` is `ok` or the `AdapterError` variant name.
Success rate is derived at query time rather than stored, so a new failure variant appears in the data the moment it is added to the enum with no change to the metric, which is the direct payoff of having made `AdapterError` a precise enumeration rather than a collapsed error type.
This is the only thing that tells the founder a marketplace changed their form before customers do, because no seller will report that a selector moved — they will report that it stopped working, days later, individually, after they have already decided the product is unreliable, and the aggregate of those reports arrives after the churn rather than before it.
A step change in the `SelectorMissing` rate for one marketplace is the redesign itself, observable within one scheduled sync cycle.
Alert on rate over a window with a minimum denominator, never on count, because with a small tenant base a hundred percent failure rate over two attempts is noise and an alert that cries wolf in month two is muted in month three; the threshold and the minimum attempt count go in `crates/limits` marked uncalibrated.
Alert on the absence of attempts as well, because a sync engine that stopped scheduling produces a perfect success rate and a stuck system is indistinguishable from a healthy one if you only measure the ratio, which is the simulator's safety-versus-liveness distinction applied to production instead of to a seed.

### The hard constraint is retention, not visibility

Because execution is server-side, the diagnostic material an earlier draft could not obtain is now obtainable in full: the DOM at the moment a selector went missing, a screenshot, the network exchange around a submit, and a replayable trace of the whole attempt.
That changes the question from what can be collected to what may be kept, and it has to be answered in the type system rather than in a retention policy nobody enforces, because a capture is written once and read for months.

Three rules make a capture safe to hold.
Capture is reduced where it is taken rather than filtered on read: the default stored artefact is a reduced page state — the presence or absence of each selector in the adapter's declared set as a fixed-length bitmap versioned with that set, the page's route as a matched pattern identifier rather than a URL, form field names without their values, timings, attempt numbers, adapter version, and a `FailureCode` being the `AdapterError` variant plus the `SelectorId` where one applies — and the raw page is never that artefact.
Where a raw capture is genuinely needed to diagnose drift, it is taken deliberately for one job, encrypted under that tenant's data key like any other tenant content, and given a retention short enough that it expires without anyone acting, which is what makes it short-lived by construction rather than by intention.
And the telemetry payload type admits no free text at all, so no field can become the place a page fragment ends up: every field is an enum, an identifier, a boolean or a number.

That last rule needs a mechanism, and the one an earlier draft named does not exist.
A crate-local `clippy.toml` banning `std::string::String` in the telemetry crate would replace the workspace-root ban list rather than add to it, which the toolchain file measures, so the enforcement is the crate's own public API instead: the payload is one struct whose fields are all closed types, its constructor takes those types, and it implements no conversion from a string, so a string field is a change to the type rather than an oversight in a call site.
The dependency ban that gives the browser-driver crate one permitted parent carries the other half, since there is then no code path from a live page to the wire.

Two consequences follow that an earlier draft had the wrong way round.
A failure that cannot be diagnosed is now a recording gap rather than an access problem, so the escalation is to widen the recording for that adapter and re-run, not to ask the seller for a capture they would have to approve.
And the thing being recorded is the marketplace's own interface, which is a reason to keep retention short quite apart from personal data, because a durable archive of someone else's pages is a liability that grows without anyone adding to it.

## 13. Per-marketplace kill switch and synthetic canary

These are one control with two halves: the canary detects that a marketplace changed, and the kill switch is what you do about it in the minutes before a fix exists.

| State | New leases | In-flight leases | Enqueue |
|---|---|---|---|
| `Enabled` | Issued | Run | Normal |
| `Draining` | Refused | Allowed to finish | Marked blocked |
| `Paused` | Refused | Revoked | Marked blocked |

Two states would lose the ability to stop cleanly, and a hard stop in the middle of a publish is exactly how the duplicate listing this charter cares most about gets created: `Draining` is the state you use for a deploy, and `Paused` is the state you use when the marketplace has changed under you and every further attempt is wasted against a form that no longer exists.
The enforcement point is the job-lease query and it is one place, because every unit of adapter work reaches a worker by being leased from the ledger, so a flag checked there cannot be bypassed and the state row is read by the query that was going to run anyway, whereas a flag checked in five places is a flag that is wrong in one of them.
It lives in Postgres as `marketplace_state (marketplace, state, reason, changed_at, changed_by)` rather than in configuration, because it must change without a deploy and be visible to every process at once, and it takes effect everywhere with no client involvement at all, since the workers that lease the work run on the same infrastructure as the row they read.
Two details keep a pause from causing its own incident: while paused, jobs are still created but marked as blocked by the marketplace state rather than failing, and resuming releases them through the existing outbound rate limiter rather than all at once, since a stampede on resume is a good way to get throttled immediately after recovering; and a revoked lease is a distinct outcome from a failed publish, so it must not consume an attempt from `limits::job::ATTEMPTS_MAX` or pausing during an incident quietly exhausts every in-flight job's retry budget.
Machine checks are cheap here: the state is a closed enum under the `wildcard_enum_match_arm` deny so adding a state is a compile error at every site that must handle it; an integration test pauses a marketplace mid-batch and asserts zero further publishes and no attempt-count increment; and the alerting must know about the switch, or the pause generates the alarm that the pause was the response to.

The canary is a scheduled job exercising each adapter end to end against a real account, running on the same automation plane as production work, through a dedicated canary connection in the founder's own seller account.
An earlier draft said it could not run there because the cloud plane may never hold a marketplace session, and that constraint is gone; running it beside the work it watches is now the only placement that makes it a canary rather than a second system with failure modes of its own.
The rule that replaces it is that the canary leases a browser session from the same pool under the same per-tenant mutex as everything else, so a canary run cannot starve a paying tenant's job and cannot race a real publish on the same account.
It exercises the full path with a fixed synthetic payload — authenticate, navigate, fill, create a draft, verify that the scraped-back confirmation agrees with what was sent, then retract — using create-and-retract rather than publish-and-delete so it never leaves artefacts on a real storefront, and where a marketplace has no draft state it exercises everything up to the final submit and stops, which still covers every selector on the path and every field mapping.
Daily, plus before every adapter deploy; hourly is the wrong instinct, because this is traffic against a third party with whom the product has a rate-limit relationship and a terms-of-service posture to protect, and the marginal detection latency is not worth the marginal footprint.
Put the cadence in `crates/limits` next to `marketplace::OUTBOUND_REQUESTS_PER_MINUTE_MAX` so the two numbers are read together, marked uncalibrated in the same way, and let a canary failure name the marketplace, the failing selector identifier and the adapter version, with a runbook whose first line is to consider the kill switch.

The value of a detector is proportional to how quickly you can act on what it detects, and that is why this pair is cheap here specifically.
In an organisation where a fix passes through a change-approval process and a release train, a canary tells you about an outage you are going to endure for a week, and the rational investment is in making the adapter tolerant of variation rather than in learning sooner that it broke; here `nixos-rebuild switch --flake` takes minutes, the adapter is one crate with deliberately accepted debt, and selector discovery on drift is one of only two places this system permits an LLM, so the canary's finding converts into a shipped fix inside the same working session.
The same property explains why the kill switch is one Postgres row rather than a feature-flag service: large organisations build elaborate flag infrastructure because deploying is expensive, whereas here deploying is cheap, so the flag only needs to cover the window between noticing and fixing — minutes, not weeks — and a row the lease query already reads is exactly the right amount of machinery.

## 14. Inbound abuse and the financial exploit surface

An earlier draft declared a per-tier requests-per-minute constant and named nothing that reads it, and a limit with no enforcement point is decoration, so this section's organising rule is that every bound in `crates/limits` names the single function that enforces it.

| Control | Constant | Enforcement point | Failure mode when absent |
|---|---|---|---|
| LLM spend, per tenant | `llm::CENTS_PER_TENANT_PER_DAY_MAX` | `LlmBudget::reserve` | One tenant's bill |
| LLM spend, fleet-wide | `llm::CENTS_PER_DAY_GLOBAL_MAX` (pending) | `LlmBudget::reserve` | The whole bill |
| Upload type | Allow-list in `ingest` | The streaming upload reader | Parser fed arbitrary bytes |
| Outbound fetch | Deny-by-default | The validating DNS resolver | SSRF to loopback services |
| Signup rate | `signup::PER_IP_PER_HOUR_MAX` (pending) | The signup handler | Free-tier spend farming |
| API rate, per tenant | `Tier::quota().api_requests_per_minute` | Tenant-keyed tower layer after auth | Resource exhaustion |

Two constants in that table are marked pending because they do not exist in `crates/limits` yet.
Each arrives in the same change as the function that enforces it and never before, which is the admission rule the charter states.

Route every provider call through one function, `reserve(&self, tenant: TenantId, estimate: Cents) -> Result<BudgetGrant, BudgetError>`, whose implementation is a conditional write rather than a read followed by a decision — `UPDATE llm_budget SET spent_cents = spent_cents + $2 WHERE tenant_id = $1 AND spent_cents + $2 <= $3 RETURNING spent_cents`, followed by an assertion that exactly one row came back.
This is the same shape as the job-claim path for the same reason: checking a balance, awaiting an LLM completion for several seconds, and then spending is the place-of-check-to-place-of-use bug with an unusually wide window, so reserve on an estimate before the call and reconcile against actual usage after it, because a tenant issuing concurrent requests can spend the entire daily budget inside the interval during which nothing has been recorded.
The guard cannot be bypassed if there is nowhere else to call from, so make the provider SDK a dependency of one crate and enforce that edge with a `cargo-deny` `[bans]` entry; cap per-tenant concurrency as well as spend, because spend reconciles late and concurrency binds immediately; and surface exhaustion as a typed error the tenant sees rather than as silent degradation, because a listing that quietly did not get rewritten is worse than one that reported why.
The fleet-wide ceiling is the control that actually protects the founder, because per-tenant caps bound the blast radius per tenant and multiplying tenants multiplies caps, so only a global ceiling bounds the invoice, and its breach is a hard stop and a page rather than a warning, since the failure mode is a bill and a bill cannot be declined afterwards.

Never trust an upload's filename extension or `Content-Type` header, both of which are client-supplied; sniff the leading bytes with `infer` (0.22.0) or `file-format` (0.29.0) against an allow-list of the types actually supported and reject everything else, in the streaming upload reader that already counts bytes against `http::UPLOAD_BODY_BYTES_MAX`, before a byte is written to the object store and before the file is enqueued.
One subtlety defeats naive sniffing: an OOXML package is a ZIP archive, so magic bytes cannot distinguish a PPTX from an arbitrary archive, and the discrimination is made by opening the container under the archive bounds already in `crates/limits` and inspecting its part names — which means the container is opened before its type is known and the depth, entry-count and compression-ratio bounds are doing real work at that moment.
Filenames are display data and nothing else: objects are stored under generated identifiers and no filesystem path or object key is ever derived from client-supplied text, and the version that bites is the archive entry name, so an entry that is absolute, contains `..`, or contains a backslash is rejected before extraction, with `Err` rather than an assertion because it arrived from outside.
The test is a fixture corpus with one case per hostile shape — wrong extension, ZIP bomb, traversal entry, truncated PDF, deeply nested archive — each asserting a typed `Err` and not a panic.

The cheapest server-side-request-forgery control is the correct first answer: the application fetches no user-supplied URL at all in phase 1, and that is worth stating as a policy because the feature that introduces the hazard is usually added casually.
Where a fetch becomes necessary the shape is fixed: allow only `https`; resolve the hostname yourself, validate every resolved address, and connect to the validated address rather than re-resolving the name, since the gap between checking a name and connecting to it is a DNS-rebinding hole and this charter's place-of-check-to-place-of-use rule wearing a different costume; and disable automatic redirects with `redirect(Policy::none())` and re-validate each hop in your own code, because a redirect to a link-local metadata address after a clean first hop is the classic bypass.
`reqwest::ClientBuilder::dns_resolver` (verified present on reqwest 0.13.4) is where the validating resolver is installed.
One trap deserves naming because it is exactly the kind an agent falls into: `IpAddr::is_global` is a nightly-only experimental API, the `ip` feature, tracking issue 27709, on std 1.98.0 as of 2026-08-18, so a charter saying "reject anything where `is_global` is false" produces either a switch to a nightly toolchain or a hand-rolled range check that misses IPv6-mapped IPv4, carrier-grade NAT and link-local; use `ip_network` (0.4.1), whose `IpNetwork::is_global` is available on stable, or an explicit range deny-list in `crates/limits`.
The deny-list must cover the local equivalent of a cloud metadata service: this box has no metadata endpoint, which removes the highest-value target outright, but it does have Postgres and an admin surface on loopback, and below the application systemd's `IPAddressAllow` and `IPAddressDeny` on the service unit constrain egress independently of application correctness, which is worth having precisely because it survives an application bug.

Signup abuse is arithmetic: a per-tenant spend cap multiplied by unlimited tenants is an unlimited spend cap, so the binding control is on signup rather than on spend.
Require a verified email before any LLM-consuming action and enforce it inside `LlmBudget::reserve` rather than scattered through handlers, so it is one check in one place and an unverified tenant receives a typed `BudgetError` rather than a partially-completed job; rate-limit signups per source address and per email domain with `governor` (0.10.4), whose `DefaultKeyedRateLimiter` maintains one state per key, in the signup handler; and cap the number of tenants one owner may create, which is the control most often missing when per-tenant caps are the design.
Do not maintain a disposable-domain blocklist, which is a treadmill that consumes attention and expires.
Requiring a payment instrument before the first LLM call is the strongest control and the largest conversion cost, which makes it a product decision rather than an engineering one and puts it in the question list below: the engineering supports either answer, but the free-tier answer makes the fleet-wide ceiling load-bearing rather than precautionary.

Give the per-tier API rate a tower layer keyed by the `TenantId` extracted from the authenticated claims, placed after authentication and before the router, using `tower_governor` (0.8.0) or `governor`'s keyed limiters directly.
Placement is the substance, because before authentication you can only key by source address, which is the right key for a signup limit and the wrong key for a tenant limit, so there are two limiters at two points and conflating them yields one limiter that works for neither.
Weight by cost rather than counting requests, because an upload and a batch enqueue are not one request's worth of anything, with the weights coming from `crates/limits` like every other number; respond with 429 and `Retry-After`, and require the client to honour the header rather than applying its own backoff schedule, or the limiter and the client argue with each other for the duration of the incident.
The limiter's state is in-process and does not survive a restart, which is acceptable on one box with one API process and is recorded here as an assumption that breaks if a second process ever appears, the same conditional framing this charter applies to its panic policy.
The tests are the two that fail silently otherwise: exceed a tier's limit and assert 429 with `Retry-After`, and assert that two tenants' limits are independent, since a keying mistake produces a limiter that works perfectly in a single-tenant test.
The rule that generalises is that every public constant in `crates/limits` carries a doc comment naming the function that enforces it, and a CI check asserts that every public constant in that crate is referenced from at least one non-test site, so an unreferenced bound is a build failure rather than a review finding.

## 15. Prompt injection

This is the one genuinely novel trust boundary in the system: an attacker supplies a PDF, deterministic Rust extracts its text, the text enters a prompt, a model's output becomes listing copy, and the copy is published to a real storefront under a paying seller's name — so the attacker's input and the victim's storefront are joined by a path with no human on it unless one is deliberately placed there, while every other untrusted input in this system terminates in a `Result` rather than in a published artefact.
The attacker is not only a third party: the uninteresting case is a tenant injecting their own listing, which harms only themselves, and the interesting cases are resources acquired elsewhere and re-uploaded, any future shared, collaborative or imported content, and a file supplied by a competitor.
The vector is documented, since OWASP's own reference list for this risk includes Kai Greshake's "Inject My PDF: Prompt Injection for your Resume", which is precisely this shape: a document-processing pipeline whose input is a file a stranger chose.

OWASP's Top 10 for LLM Applications entry LLM01:2025 Prompt Injection (genai.owasp.org, read 2026-08-24; the 2025 list is the current edition on that site) sets out seven mitigation strategies — constrain model behavior; define and validate expected output formats; implement input and output filtering; enforce privilege control and least privilege access; require human approval for high-risk actions; segregate and identify external content; and conduct adversarial testing and attack simulations — and its scenario list names payload splitting, multilingual and obfuscated encodings, adversarial suffixes and multimodal injection as distinct cases.
NIST AI 100-2 E2025, "Adversarial Machine Learning: A Taxonomy and Terminology of Attacks and Mitigations" (March 2025), supplies the surrounding taxonomy, and MITRE ATLAS identifies this specific case as AML.T0051.001, indirect prompt injection.
One sentence from the OWASP entry governs the design and is worth quoting rather than paraphrasing: "Given the stochastic influence at the heart of the way models work, it is unclear if there are fool-proof methods of prevention for prompt injection."
Take that literally, and arrange every control below so a successful injection cannot cause an unreviewed publish: containment, not prevention.

Instructions and content occupy different channels, so extracted document text is passed through the provider's structured message roles as a labelled, delimited data block, never concatenated into the instruction string, with the system prompt stating that the block is untrusted reference material containing no instructions, and where text must be inline the delimiter carries a per-request nonce so the content cannot close its own block.
Be honest about what this buys: delimiting is a mitigation rather than a boundary, and a determined injection can still persuade a model that an ignored delimiter is a delimiter that was not there, so the weight is carried by what follows.
Extraction is deterministic and happens first: before any model call, Rust parses the document and produces a `FactSheet` — page count, word count, grade and age tokens matched against the project's own taxonomy, prices found by a currency-and-numeral scan, the archive's file inventory, and the subject the tenant declared on their own form — which is ordinary parsing code and is the oracle for everything the model later says, so the model never establishes a fact and only phrases facts the fact sheet already contains.
That is the parse-don't-validate membrane applied to a boundary where the untrusted party is a document.
Output is parsed into a type rather than read as text: the model returns a constrained JSON object deserialised via `#[serde(try_from = "RawListingCopy")]` into `ListingCopy`, whose smart constructor enforces length bounds from `crates/limits`, no URLs at all, no email addresses, no markup, no control characters, a permitted character set, and no content outside the declared fields; a failing output is an `Err`, retried once against the same input and then routed to `needs_human`, and never repaired by a second model call, because a repair prompt is another injection surface fed by the output of a compromised one.
Numeric claims are blocked unless they verify: every number in the generated copy — page count, activity count, grade range, price, "over 200 worksheets" — is extracted deterministically from the output and checked against the fact sheet, and a number absent from the fact sheet or contradicting it fails the listing closed.
This is the hardest rule here and it earns its place twice over, because a numeric claim is both the most common confabulation and the one with commercial consequence, since a wrong page count or price on a live storefront is a refund, a marketplace policy problem and a public review — and it is also why the fact sheet must be built in Rust rather than requested from the model, since if the model produces both the claim and the material used to check it the check is vacuous.
The fields the model may write and the fields it may not are different types: subject, grade range and price come from the tenant's own form and the taxonomy crosswalk, with no code path by which generated copy can alter them, so "the model changed the price" is not a bug that can be written.
Injection-marker detection is a signal and never a gate: scan extracted text for instruction-shaped imperatives addressed to a model, base64 blocks, zero-width and bidirectional control characters, and text rendered invisibly where the extractor exposes enough to tell, and let a hit raise the review priority and increment a metric without suppressing the other controls, because a filter that is trusted becomes the boundary and OWASP's own caveat says it cannot be one.
Whether the invisible-text cases are detectable at all depends on what the chosen PDF extraction crate exposes, which is unverified here and should be measured rather than assumed.
Least privilege is nearly free in this design and must stay that way: the model has no tools, no network access, no database access and no ability to trigger a publish, being a text-to-text function, which is what keeps a successful injection from being catastrophic — and the single change that would invalidate this entire section is giving the model the ability to call an adapter, so flag that explicitly as a decision someone makes rather than a capability someone adds.

Generated copy requires human approval before first publish, and the requirement is enforced by the type system rather than by a check someone might forget: `ListingCopy` cannot be converted into a publish payload and only `ApprovedListingCopy` can, whose sole constructor takes an `ApprovalRecord` carrying the approving user, the timestamp and the hash of the content approved.
Regeneration after approval changes the hash and therefore invalidates the approval, which is the detail usually missed and the one that makes the gate real rather than ceremonial.
After the first publish, requiring approval for every subsequent edit forever would destroy the product's value, so the rule is approval on first publish and on any change to a field the model wrote, with the content hash making "any change" mechanical rather than a judgement call.
Bulk approval across many listings is legitimate, since the seller is the actor and it is their storefront, provided the interface shows what is being approved; bulk approval that shows nothing is the failure mode, and that is a user-interface rule, which makes it one of the few controls in this charter that no lint can enforce, recorded as such rather than pretended otherwise.

Commit a fixture corpus of injection attempts covering the shapes OWASP's scenario list names — instruction text in the body, zero-width characters, payloads split across pages, multilingual and base64-encoded variants, and adversarial suffixes — and run it as a test whose assertion is that for every fixture the pipeline either produces copy that passes validation and contains no injected content, or fails closed.
The pass criterion is never that the model resisted; it is that the deterministic validator caught it, or that the content never reached publish, and stating it that way is what makes the test stable across model and provider changes, the model being the one component whose behaviour can change without any deploy on this project's side.
Selector discovery on drift reads marketplace HTML, which is also attacker-influenced content, and its output is a selector, which is closer to code than to prose, so the same discipline applies with a different oracle: a discovered selector is a proposal, validated by running it against a recorded page fixture and asserting it yields the expected element, then reviewed and committed as an ordinary adapter change, because a selector the model invented and the system applied at runtime is remote code execution against a seller's storefront with extra steps.
This is also where the sync engine's definition earns restating: sync is deterministic scheduled execution, and nothing an LLM produces takes effect anywhere in this system without first passing a deterministic gate written in Rust.

## 16. Deployment and rollback

The claim that this system is redeployable in minutes is load-bearing for the kill switch, for the canary, for the debt deliberately accepted in the adapters, and for the whole shape of the rigour budget, yet an earlier draft specified how the artefact is built and said nothing about how it gets onto the machine or how it comes off again.
Build the whole system closure from the flake, which is the same closure the flake checks already evaluated, so the artefact deployed is the artefact tested — that property is the reason Nix is worth its cost on this project and is worth stating as the reason rather than assumed as background.
The command is `nixos-rebuild switch --flake .#<host> --target-host <user>@<host> --sudo`, all flags verified against nixos-rebuild on the `nixos-26.05` branch, where a further detail matters: `nixos-rebuild` is now `nixos-rebuild-ng`, the Python implementation, and the module carries the message that the Bash implementation has been removed, with actions `switch`, `boot`, `test`, `build`, `dry-build`, `dry-run`, `dry-activate`, `build-image`, `build-vm`, `build-vm-with-bootloader`, `list-generations`, `edit` and `repl`, and relevant flags including `--flake`, `--target-host`, `--build-host`, `--rollback`, `--specialisation`, `--profile-name` and `--diff`.
The build happens on the founder's machine or in CI rather than on the target: `--build-host` exists, but building on the production box competes for memory the service needs, and a deploy that causes an out-of-memory kill is a poor deploy.
Two inspection steps belong in the deploy task rather than in a habit — `--diff`, which prints `nix store diff-closures` between the running system and the new one and is the cheapest honest answer to "what am I actually changing", mattering more than usual when an agent wrote the change; and `dry-activate`, which shows which units would restart without activating anything.

Activation creates a generation on the system profile, `nixos-rebuild list-generations` enumerates them, and `nixos-rebuild --rollback switch` activates the previous one, with the closure already on the machine so a rollback costs a unit restart and nothing else, which is the strongest part of the minutes claim.
State the limit of that plainly, because conflating the two halves is the mistake this section exists to prevent: a generation rollback reverts code and configuration and does not revert the database, the object store, the contents of the job ledger, or anything an already-applied migration did, so every deploy carries two independent questions — can the code be rolled back, and can the data — where the answer to the first is almost always yes and to the second usually no.
Migrations run as an explicit step, never from the application's startup path, because an application that migrates on boot means a rollback restarts the old binary against the new schema with no gate and means two processes can race the same migration; run them from a oneshot unit ordered before the API unit, or from the deploy task, but one runner, once.
The expand-and-contract sequence is what makes the rollback complete, since expand-phase migrations are backward-compatible by construction so the previous generation runs unchanged against the migrated schema, making `nixos-rebuild --rollback switch` a full rollback for every deploy except the contract one.
The contract deploy is handled by procedure worth automating into a gate: it ships alone, after the new code has soaked for the stated period, and the deploy task refuses to proceed when the pending migration set contains a destructive statement whose marker comment's soak period has not elapsed, which converts the one rule in this charter that depends on someone remembering into a check.
Its rollback plan is not a generation switch: if only the code is wrong, fix forward, and if data was destroyed, restore by point-in-time recovery into a scratch database and copy the lost rows forward, never over production.
Three supporting details: the deploy step takes a fresh incremental database backup immediately before running migrations rather than relying on the schedule, because the interval between the last scheduled backup and a destructive migration is exactly the interval you will wish were zero; `sqlx migrate info` runs before and after with its output in the deploy record, so the deployed schema version is a fact rather than an assumption; and the readiness endpoint compares the applied migration set against the set the binary embeds, so a mismatch fails readiness rather than serving requests against a schema the code does not expect.

sops-nix decrypts into `/run/secrets` at activation and `restartUnits` restarts the consumers of a changed secret as part of the same `nixos-rebuild switch`, so rotation is a deploy and costs the same minutes, which removes the usual excuse.
Because workers lease work, an activation that restarts one mid-lease must not orphan jobs: the lease expiry means orphans self-heal, and the graceful path for a deploy that changes the adapter contract is to set the affected marketplaces to `Draining` first, reusing the kill switch rather than inventing a second mechanism for the same need.
A restart also tears down every browser session, so a deploy landing during a submit is exactly the ambiguous-write case the rules file's fault taxonomy exists for, which is a second and stronger reason `Draining` is the first step rather than a courtesy.
Zero downtime is not a goal — one box, one API process, a restart measured in seconds, and clients that poll for progress and already tolerate an unreliable network — so the API is briefly unavailable on every deploy, and saying so is more useful than pretending otherwise, since the effort a blue-green arrangement would cost is better spent on the restore drill, which protects against a failure mode that actually ends the business.
The deploy task records wall-clock time for build, copy, activate and rollback, and those numbers go where the restore-drill records go, because a "redeployable in minutes" premise nobody has timed is precisely the sort of unchecked assumption this charter exists to convert into evidence, and several decisions above depend on it being true.
Rehearse the rollback as well as the deploy, for the same reason a backup nobody has restored is not a backup.

## 17. Data protection as an engineering constraint

This shapes the architecture more than any lint in this document, and while the legal analysis belongs elsewhere, what follows is what that analysis forces the code to look like, stated here because these consequences are cheap at design time and extremely expensive to retrofit.
UK and EU obligations both apply to a platform serving teacher-authors in both jurisdictions, and four categories of data determine everything downstream: tenant account data, where the seller is the data subject; tenant content, mostly not personal data but the seller's intellectual property and livelihood, and occasionally containing personal data such as pupil names in a worksheet built from a real class list; marketplace credentials and the session material minted from them, which this system holds under envelope encryption and which is the highest-value personal data in the product; and billing data, which lives at Stripe and never here, because the hosted payment surfaces mean this system holds a customer identifier and never a card number.
The third is worth pausing on, because an earlier draft recorded it as the architecture's largest data-protection dividend and it is now its largest data-protection liability.
Holding a seller's marketplace credential makes this system a target in a way that holding their listing text does not, since a breach there is account takeover on the storefront that is their income rather than disclosure of documents they also hold themselves.
That is the strongest engineering argument for the credential seam in section 9, and it belongs here as well as there: the seam is a data-protection control rather than merely an abstraction, and the case for funding its better implementations is made in this section's currency.

Deletion is an operation rather than a flag, so an erasure request means the data goes, which makes backup retention a data-protection parameter rather than a storage-cost one, since an unbounded backup archive makes erasure impossible to honour honestly: retention is bounded, stated, disclosed and single-sourced like every other number, erasure executes immediately in the live system, and residual copies in backups expire within the stated retention window.
Deletion must be complete across every store, which makes "where may data live" an architectural constraint rather than an implementation choice — today Postgres, the object store, the logs, the failure captures of section 12, and whatever the LLM provider retains — so every new store is a new place erasure has to reach and adding one carries a deletion cost paid in the same change or not at all.
Make that mechanical with a `DataStore` enum under the `wildcard_enum_match_arm` deny and an erasure routine that matches exhaustively over it, so adding a store fails to compile until its erasure path exists, which is the technique used elsewhere in this charter for adding a marketplace or a fault variant, applied to the obligation hardest to remember and most damaging to forget.
Export must be self-service and machine-readable, covering both the access and the portability duties, and is the same implementation as the wind-down control in section 10 — one feature, three obligations — exporting the tenant's catalogue as structured data and their original uploaded files unmodified, because the files are theirs and re-encoding them loses fidelity they may need.
Logs are a store, and a log line containing tenant content is personal data sitting in a place with a different retention period and no erasure path, which is why the telemetry payload type admits no free text and why the field vocabulary admits only identifiers and enums: section 12 and this section are one control described twice.
A failure capture is a store too, and a harder one, since a raw marketplace page can hold a seller's account details and a buyer's name at once; that is why it is encrypted under the tenant's data key, expires on its own, and appears in the `DataStore` enum below rather than living outside the erasure path because it is diagnostic.
Breach notification runs on a 72-hour clock — the regulation requires notification without undue delay and, where feasible, not later than 72 hours after becoming aware — and that is an engineering requirement because you cannot notify about what you cannot determine, since answering which tenants' data was in which store at which time within three days requires the correlation identifiers and the audit trail to already exist, and they cannot be added after the incident that needs them.
Residency and transfers follow from where the box is and where the model runs, and neither is a decision to make late.

Whether extracted document text may be sent to a third-party model is a contractual question, and if the answer is no, or conditional, the AI listing-copy service does not exist in its present form, so it is answered before the feature is built.
For the feature to be buildable the arrangement has to say certain things — that the provider processes only on documented instructions, that sub-processors are disclosed and may be objected to, that content is not used for training, that retention is zero or short and stated, and that a transfer mechanism covers wherever inference actually runs — which are contract terms rather than code, though each has an engineering consequence if absent.
Three hedges keep the answer changeable without a rewrite: the provider client sits behind one trait in one crate with no other dependents, the same seam the spend guard already relies on, so replacing a provider is a crate-sized change rather than a search-and-replace; prompt construction is a separate step from the provider call, so what is sent is a reviewable value rather than a string assembled inside a client library; and every call records identifiers and sizes rather than content, so "what did we send them" is answerable without keeping a second copy of the thing you are trying not to duplicate.
A per-tenant opt-out must genuinely disable the feature for that tenant rather than degrading it silently, because a tenant who declines third-party processing is entitled to a working product without it, which is a product requirement falling directly out of a legal one.
The control that does the most work is minimisation: send the fact sheet and the tenant's own declared fields rather than the whole extracted document wherever the copy task permits it, because less content leaving means less to contract about, less to disclose in a breach, less to delete on request — and, usefully, a smaller prompt-injection surface, since the fact sheet is deterministic Rust output rather than attacker-supplied prose.
The privacy control and the security control converge on the same design, which is the strongest available argument for building the fact sheet before building the prompt.

A mistake in the lint table costs an afternoon; a mistake in the architecture here — tenant content in a third-party model with no contract behind it, a store nobody wired erasure into, logs holding tenant text under an unbounded retention — costs a rewrite of a layer, and possibly the product.
So these questions get answered at design time and the answers get written down, and that record is the artefact for this section in the same way `[workspace.lints]` is the artefact for the toolchain file, making this the one part of the charter where the correct amount of ceremony is more than the compiler can supply.

## 18. What this costs, and what to defer

An earlier draft had a spike and a production build and nothing between them, which put every day-one decision on the far side of a boundary the project may not reach; there are three stages, and the middle one is where this product actually lives for its first year.

The spike is throwaway code whose purpose is to answer questions: does automation drive the upload form at all through WebDriver against a real Chromium, what does that form do on partial or rejected submission, how long does a real upload take, what does one browser session cost in memory and CPU, and what do the marketplace terms permit.
Almost none of this charter applies to it, and applying it would be a mistake rather than merely wasteful; four things apply from the first line because each costs seconds — `#![forbid(unsafe_code)]`, rustfmt at its defaults, explicit timeouts on every HTTP client and every navigation, and a wrapper around every WebDriver BiDi call, since that surface carries no deadline of its own and a wedged spike teaches nothing.
One thing applies that is not about code quality at all: record everything, because the spike's real output is not a working upload but the fault taxonomy, and that taxonomy is the input to the seam, the limits and the simulator.
Explicitly not applied: the lints table, `--deny warnings`, `cargo-deny`, the limits module, the assertion discipline, test-driven development, the seam, and every flake check beyond formatting.

The first paying deployment is the new stage, and its defining property is that it has customers and can still be rewritten, so the only rigour that earns its place is rigour that is expensive to retrofit or that protects money, tenants, or the law.

- The database schema, and `TenantId` as a mandatory positional parameter on every repository method and cache key, plus the two-tenant cross-visibility integration test.
- The `AdapterError` taxonomy including `Ambiguous` as a distinct variant, and `IdempotencyKey` on every publish.
- The sans-IO seam: `SyncMachine::step` as a synchronous total function, and the `MarketplaceAdapter` trait.
- The release profile carrying `overflow-checks = true` and `debug-assertions = true`.
- Explicit timeouts on every external call, bounded channels only, and `crates/limits` as shipped in section 6 of the rules file.
- `CatchPanicLayer` with its alerting counter, `spawn_supervised`, and crash-at-boot for configuration.
- The `Secret<P>` type with its redacting `Debug`, and `cargo-deny` licence allow-listing, which is a legal requirement and gets harder the more dependencies exist.
- Envelope encryption of marketplace credentials with a per-tenant data key, the broker as the sole holder of the key-encryption key, and the credential-acquisition seam, which is cheap now and a rewrite of a layer later.
- The pre-spawned browser pool at `limits::browser::SESSIONS_MAX`, one systemd unit per session with its own dynamic user and state directory, and the durable per-tenant mutex, because unpicking a shared profile after two tenants have used it is not a refactor.
- Database backups with a restore that has actually been run once, and the per-marketplace kill switch, which is one Postgres row and one clause in the lease query.
- The prompt-injection controls of section 15, because the fact sheet must exist before the prompt does and the approval gate must be a type before there is copy to approve.

Everything else in the earlier draft's day-one list is a Tuesday afternoon at any point in the next year and should be deferred explicitly rather than quietly: `pedantic` at deny, the full flake check set, `cargo-machete`, `cargoDoc` with `RUSTDOCFLAGS=--deny warnings`, `missing_docs`, OpenAPI plus `oasdiff`, `cargo-hack` matrices, mutation testing, sandboxed ingestion, Kani, and the simulator.
None of it gets harder by waiting, and the production build is everything above plus the deferred list, starting when the product has proven it deserves to exist.

| Deferred | Trigger |
|---|---|
| The deterministic simulator | The first ambiguous-write incident, or the first bug taking more than a day to reproduce |
| Kani proof harnesses | The crosswalk stabilising, and its domain proving too large for an exhaustive test |
| shuttle | A real concurrency bug in the job ledger that survives designing the race away |
| turmoil | The API server and workers becoming separate networked processes |
| Revisiting the panic policy toward TigerBeetle's | A second machine, a standby, or managed Postgres |
| An OpenTelemetry collector | A second process, or a question the journal cannot answer |

The one-time setup is small: the lints table, `clippy.toml`, `deny.toml`, the release profile and the fast flake checks are perhaps one to two days, most of it copying the blocks in the toolchain file and then fixing whatever the first `--deny warnings` run surfaces, and `crates/limits` at ten constants is an hour rather than the half-day the sixty-constant version needed, which is most of the argument for ten.
The sans-IO seam is a few days at the start of the sync engine and several weeks if retrofitted later, which is the entire argument for cutting it early.
The ongoing costs are where a charter is abandoned, so they should be named: denying `unwrap_used` and `expect_used` is the largest recurring tax, and the `allow-*-in-tests` keys remove most of it, without which this pair is the single most likely lint to be switched off in month two; denying `pedantic` means a toolchain bump can break the build for reasons unrelated to the code, so pin the toolchain in the flake, and with `nursery` and `cargo` moved to the advisory lane that risk is bounded to one group rather than three; and `cargo-mutants` costs mutant count multiplied by full-suite duration, which is hours, and is why it is scheduled and `--in-diff`-scoped.
One cost is not measured in hours at all, and it is the founder-gated shared state recorded in the toolchain file: the lint files, `crates/limits` and every dependency addition are where the charter actually lives, so a quiet relaxation there silently invalidates everything above it.

### Questions that change this charter

The marketplace-terms question has moved to section 1 of the rules file, where it is a gate rather than a question; six remain, ordered by how much they change the document.

Where is the trust boundary for LLM-generated listing copy?
Section 15 answers it with an approval gate enforced by `ApprovedListingCopy`, and that answer should be confirmed rather than assumed, because if rewritten copy may reach a marketplace without human review then every property of it becomes untrusted input requiring an `Err` path, which is a large amount of code — and the question is sharpened rather than softened by the sync engine being deterministic, since the model's output is the only untrusted value in the whole pipeline that a human might not see.

May extracted document text be sent to a third-party model at all?
Section 17 makes this contractual and prior to the feature, and a negative or conditional answer changes the shape of the listing-copy service rather than one of its parameters.

Which entity operates the service, and under which jurisdiction?
The decision register records this as open, and it forks the privacy regime, the consumer-law regime, the insurance market and the customer terms at once, which makes it the one open question that changes section 17 wholesale rather than changing a parameter inside it.

What does the marketplace actually do on an ambiguous submit — is there an idempotency handle, a client-supplied reference, or a probe that distinguishes created from not-created?
The `Ambiguous` recovery path is only well-defined if such a probe exists, and if it does not then reconciliation becomes probabilistic deduplication by content hash and the state machine changes rather than just the adapter, which is the strongest argument for the spike's recording obligation.

Is there any redundancy planned, and how far does inter-tenant concurrency go?
Redundancy shifts the per-tier panic policy toward crashing and the rate limiter's in-process state toward something shared; the browser pool already makes concurrent publishing for two tenants the ordinary case, so worker identity belongs in the state machine from the start and "two workers publish the same listing" is the highest-value simulation target, while the open half is whether concurrency within one tenant is ever permitted, which the durable per-tenant mutex currently forbids.

How long does a marketplace session survive before it needs re-authentication?
Everything in the parked-work design depends on the answer: a session measured in weeks makes unattended overnight sync ordinary, while one measured in hours makes each seller's availability window the binding constraint on throughput, which changes the scheduling model rather than one of its parameters.
