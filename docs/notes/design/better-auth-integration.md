# better-auth integration

How better-auth supplies platform-user identity and browser sessions for the SvelteKit dashboard while Rust and Postgres remain authoritative for tenancy, row-level security, and marketplace-credential custody.

This is a design note, not a decision.
It amends a non-negotiable recorded in `docs/design/decisions.md:26-27`, so the amendment in "The TypeScript boundary, amended" needs founder ratification before any of it is built.

Sources are cited by path.
better-auth was read from a local clone at `~/ghq/github.com/better-auth/better-auth`, version 1.7.2 (`packages/better-auth/package.json`), at commit `3660f062d6e7a9b02e3fc8eee5b99482117ebba0` dated 2026-08-30.
Paths beginning `packages/` or `docs/content/` below are that clone; every other path is this repository, cited with line numbers.

## What exists today

Authentication is one axum extractor over one Postgres table.
A `tam_session` cookie carries 32 random bytes hex-encoded; the extractor parses the cookie, hashes the token, and resolves it to an organisation and a user, refusing the request with a structured 401 before any handler runs (`crates/tam-api/src/session.rs:18`, `:22-25`, `:30-39`, `:50-75`).
The store holds the token's BLAKE3 digest rather than the token, so a database read yields a verifier and never the capability (`crates/tam-storage/src/sessions.rs:1-6`, `:151-169`).
Logout deletes the row outright, because a bearer token has nothing to tombstone (`crates/tam-api/src/session.rs:177-197`).

The tables behind that are `app_user` and `user_session`, added in migration 0014 (`crates/tam-storage/migrations/0014_sessions_and_operations.sql:8-29`).
`app_user.org_id` is `uuid NOT NULL` referencing `organisation`, so a user belongs to exactly one organisation today (`:10`).
`organisation` itself is three columns and the tenant root (`crates/tam-storage/migrations/0001_product.sql:4-10`).

Both `app_user` and `user_session` are deliberately global rather than tenant tables: a session row must be readable before any tenant pin exists (`crates/tam-storage/migrations/0014_sessions_and_operations.sql:1-6`, `crates/tam-storage/tests/rls_matrix.rs:44-50`).
Every other tenant table is fenced.
`pin_org` sets `app.current_org` as a transaction-local setting (`crates/tam-storage/src/lib.rs:97-106`), and twenty-nine tables carry enabled and forced row-level security with a policy keyed on that setting (`crates/tam-storage/tests/rls_matrix.rs:13-41`, `:95-113`).
That test also asserts a closed world: every table in the `public` schema must appear in exactly one classification list, so an unclassified table is a test failure rather than a review oversight (`crates/tam-storage/tests/rls_matrix.rs:1-5`, `:75-84`).

Three database roles carry the privilege boundary (`db/init/01-app-role.sql:6-51`).
`tam_app` is the API path and is deliberately not a superuser, because superusers bypass row-level security (`:1-9`).
`tam_engine` has `BYPASSRLS` for the cross-tenant lease scan (`:11-19`).
Amended 2026-09-04: `tam_broker` read `connection_secret` inside `tam-session-broker`, and D1 retired both.
Migration 0051 revokes every privilege the role held, so no role may read `connection_secret` at all, and `db/init/01-app-role.sql:20-49` keeps the name inert — `NOLOGIN`, no password, no `BYPASSRLS` — only because migrations 0010, 0017 and 0032 grant to it and a frozen grant cannot name a role that does not exist (`crates/tam-storage/migrations/0051_retire_tam_broker_grants.sql`).
Production drops the role by the operator step in `docs/notes/runbooks/retire-tam-broker-role.md`.

Login today is an operator one-shot.
`tam-mint-session` finds or creates a user for an email under an organisation, mints a session, and prints the cookie token exactly once (`crates/tam-mint-session/src/main.rs:1-6`); `just dev-session` wraps it (`justfile:156-158`).
The dashboard's sign-in page is a textarea for pasting that line, exchanged for an `HttpOnly` cookie (`web/src/routes/login/+page.svelte:9-26`, `crates/tam-api/src/session.rs:136-173`).
Self-serve signup is scheduled as part of M5 alongside billing and tiers (`docs/design/milestones.md:142`).

The client is Svelte 5, SvelteKit 2, Vite 7 and Tailwind 4 (`web/package.json:12-24`).
It builds with `adapter-static` and an `index.html` fallback (`web/svelte.config.js:1-13`), with server rendering and prerendering both off (`web/src/routes/+layout.ts:3-4`).
`tam-server` serves the built directory through `ServeDir` with an explicit single-page fallback (`crates/tam-server/src/main.rs:41-44`, `:163-184`), and in development Vite proxies `/v1` and `/healthz` to the API so the cookie and the `EventSource` behave as they will in production (`web/vite.config.ts:9-16`).
There is no SvelteKit server process anywhere in the shipped deployment.

## What better-auth actually is

better-auth's core is a Web-standard fetch handler: `betterAuth()` returns an object whose `handler` takes a `Request` and returns a `Response` (`packages/better-auth/src/auth/base.ts:35`), mounted by default under `/api/auth` (`packages/better-auth/src/context/create-context.ts:196`).
Framework integrations are thin adapters over that handler rather than separate implementations, which is why the same instance runs under SvelteKit, Node, or anything else that speaks `Request`.

Persistence goes through a database-adapter interface with first-party adapters for Kysely, Drizzle, Prisma, MongoDB and memory (`packages/better-auth/src/adapters/`).
The Kysely adapter issues unqualified table names — `.selectFrom(model)` and the equivalent insert, update and delete builders (`packages/kysely-adapter/src/kysely-adapter.ts:195`, `:563`, `:734`).
No `withSchema` call appears anywhere in the packages tree, so the connection's `search_path` alone determines which Postgres schema better-auth's tables land in.
That single fact is what makes schema isolation available for free below.

The core schema is four tables, assembled in `packages/core/src/db/get-tables.ts`: `verification` (`:90`), `session` (`:131`), `user` (`:198`) and `account` (`:251`), plus an optional `rateLimit` when rate-limit storage is set to the database (`:61`).
`session` carries `expiresAt`, a unique `token`, timestamps, `ipAddress`, `userAgent`, and `userId` referencing `user` (`:135-176`).
`user` carries `name`, `email`, `emailVerified`, `image` and timestamps (`:202-240`).
`account` carries the provider linkage and the credential material for social and password login, including `password`, `accessToken`, `refreshToken` and `idToken` (`:263-346`).
Passkeys are a separate package with its own table (`packages/passkey/`).

Primary keys are 32-character random alphanumeric strings by default (`packages/core/src/utils/id.ts:3-5`).
That default is configurable: `advanced.database.generateId` accepts `"uuid"`, which on Postgres uses `gen_random_uuid()` (`packages/core/src/types/init-options.ts:444-458`).
This matters, because every identifier in our schema is a `uuid`.

Sessions are database-backed and opaque.
The default lifetime is seven days with a one-day refresh interval and a one-day freshness window for sensitive operations (`packages/core/src/types/init-options.ts:1020-1039`, `:1150-1156`).
Revocation is explicit: `/revoke-session`, `/revoke-sessions` and `/revoke-other-sessions` (`packages/better-auth/src/api/routes/session.ts:663`, `:744`, `:799`).
An optional cookie cache trades a database read for a signed five-minute cookie, and the documentation is explicit that with it enabled a revoked session can remain active on other devices until that cookie expires (`packages/core/src/types/init-options.ts:1073-1078`, `docs/content/docs/concepts/session-management.mdx:238-240`).

### The two plugins that let a separate backend trust a session

The bearer plugin is not what its name suggests for our purpose.
It reads an `Authorization: Bearer` header and rewrites it into better-auth's own session cookie before better-auth's handler runs (`packages/better-auth/src/plugins/bearer/index.ts:41-120`).
The token it carries is better-auth's opaque session token, and better-auth is still the thing that verifies it.
It solves "my client cannot hold cookies", not "my Rust service needs to authenticate a request without calling Node".

The JWT plugin is the mechanism we want.
It adds two endpoints: `GET {basePath}/jwks`, which publishes the public JSON Web Key Set (`packages/better-auth/src/plugins/jwt/index.ts:58`, `:109`), and `GET {basePath}/token`, which requires a live session and returns a signed JWT (`:249-250`, `:254`).
It also sets a `set-auth-jwt` response header on `/get-session`, so a client that is already fetching its session gets a fresh token without a second round trip (`:374-375`).
The documentation states the intended contract directly: "The token can be verified in your own service, without the need for an additional verify call or database check.  For this JWKS is used." (`docs/content/docs/plugins/jwt.mdx:140-141`).

Signing defaults to EdDSA over Ed25519 (`packages/better-auth/src/plugins/jwt/sign.ts:227`), the key pair is stored in a `jwks` table with the private half symmetrically encrypted under the better-auth secret (`packages/better-auth/src/plugins/jwt/schema.ts:3-39`, `sign.ts:215-226`), and the signing key's identifier travels as the JWS `kid` header so a verifier can select the right key and refetch the set when it sees an unknown one (`docs/content/docs/plugins/jwt.mdx:143-145`).
Issuer and audience both default to the configured base URL, and the default expiry is fifteen minutes (`sign.ts:281-283`, `:298`, `:302`).
The default payload is the entire user object with `sub` set to the user id (`sign.ts:352-370`), and both are overridable through `definePayload` and `getSubject`, each of which receives the full `{ user, session }` pair (`packages/better-auth/src/plugins/jwt/types.ts:126-141`).

### The organization plugin

The organization plugin adds `organization` (`name`, unique `slug`, `logo`, `createdAt`, `metadata`), `member` (`organizationId`, `userId`, `role` defaulting to `member`, `createdAt`) and `invitation` (`organizationId`, `email`, `role`, `status`, `expiresAt`, `createdAt`, `inviterId`), with optional `team`, `teamMember` and `organizationRole` tables behind feature flags (`packages/better-auth/src/plugins/organization/organization.ts:1086-1224`).
Membership is many-to-many by construction, which is exactly the enterprise shape the founder described.

It also extends the `session` table with `activeOrganizationId`, marked `input: false` (`packages/better-auth/src/plugins/organization/organization.ts:1254-1262`), set through `organization.setActive` (`docs/content/docs/plugins/organization.mdx:623`).
The model is therefore that a user may belong to many organisations, and each browser session has one active organisation at a time.
Every table it creates uses `modelName` overrides, so the plugin can be pointed at differently-named tables, and `additionalFields` can extend them (`organization.ts:1089`, `:1129`, `:1167`).

## Recommended topology

Run better-auth as a standalone Node process — call it `tam-auth` — serving only `/api/auth/*`, reverse-proxied onto the same origin as the dashboard and the API.
Do not run it inside a SvelteKit server.

The reason is the shipped deployment shape.
The client is a static bundle with `ssr = false` (`web/src/routes/+layout.ts:3`) built by `adapter-static` (`web/svelte.config.js:1-13`) and served by `tam-server` as files plus a fallback (`crates/tam-server/src/main.rs:163-184`).
Mounting better-auth in a SvelteKit `handle` hook, which is what the documented integration does (`packages/better-auth/src/integrations/svelte-kit.ts:15-37`, `docs/content/docs/integrations/svelte-kit.mdx:11-22`), requires switching to `adapter-node` and running a SvelteKit server in production.
That would put a TypeScript process on the request path of every page rather than of the auth endpoints alone, which is a far larger amendment to the language boundary than the founder is asking for.
A standalone process running `toNodeHandler(auth)` (`packages/better-auth/src/integrations/node.ts:5-13`) keeps the amendment surgical: one process, one responsibility, one set of grants.

Because there is no SvelteKit server, the documented `sveltekitCookies` plugin and its `event.cookies` bridging (`packages/better-auth/src/integrations/svelte-kit.ts:58-103`) are not needed and should not be configured.
The browser talks to `/api/auth/*` directly and better-auth sets its own `Set-Cookie` on those responses.
The dashboard uses better-auth's browser client instead, which ships a Svelte binding (`packages/better-auth/src/client/svelte/`, and the `./svelte` entry in `packages/better-auth/package.json`).

Same-origin is a requirement, not a convenience.
better-auth's session cookie must be first-party relative to the dashboard, and the documentation flags Safari blocking authentication cookies outright when the auth API is on a different domain from the frontend (`docs/content/docs/concepts/cookies.mdx:112`).
The origin already exists: `tam-server` serves `/v1/*` and the client from one address.
Add `/api/auth/*` to that origin at the ingress or reverse proxy, which requires no Rust and no new dependency, and mirror it in development by adding one line to the existing Vite proxy table (`web/vite.config.ts:9-16`).
Routing it inside `tam-server` instead would work but means writing a proxy in Rust for no gain.

The request flow in prose.
The browser loads the static dashboard from `tam-server`.
Sign-in, social callback, passkey ceremony, email verification and password reset all happen against `/api/auth/*`, which the proxy routes to `tam-auth`; better-auth writes a row in its `session` table and sets its session cookie on the dashboard's own origin.
The dashboard then obtains a short-lived JWT from `GET /api/auth/token` and posts it once to the API's existing `POST /v1/session`.
The Rust API verifies that JWT locally against a cached JWKS, with no call to Node, resolves the subject to an `app_user` row, and mints its own `tam_session` cookie exactly as it does today.
Every domain request after that — including the `EventSource` progress stream, which cannot carry a header — authenticates on that cookie, and `app.current_org` is pinned unchanged.
`tam-auth` is never in the path of a domain request, and is on the login path only.

## Table ownership and the org fork

For launch, better-auth owns identity only.
Its tables are `user`, `session`, `account`, `verification`, `jwks`, plus `passkey` if passkeys ship in the first cut and `rateLimit` if rate-limit storage is put in the database.
`organisation`, `app_user` and every tenant table stay exactly where they are, owned by Rust migrations and fenced by row-level security.

Put better-auth's tables in their own Postgres schema, `auth`, reached by their own login role, `tam_auth`, whose `search_path` is set to that schema with `ALTER ROLE tam_auth SET search_path = auth`.
This works precisely because the Kysely adapter emits unqualified table names (`packages/kysely-adapter/src/kysely-adapter.ts:195`), so `search_path` fully determines placement, and it buys three things at once.
It avoids a collision of concepts, because a table literally named `user` alongside our `app_user`, and `session` alongside our `user_session`, is a naming accident waiting to be misread.
It leaves the row-level-security matrix test untouched, because that test already scopes its catalogue query to `nspname = 'public'` (`crates/tam-storage/tests/rls_matrix.rs:66-72`), so tables in `auth` are outside its closed world by construction rather than by an added exemption.
And it makes the trust boundary a database grant rather than a convention: `tam_auth` receives privileges inside `auth` and nothing at all in `public`.

Forced row-level security is not needed on better-auth's tables, and that is not a gap.
Row-level security in this codebase fences tenant data by `app.current_org`; the `auth` schema holds no tenant data, and exactly one role can reach it.
The equivalent guarantee is delivered by grants instead of policies.

Do not run better-auth's tables through sqlx's migrator.
Generate the DDL with the better-auth CLI's `generate` action rather than letting `migrate` write to a live database, review the SQL, and land it as a numbered file under a separate `db/auth/` directory applied as `tam_auth`.
The workspace's migration discipline is a reviewed, numbered, ordered sequence, and an opaque CLI mutating production schema is at odds with it.

### Why orgs stay in Rust

Three reasons, in order of weight.

Organisation identity is a database concept here, not an application concept.
`app.current_org` is the row-level-security key (`crates/tam-storage/src/lib.rs:97-106`), twenty-nine tables carry a forced policy on it, and a test fails the build if any tenant table stops carrying one (`crates/tam-storage/tests/rls_matrix.rs:13-41`, `:95-113`).
Moving the authoritative organisation row into a table written by a Node process makes the referent of that key something Rust does not control.

`organisation.id` is a `uuid` and a foreign key from every tenant table (`crates/tam-storage/migrations/0001_product.sql:5`, `:13`).
better-auth's `organization.id` is a random string by default (`packages/core/src/utils/id.ts:3-5`); it can be made a `uuid` (`packages/core/src/types/init-options.ts:458`), so this is surmountable, but it would still leave twenty-nine foreign keys and the per-tenant key derivation in the custody vault pointing at a row Node owns.

better-auth's active organisation is session state (`packages/better-auth/src/plugins/organization/organization.ts:1254-1262`), which is a user-interface affordance.
Our row-level-security pin needs the organisation to be a property of the request, decided server-side from data the server trusts.
Those are reconcilable — the active organisation can travel as a claim and be re-checked against a membership table — but with one organisation per user today (`crates/tam-storage/migrations/0014_sessions_and_operations.sql:10`) the question does not arise at launch, and taking on the reconciliation now buys nothing.

### The enterprise vision does not require the plugin

The founder's future model — schools with users under them, users affiliated with several organisations, resources co-authored across users and organisations — decomposes into a membership table and an authorship model.
The membership half is four columns: `org_membership(user_id, org_id, role, created_at)`, plus dropping `app_user.org_id NOT NULL`, plus a request-scoped organisation selection that the API validates against membership before pinning.
That is a Rust migration and an extractor change, and it needs better-auth for none of it.
The co-authorship half is a domain model over `product` and is further from auth still.

What the organization plugin adds beyond a membership table is the invitation lifecycle, a role and permission system, teams, and the client-side hooks.
That is real value and a genuine reason to revisit.
But adopting it later as the authority for membership means either migrating the referent of twenty-nine foreign keys, or running two authorities for membership and accepting drift, which is the failure mode worth avoiding by deciding ownership once, now.
The recommendation is therefore to state deliberately that Postgres and Rust own organisation and membership permanently, and that if better-auth's invitation experience is wanted later it is adopted as a front end over our tables (see option three below), not as a second source of truth.

## Session bridging

The mechanism is a JWT verified against JWKS with no call to Node, exchanged once at login for the API's own session cookie.
Exchanging is the recommendation rather than sending a bearer token on every request, and the reason is a concrete constraint in the shipped client.

### Why an exchange rather than a bearer header on every request

The progress stream is Server-Sent Events consumed by the browser's native `EventSource` (`web/src/lib/ledger.ts:1`, `web/src/routes/jobs/[id]/+page.svelte:39`, `web/src/routes/queue/+page.svelte:33`).
`EventSource` cannot set request headers; it sends cookies and nothing else.
The route is built around that API's behaviour deliberately: it answers a dead session with 204 rather than 401, because a failed SSE response permanently halts reconnection (`crates/tam-api/src/stream.rs:1-8`, `crates/tam-api/src/session.rs:1-6`, `crates/tam-api/src/session.rs:77-92`), and it resumes from `Last-Event-ID` (`crates/tam-api/src/stream.rs:3-4`, `:89`).
An `Authorization`-header design therefore either breaks the stream, or puts a token in the query string where it lands in access logs and `Referer`, or forces a hand-rolled `fetch` reader that gives up the reconnection and cursor handling the route is designed around.
None of those is worth paying when the exchange is available.

### The exchange

Keep `tam_session` as the API credential, and make better-auth's JWT the login assertion that mints it.
The endpoint already exists in the right shape: `POST /v1/session` takes a token, resolves it, sets the `HttpOnly` cookie with a `Max-Age` honest to the session's own expiry, and returns the identity (`crates/tam-api/src/lib.rs:97-100`, `crates/tam-api/src/session.rs:126-173`).
Change what it accepts, not what it does.

The flow, in prose.
The user signs in against `/api/auth/*` and better-auth sets its session cookie on the shared origin.
The dashboard fetches a JWT from `GET /api/auth/token`, which requires that cookie (`packages/better-auth/src/plugins/jwt/index.ts:249-250`, `:254`).
It posts that JWT once to `POST /v1/session`.
The Rust API verifies it against a JWKS cached in `AppState` — selecting the key by the `kid` header, refetching `/api/auth/jwks` on an unknown one, and checking `iss`, `aud` and `exp` — then resolves the subject to an `app_user` row, mints a `user_session` row, and sets the `tam_session` cookie.
Every subsequent request, the `EventSource` stream included, authenticates exactly as it does today.

What this buys is that almost nothing downstream changes.
`OrgContext`, `StreamAuth`, `resolve` and the cookie parser are untouched (`crates/tam-api/src/session.rs:22-25`, `:30-39`, `:50-92`), as is `pin_org` and the whole row-level-security path (`crates/tam-storage/src/lib.rs:97-106`).
Revocation on the API side stays exact rather than expiry-bounded, because logout still deletes the row (`crates/tam-api/src/session.rs:177-197`, `crates/tam-storage/src/sessions.rs:172-180`).
The JWT is verified once per login rather than once per request, so JWKS caching is not on a hot path.
And the token never rests anywhere: it is fetched, posted same-origin, and discarded.

Configure the JWT plugin with a deliberately narrow payload.
The default payload is the whole user object (`packages/better-auth/src/plugins/jwt/sign.ts:356-358`), which puts name, email and image into a token crossing a trust boundary for no reason; override it.

```ts
jwt({
  jwt: {
    issuer: "https://app.example.com",
    audience: "tam-api",
    expirationTime: "2m",
    definePayload: ({ user }) => ({ email_verified: user.emailVerified }),
  },
})
```

The claim set is then `sub`, `iss`, `aud`, `iat`, `exp` and one boolean.
`sub` is better-auth's `user.id`.
Set `advanced.database.generateId: "uuid"` (`packages/core/src/types/init-options.ts:444-458`) so `sub` is a Postgres `uuid` and can be stored as one rather than as text.
The expiry can be short — two minutes is ample — because the token is redeemed immediately and never reused.

Note what the token is not asked to carry.
It asserts who the human is; Postgres decides which organisation they speak for and what they may do there.
No claim is consulted when the tenant boundary is drawn, so a forged or stale claim cannot widen one.
That is what makes the multi-organisation future cheap: when a user gains several organisations the token does not change at all, only the lookup and a request-scoped selection do.

### Linking and provisioning

Linking a better-auth user to `app_user` needs one column and one decision.
Add `app_user.auth_subject uuid UNIQUE`, nullable at first so existing operator-minted rows survive, and link on it rather than on email, because email is mutable in better-auth and is the wrong join key for an identity.
The decision is who creates the `app_user` row for a subject seen for the first time.
The recommendation is provisioning in Rust inside the exchange: an unknown `sub` with a verified token creates `organisation` and `app_user` in one transaction, then mints the session.
This keeps every write to our tables in Rust, needs no callback from Node, and is exactly the self-serve signup M5 wants (`docs/design/milestones.md:142`).
It does mean anyone who can complete a better-auth registration can create a tenant, which is what signup means; gate it with better-auth's email verification and rate limiting, and refuse the exchange when `email_verified` is false.

### Where the two systems can diverge, and what to do about it

Two session lifetimes now exist, and they are not automatically coupled.
Signing out of better-auth stops new tokens being issued but does not by itself end an already-minted API session.

Three measures close that, and all three are cheap.
The dashboard's sign-out calls both `/api/auth/sign-out` and `DELETE /v1/session`, so the ordinary case is exact on both sides.
The API session's lifetime is set no longer than better-auth's, whose default is seven days (`packages/core/src/types/init-options.ts:1020-1025`), so a lost sign-out cannot outlive the identity.
And the dashboard re-exchanges on a fixed interval — hourly, say — which bounds how long a revoked identity keeps an API session to that interval, because a re-exchange against a revoked better-auth session fails and the client signs out.
For an immediate kill independent of both, the authority must be a table Rust reads: a `disabled_at` column on `app_user` checked during resolution, or the existing `org_halt` fleet switch.

State the guarantee plainly rather than implying a stronger one.
Sign-out is exact when the client performs both calls; a revoked identity whose client did not is bounded by the re-exchange interval; and anything faster than that goes through a Rust-side kill switch.
Leave better-auth's cookie cache off, or the same class of lag applies to better-auth's own session checks as well (`docs/content/docs/concepts/session-management.mdx:238-240`).

### The alternative, named for completeness

If the stream ever moves off `EventSource` — to WebSocket, or to a `fetch`-based reader that owns its own reconnection — the pure bearer design becomes available: the dashboard holds a short-lived JWT and sends it on every request, and `resolve` verifies it and looks up `app_user` in place of `user_session`.
That lookup costs what the current one costs (`crates/tam-storage/src/sessions.rs:151-169`), so it is not a new round trip, and it removes a table and a credential from the system.
Its price is that revocation becomes expiry-bounded rather than exact, and that JWKS verification moves onto the hot path.
It is a reasonable end state and a bad starting point, because it requires rewriting a stream route that currently works.

## Security boundary

better-auth joins the trust surface for exactly one thing: platform-user identity and browser session.
That is who the human is, whether they have proven control of an email address or a passkey, and whether their browser currently holds a live login.
It also holds the private half of the token-signing key pair, encrypted under its own secret (`packages/better-auth/src/plugins/jwt/schema.ts:10-13`, `packages/better-auth/src/plugins/jwt/sign.ts:215-226`), which is inherent to issuing tokens the API trusts.

It must never touch the following, and under this design cannot, because `tam_auth` holds no privilege in `public`.
The credential vault and `connection_secret`, which since the broker's retirement no role may read (`crates/tam-storage/migrations/0051_retire_tam_broker_grants.sql`, `docs/notes/runbooks/retire-tam-broker-role.md`).
Any tenant table, and therefore `app.current_org`.
Any domain read or write, any marketplace request, any part of the engine, worker or transport path.

The crown-jewel invariant is preserved, and it is worth stating what the blast radius actually is rather than asserting safety in general.
Marketplace credentials never leave the Rust custody path: they are encrypted with per-tenant data-encryption keys, and since the broker's retirement no role holds a grant on `connection_secret` at all, so no server-side process unseals one (`crates/tam-storage/migrations/0051_retire_tam_broker_grants.sql`, `docs/notes/runbooks/retire-tam-broker-role.md`).
An attacker with full control of `tam-auth` holds the signing key and can therefore mint a valid token for any subject, and so can act as any seller through the API.
They still cannot read a credential, because the API path itself cannot read one — `tam_app` is denied `connection_secret` by grant.
That is the same blast radius an attacker who could write `user_session` rows has today, so this design moves the credential for platform identity without widening what compromising it yields.

One consequence deserves an explicit note.
`tam-auth` needs its own Postgres connection string and its own secret, and those are new operational secrets with a new process to hold them.
They must not be co-located with any key material that would unseal `connection_secret`, on the general principle that a process which cannot reach a capability cannot leak it.

## The TypeScript boundary, amended

The current rule appears in three places with the same force.
The decision record says "Rust is non-negotiable for the engine, all I/O, batch processing, the automation layer, and anything computationally heavy.  TypeScript is acceptable for the user interface only." (`docs/design/decisions.md:26-27`).
`CLAUDE.md` repeats it as a non-negotiable, and the engineering charter states it as a requirement in the core rather than a preference (`docs/design/engineering-charter.md:13-17`).

This integration amends it, because a Node process serving `/api/auth/*` writes to Postgres and is not a user interface.
Proposed wording, bounded so that ratifying it does not ratify anything further.

> Rust is required for the engine, all I/O, batch processing, the automation layer, and anything computationally heavy.
> TypeScript is for the user interface, and for one bounded identity service.
> That service is better-auth, running as `tam-auth`, and it owns platform-user identity and browser session only: registration, sign-in, social and passkey credentials, email verification, password reset, and the keys for the tokens it issues.
> It owns no domain data, performs no marketplace request, and holds no marketplace credential.
> It reaches Postgres only as the `tam_auth` role, whose grants are confined to the `auth` schema; it never reads or writes a table in `public` and never sets `app.current_org`.
> Authorisation — which organisation a request speaks for and what it may do there — is decided in Rust from Postgres, and is never asserted by a token claim.
> Any extension of this service beyond identity and session is a new founder decision, not an application of this one.

The last two sentences are the ones doing the work.
The second-to-last states the property that keeps the amendment safe; the last blocks the amendment from being read as a general licence.
The amendment is also mechanically checkable in the way the charter prefers (`docs/design/engineering-charter.md:11`): the grant list on `tam_auth` is a test, and `crates/tam-storage/tests/rls_matrix.rs` already fails if a table appears in `public` without a tenancy decision.

Amended 2026-09-04: the quote now reads as ratified in `CLAUDE.md`'s non-negotiables, which drop the proposal's closing clause on never contacting the session broker, because D1 retired the broker itself; what stood behind that clause is the grant list, and `docs/notes/runbooks/retire-tam-broker-role.md` retires the role in production.
The ratified text also carries one sentence this quote does not, exempting the client entitlement token (D10 in `docs/notes/design/vendoo-for-teachers-rethink.md`).

An unrelated staleness surfaced while reading.
`docs/design/engineering-charter.md:16` states "the web client is React and TypeScript", and `:116` repeats it.
The shipped client is Svelte 5 and SvelteKit (`web/package.json:12-24`).
The charter's last edit is 2026-08-25 and the decision record's is 2026-08-29, so the charter is the older document and the code supersedes it; this is flagged rather than fixed here, since it is outside this note's scope.

## Options

Option one, better-auth as identity and session only, orgs stay in Rust.
better-auth owns `user`, `session`, `account`, `verification` and `jwks` in an `auth` schema; the Rust API verifies a JWT against JWKS at login, exchanges it for its own `tam_session` cookie, and resolves the organisation from `app_user`.
Smallest amendment to the language boundary, no change to the row-level-security model, no change to the custody path, and the existing `OrgContext` extractor keeps its shape.
Costs: a second identity table to keep linked to `app_user`, and the invitation and role experience must be built in Rust when it is wanted.
This is the recommendation.

Option two, better-auth owns organisations too.
Add the organization plugin as the authority for `organization`, `member` and `invitation`, and carry `activeOrganizationId` as a claim.
Buys the multi-organisation model, invitations, roles and teams immediately, and matches the enterprise vision directly.
Costs: the referent of `app.current_org` becomes a row a Node process writes, twenty-nine foreign keys point into a table outside Rust's migrations, the tenant boundary starts depending on a claim rather than on a database lookup, and the per-tenant key derivation in the custody vault keys on an identifier Node mints.
That is a change to the tenancy core in exchange for features nothing is currently blocked on, and it should be refused for launch.

Option three, better-auth's organization plugin pointed at our tables.
The plugin supports `modelName` overrides on every table it declares and `additionalFields` on each (`packages/better-auth/src/plugins/organization/organization.ts:1089`, `:1129`, `:1167`), so it can be configured against `organisation` and a Rust-owned `org_membership`.
This is the path to the invitation experience without a second authority, and it is worth naming now so that later adoption is a configuration change rather than a migration.
Costs: it requires `organisation` to grow the columns the plugin requires, notably a unique `slug` and `metadata`; it requires granting `tam_auth` write access to specific `public` tables, which breaches the clean grant boundary option one buys; and it makes the plugin's field expectations a constraint on our schema.
Defer it, but shape `org_membership` with it in mind when multi-organisation membership is built.

## Open questions and risks

Q1, is the JWT a per-request credential or a login assertion.
Recommendation: a login assertion, exchanged once for the API's own session cookie, because the `EventSource` stream cannot carry a header and the exchange endpoint already exists (`crates/tam-api/src/lib.rs:97-100`, `crates/tam-api/src/session.rs:126-173`).
Reading better-auth's `session` table from Rust would be the third alternative; it gives exact revocation but couples two schemas across a trust boundary and re-introduces the cross-schema grant that the `auth`-schema isolation exists to avoid.

Q2, revocation across both systems.
Sign-out is exact on both sides when the dashboard calls `/api/auth/sign-out` and `DELETE /v1/session`; a revoked identity whose client did neither keeps its API session until the next re-exchange.
The founder decision is the re-exchange interval, which is the revocation lag; one hour is proposed, and the API session's own lifetime must not exceed better-auth's seven-day default (`packages/core/src/types/init-options.ts:1020-1025`).
better-auth's cookie cache stays off, or the same class of lag applies to better-auth's own checks (`docs/content/docs/concepts/session-management.mdx:238-240`).

Q3, the fate of `tam-mint-session`.
It becomes a break-glass and test-fixture tool rather than the login flow.
Recommendation: keep the cookie path alive behind a flag through the transition so the current dashboard keeps working while better-auth is stood up, then delete the cookie extractor branch and the sign-in paste page (`web/src/routes/login/+page.svelte`) once the new path is proven, rather than carrying two credentials into production.
Running two accepted credential types indefinitely is the risk here; it doubles the authentication surface for no benefit after cutover.

Q4, SvelteKit server-side-rendering cookie handling.
There is nothing to handle, because there is no SvelteKit server (`web/src/routes/+layout.ts:3`, `web/svelte.config.js:1-13`), and this is a reason to keep it that way.
If server rendering is ever wanted, that is a separate decision that pulls the SvelteKit server onto the request path, and it should be taken on its own merits rather than inherited from this one.

Q5, the database role and schema for better-auth.
Recommendation as above: a `tam_auth` login role with `search_path` set to an `auth` schema, no privileges in `public`, and no `BYPASSRLS`.
The residual question is whether `tam_auth` owns the `auth` schema or whether a separate owner applies the DDL and grants only DML, which is the stronger posture and costs one more role.

Q6, identifier alignment.
`advanced.database.generateId: "uuid"` makes better-auth's ids Postgres uuids (`packages/core/src/types/init-options.ts:444-458`), which lets `app_user.auth_subject` be a `uuid` rather than text.
This must be set before the first user row is written, because changing it afterwards is a data migration across every better-auth table.

Q7, key rotation and JWKS caching in Rust.
The public set can be cached indefinitely and refetched on an unknown `kid` (`docs/content/docs/plugins/jwt.mdx:143-145`), but the refetch needs a bound and a rate limit, or an unknown-`kid` flood against the exchange endpoint becomes an amplification path onto `tam-auth`.
No JWT or JWKS crate is in the workspace today, so this needs a dependency, which is a founder decision under the enforcement rules in `CLAUDE.md`.
EdDSA over Ed25519 is the default algorithm (`packages/better-auth/src/plugins/jwt/sign.ts:227`), so whichever crate is chosen must support it, or the algorithm must be pinned to something the chosen crate supports.

Q8, where email actually gets sent.
Email verification and password reset require a transport, and better-auth expects a send function rather than providing one.
This is unbudgeted work and an unchosen vendor, and it gates the "batteries-included" value of adopting better-auth at all.

R1, a second process in the deployment.
`tam-auth` is a new process, a new secret set, and a new restart and upgrade path.
Its failure mode under the exchange design is benign and should be documented as such: if `tam-auth` is down, nobody can sign in or re-exchange, while every already-established API session keeps working for its full lifetime, because the domain path never consults it.
The corollary is the same fact read the other way — an outage lasting longer than the re-exchange interval logs everyone out as their interval elapses, so that interval is an availability parameter as well as a revocation one.

R2, the Node dependency tree.
better-auth pulls a substantial npm dependency graph into a repository whose supply-chain posture is currently enforced by `cargo-deny` over Rust crates only (`deny.toml:1-41`).
There is no equivalent gate on the client's `node_modules` today, and adding one that reaches the auth service is a prerequisite worth naming rather than discovering later.

R3, the amendment's edge.
Once a TypeScript process legitimately writes to Postgres, the argument for the next one is easier to make each time.
The last sentence of the proposed wording exists for that reason and should not be dropped when the amendment is transcribed.
