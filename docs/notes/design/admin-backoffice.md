# Admin backoffice

An operator's view over the whole platform: signups, organisations, products, sync health across Tes and TPT, and operational settings.

This is a design note, not a decision.
Nothing below is built, and the investigation behind it was read-only against this tree; no external service or marketplace was contacted.
Every schema change, grant and endpoint named here is a proposal for founder decision, and the grant in particular is founder-gated by `CLAUDE.md:74-80`.
Paths are this repository, cited with line numbers.

## Verdict

Nothing app-side can read across tenants today, and that is the fence working as designed rather than a gap.
The smallest honest change is one new marking and one new database role: an app-side operator row checked in Rust, and a SELECT-only `tam_backoffice` role whose grants are enumerated table by table in a single migration.
The identity plane's existing admin powers stay where they are and are never consulted for app-data access.
Everything in v1 is a read, and the credential vault stays unreachable by construction rather than by convention.

## 1. What "admin" means here

Two planes exist and the proposal keeps them apart.

The identity plane already has an admin surface.
`auth/src/auth.ts:185` mounts better-auth's `admin()` with no options, so the plugin's defaults apply: `defaultRole` is `"user"` and `adminRoles` is `["admin"]` (`packages/better-auth/src/plugins/admin/admin.ts:44-45` in the 1.7.2 clone at `~/ghq/github.com/better-auth/better-auth`, which matches the pin at `auth/package.json:17`).
The marking it reads is the `role` column on the identity user table, beside `banned`, `banReason` and `banExpires` (`db/auth/0001_identity.sql:32`).
That surface lists users, bans and unbans them, sets roles and revokes logins, and it sees no domain data at all, because `tam_auth` holds no privilege on any table in `public` (`db/init/02-auth-role.sql:36-43`).

The app plane has no operator concept.
`app_user` carries id, org_id, email and created_at (`crates/tam-storage/migrations/0014_sessions_and_operations.sql:8-17`) plus the better-auth subject link (`0035_auth_subject.sql:17-19`), and nothing else.
`OrgContext` carries an organisation and a user and no capability (`crates/tam-api/src/session.rs:22-25`), and it is resolved from the session row rather than from any claim (`session.rs:50-67`, `:218-224`).

The proposal is a dedicated `platform_operator` table keyed by `app_user.id`, carrying `granted_at`, `granted_by` and a nullable `revoked_at`, read in Rust on every operator request.
A column on `app_user` was the alternative and is not recommended: `app_user.org_id` is `NOT NULL` (`0014:10`), so a column there reads as an org-scoped attribute of a tenant's user, whereas being an operator is a platform fact about a human.
A separate table also records grant and revocation without widening a tenant-facing table, and it forces the tenancy question to be answered explicitly, because `crates/tam-storage/tests/rls_matrix.rs:82-86` fails any public table absent from both classification lists.
It is classified global, beside `app_user` and `user_session` and for the same reason (`rls_matrix.rs:46-63`).

The check is a distinct extractor, `OperatorContext`, resolved through the same session path with one extra query.
A flag added to `OrgContext` is refused: every existing handler takes that extractor (`crates/tam-api/src/lib.rs:104-167`), so a field there would be a capability travelling into thirty routes that have no business carrying one.
Nothing about the operator marking is ever asserted by a token, which is the standing rule at `docs/design/decisions.md:31`.

The first operator is bootstrapped by a one-shot on infrastructure we run.
`tam-mint-session` is the precedent — an operator command that connects with database credentials and prints its result once (`crates/tam-mint-session/src/main.rs:1-6`) — and the specification already names `tam-admin` as the operator command-line tool, the one binary with no systemd unit (`docs/design/2026-08-25-listing-sync-design.md:705`, `:707`).
No HTTP path creates an operator, so there is no self-elevation endpoint to attack.
A seed migration is refused because a migration cannot know which uuid is the founder's.

The two markings compose by staying unsynchronised.
The identity plane decides who may ban a login; the app plane decides who may read cross-tenant domain data; neither is consulted for the other's question.
The cost is that a person is elevated twice, deliberately: a compromise or a misconfiguration of the identity service then cannot grant cross-tenant reads of app data, which is the property `decisions.md:31` exists to hold.

## 2. The cross-tenant read mechanism

The fence today is per-table row-level security keyed on `app.current_org`, enabled and forced onto the owner (`crates/tam-storage/migrations/0001_product.sql:32-44`), null-safe against a pooled connection's reset setting (`0009_null_safe_policies.sql:1-24`), covering the thirty tables listed at `rls_matrix.rs:13-44`, and pinned transaction-locally by `crates/tam-storage/src/lib.rs:98-110`.
Nine tables are global and carry no policy (`rls_matrix.rs:53-63`), so `tam_app` can already read `organisation`, `app_user`, `user_session`, `marketplace_inventory`, `canonical_term`, `inventory_halt`, `projection_edge` and `projection_no_counterpart` across tenants; the API simply never does, because every handler is keyed on `OrgContext` (`crates/tam-api/src/org.rs:65-73` is the pattern).

Four mechanisms were weighed.

Reusing `tam_engine`'s BYPASSRLS pool is the worst of them, and `crates/tam-server/src/main.rs:219-227` already builds such a pool when `--engine-db-url` is passed, which makes it the tempting one.
That role holds INSERT and UPDATE on `job`, `job_item`, `job_event`, `org_event_counter`, `write_attempt`, `outbox_message` and `rate_budget` (`0007_engine_role_grants.sql:7-8`), INSERT on the halt tables and `field_audit` (`:9-10`), and DELETE on `job_event` (`0015_prune_grants.sql:5`).
An operator route on that pool can write every tenant's ledger, and BYPASSRLS (`db/init/01-app-role.sql:18`) means one wrong predicate reads everything.
It also dissolves the engine's stated single narrow crossing (`01-app-role.sql:11-16`).

`SECURITY DEFINER` functions buy the same reach with a worse audit story: the privilege lives inside function bodies rather than in a grant list a reviewer can read end to end, and the definer would have to be a role that already holds the reads, which returns to `tam_engine` or a superuser.
The one thing they buy — narrowing to particular columns and row shapes — a view plus a grant buys as well.

Iterating organisations under `tam_app`'s existing fence needs no new privilege at all and is genuinely the right answer for part of the surface.
It cannot serve the rest: product, mapping, job and write-attempt aggregates read fenced tables, so a page would need one pinned transaction per organisation, a per-tenant round trip that grows with the customer count.

The recommendation is a new SELECT-only role, `tam_backoffice`, with explicit per-table grants and its own pool, used exclusively by operator routes — and the global tables read under `tam_app` as they already can be, with no grant at all.
The whole widening is then one enumerable list in one migration, in the same form `0007` and `0010` already use, and the separate pool means no existing handler can reach it by accident.

Blast radius, stated as the first-class criterion.
A compromised admin session newly reaches SELECT on the granted tenant tables — `product`, `mapping`, `connection` metadata, `job`, `job_item`, `write_attempt`, `org_halt`, `org_inventory_halt` — across every tenant, plus the global tables the API can already read.
It does not reach `connection_secret`: the new role gets no grant on it, `tam_app`'s own access was revoked (`0010_broker_custody.sql:7`), and `crates/tam-storage/tests/custody.rs:23-40` asserts that the app role is denied and the broker is not.
It does not reach the key-encryption key, which exists only inside `tam-session-broker` (`db/init/01-app-role.sql:20-24`), and there is no path from an operator route to the broker socket.
It does not reach the `auth` schema beyond `auth_event`, the only object there `tam_app` holds SELECT on (`db/auth/0002_audit_event.sql:76-81`).
It performs no write anywhere, because the role is granted SELECT and nothing else.

One caveat belongs in the record rather than in a footnote.
`tam_app` owns every table in `public`, because migrations run as `tam_app` (`justfile:108-109`) against a database it owns (`db/init/01-app-role.sql:31-32`).
An attacker able to execute arbitrary SQL as `tam_app` can therefore re-grant itself the vault or drop FORCE from a policy; the revoke and the fence hold against a wrong query, not against arbitrary SQL execution.
That is true today and this design does not change it, and it is precisely why the recommendation adds a role rather than relaxing `tam_app`.

Two things would reverse the recommendation.
If the operator pages turn out to need cross-tenant writes — an operator raising an `org_halt`, say — the grant list stops being SELECT-only and the property that justified it is gone; the honest form at that point is a separate operator service with its own role and its own audit trail, not a widened grant inside the API process.
If the organisation count stays small enough that per-organisation iteration under `tam_app` is cheap for every page, the fourth option needs no new role and should win, because the smallest change that works is the right one.

## 3. The v1 surface

Every route sits under one prefix and takes `OperatorContext`.
All but one are reads; the exception is the plan grant, described at the end of this section.

`GET /{version}/admin/signups` returns counts over time from two sources: identity signups from `auth.auth_event` where `event = 'user_signed_up'` (`db/auth/0002_audit_event.sql:39`, index at `:69`), and app-side provisioning from `app_user.created_at` (`0014:8-12`), which `session.rs:238-253` writes on a subject's first login.
It needs one new repo read and no new grant, because `tam_app` already holds SELECT on `auth_event` (`0002_audit_event.sql:81`); nothing in Rust reads that table today.

`GET /{version}/admin/orgs` lists organisations (`0001_product.sql:4-10`) with per-organisation counts of products, mappings, connections and users.
`organisation` and `app_user` are global and need no privilege (`rls_matrix.rs:53-63`); `product`, `mapping` and `connection` are fenced and are what the new role is for.

`GET /{version}/admin/orgs/{org}` renders one organisation: its connections in the shape `ConnectionView` already uses, stored `state` beside the derived `status` (`crates/tam-api/src/resources.rs:282-294`), its halts, its counts, the plan it currently holds, and every grant it has ever held.
The plan is a derivation rather than a column: it is the strongest unexpired row of `entitlement_grant` (migration 0069), which is also what the request path reads.

`GET /{version}/admin/sync-health` aggregates `job` and `job_item` across tenants by state, over the vocabulary the lease and settle constraints already fix (`0005_job_ledger.sql:49-56`), with settled outcomes beside it.

`GET /{version}/admin/failed-writes` lists `write_attempt` rows carrying a `failure_code`, newest first (`0005_job_ledger.sql:82-99`), with the owning item's `failure_code` and `failure_detail` (`:29-30`).
The vocabulary is the shared `FailureCode`, which the worker, the API, the client and the operator dashboard use verbatim (`docs/design/2026-08-25-listing-sync-design.md:571`).

Operational settings need no new endpoint in v1.
`GET /{version}/status` already answers the inventory halts unpinned and cross-tenant, because `inventory_halt` is global (`crates/tam-api/src/resources.rs:695-715`), and the operator page calls it unchanged.

### The one write, and why it does not use the backoffice pool

`POST /{version}/admin/orgs/{org}/plan` grants a plan or a rung, with a reason, an optional expiry and an audit row; `POST /{version}/admin/orgs/{org}/plan/{grant}/revoke` withdraws one, keeping the row.
Both answer the same org-detail object the read above answers, so the console's plan panel renders from one shape rather than two.

This is the first operator route that writes app data, and it deliberately does not write through `tam_backoffice`.
That role holds SELECT and nothing else, and the reason every other handler on this surface is safe is precisely that the connection it holds cannot write across the tenant fence.
Granting it INSERT on one table to shorten one handler would give that property away for all of them: the next operator route added would inherit a writable cross-tenant connection nobody chose to give it.
So the grant opens the application pool — `tam_app`, the same role a seller's own write uses — and pins the target organisation from the path, exactly as a tenant write pins it from the session.
`OperatorContext` names no organisation by design, so this is the one place the pin comes from somewhere other than the session, and it is written out rather than hidden behind a helper.

The visible cost is a read through one pool before a write through the other: the organisation is read with the backoffice pool first, so a grant naming an organisation that does not exist answers 404 rather than conjuring a tenant.
A grant with a blank reason is refused, because an audit row whose reason is empty answers none of the questions an audit row exists for, and a `migration_only` grant naming no rung is refused, because the rung *is* the allowance and a grant without one would grant nothing.

Every other mutation in v1 is still the identity plane's own.
Bans, unbans and role changes go to better-auth's admin endpoints through the identity service (`auth/src/auth.ts:185`); the Rust API neither proxies nor re-implements them.
Raising a halt stays what it is today, an operator act outside the API (`docs/design/schema.md:330-333`), because wiring it to a button is a cross-tenant write and therefore a separate decision.

## 4. Deliberately not in v1

Impersonation, which better-auth's admin plugin ships and this design does not expose, because it would turn an identity-plane elevation into an app-plane session and collapse the separation section 1 exists to hold.
Cross-tenant writes of every kind except the plan grant above, including halts, job cancellation, reconciliation resolution and election answers.
Any credential-adjacent surface: no column of `connection_secret` reaches a view, not the ciphertext, not the nonce, not the wrapped DEK, not the key version (`0006_halts_connection_audit.sql:86-98`).
Anything reading the vault, and any path from an operator route to `tam-session-broker`.
Any `auth`-schema read beyond `auth_event`, so no email, no password hash and no `jwks`, holding the one-directional grant at `db/auth/0002_audit_event.sql:76-81`.
Free-text SQL, an export-everything action, and per-seller listing bodies.

## 5. Where the UI lives

Recommendation: the same SvelteKit application, under `/admin`, gated by data access rather than by bundle.

The reasoning is what `adapter-static` actually produces.
The client is a static single-page app with `ssr` and `prerender` both false (`web/svelte.config.js:8-10`, `web/src/routes/+layout.ts:3-4`), served by `ServeDir` with `index.html` returned for any path the API and the asset tree do not claim (`crates/tam-server/src/main.rs:229-251`).
Its route guard is a client-side redirect over a `whoami` result (`web/src/routes/+layout.svelte:18-23`), which hides a page and protects nothing.
A static bundle's routes are public artifacts either way, so the only protection in both designs is the API refusing the request, and every operator page is a call to `/{version}/admin/*` that a non-operator is refused.
A separate build therefore buys exactly one thing — operator route names and markup absent from the seller's bundle — at the cost of a second static tree, a second serving path and a second session story.
If that disclosure is judged unacceptable, the cheap middle answer is a second SvelteKit build sharing `$lib` and served from a distinct `--ui-dir` on its own bind address, which is a hosting change rather than an architecture change.

## 6. Founder questions

1. Does a new SELECT-only `tam_backoffice` role land, with its grant list reviewed as part of the change? Recommended: yes, SELECT only, enumerated table by table in one migration in the form `0007` and `0010` use, and never granted anything on `connection_secret`.
2. Operator marking as a dedicated `platform_operator` table, or a column on `app_user`? Recommended: the table, classified global in `rls_matrix.rs`'s closed-world list.
3. Bootstrap through a `tam-admin` one-shot on the box, or a seeded migration? Recommended: the one-shot, matching `tam-mint-session` and the binary the specification already names; no HTTP path can create the first operator.
4. Do the identity-plane `admin()` powers stay separate from the app-plane operator marking, with no synchronisation between them? Recommended: yes, two markings, and neither consulted for the other's question.
5. Same bundle under `/admin`, or a second static build? Recommended: same bundle, because the protection is identical and the split buys only route-name secrecy.
6. Is impersonation permanently out, or deferred? Recommended: deferred behind a stated bar — it does not ship until every impersonated session is recorded append-only and readable by someone other than the impersonator.
