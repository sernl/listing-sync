# Linking a marketplace account for real

Today a seller cannot connect anything.
`crates/tam-api/src/lib.rs:152-156` exposes two connection routes, a list and a revoke, and there is no link route at all.
The only path that seals a credential is `tam-session-broker link-from-jar`, an operator command reading a Netscape jar off disk (`crates/tam-session-broker/src/main.rs:188-243`).
This note designs the seller-facing replacement for Tes and Tpt, and marks where the evidence runs out rather than filling the gap with assertion.

Claims about marketplace defences cite the research notes held in the session memory research directory; claims about this tree cite file and line.
Sections 1 to 7 are the design; section 8 is what a founder must rule before any of it is built.

## 1. What the seller sees

One screen shape for both marketplaces, because the seller's question is the same and only the mechanism behind it differs.
The connections page (`web/src/routes/connections/+page.svelte`) gains a *Connect* action per marketplace beside the revoke it already has.
Pressing it asks for the marketplace email and password, states in one sentence that we sign in on our own servers and hold a session rather than a password, and asks for the authorship name Tpt requires (`crates/tam-session-broker/src/protocol.rs:93-98`).
On success the row moves to `checking` and then to `connected` the first time a real read comes back 2xx, which is the only thing that writes `session_verified_at` (`crates/tam-session-broker/src/gateway.rs:218-227`).

A wrong password is the marketplace's own answer relayed verbatim in substance and not in bytes: the row returns to `disconnected`, the form stays filled except for the password, and nothing is sealed.
A challenge the seller can clear — a mailed one-time password, a second factor — is shown as a second step in the same flow, because `seller_clears` already classifies those as theirs to answer (`crates/tam-domain/src/lib.rs:542-547`).
A challenge the seller cannot clear from a form — a captcha, a Cloudflare interstitial — is the edge answering our address rather than our credential, and the same predicate already says no seller action reaches it; the honest screen says the marketplace would not let us sign in from our servers right now, and offers the attended path of section 3 rather than a retry button that will fail identically.
Neither marketplace's two-factor behaviour has been captured, so the second-step branch is designed and unproven; Tes's one-time-password re-challenge is still an open question on the longevity probe (`docs/notes/probes/08-session-longevity.md:13`).

## 2. The credential path

The password travels browser to `tam-server` to the broker's unix socket, and stops in the broker's address space.
That is one new route, `POST /{version}/connections/{connection}/link`, relaying to one new socket op, exactly as `revoke_connection` already relays `Revoke` (`crates/tam-api/src/resources.rs:383-416`).
The broker is the only process holding the key-encryption key and the only role that can read `connection_secret`; `tam_app` had that access revoked outright (`crates/tam-storage/migrations/0010_broker_custody.sql:7`) and `tam_engine` was never granted it.

The role fence therefore holds by construction: the API process relays bytes and stores none, so the credential's only durable home stays the vault and its only plaintext home stays the broker.
The process fence is weaker and should be stated rather than implied — the password is briefly in `tam-server`'s address space.
That is acceptable, and the reason is not tolerance: the password arrives through a form `tam-server` itself rendered, so a compromised `tam-server` can simply collect it by serving its own field, and routing around the relay buys nothing against the attacker who is already there.
What the relay must do is wrap the field in `tam_secrets::Secret` at the first opportunity, keep it out of body tracing and out of every error string, and never place it in a log line.

A narrow HTTP surface on the broker is the alternative, and it is the worse trade.
The broker's whole design is one narrow surface with no attacker-facing parser (`crates/tam-session-broker/src/main.rs:1-9`), and `Claim` exists as a separate op precisely so that marketplace response parsing never happens in the key-holding process (`crates/tam-session-broker/src/protocol.rs:44-52`).
A browser cannot reach a unix socket, so an HTTP surface means a TCP listener, TLS termination, session and CSRF verification and origin checks — an attacker-reachable parser inside the one process that holds the KEK, which is the blast radius the boundary exists to keep small.
Blast radius of the relay is one request's plaintext in a process that already sees everything the seller types; blast radius of a broker listener is the KEK.

## 3. Minting a session on our own infrastructure

Three options, evaluated per marketplace against what the research actually shows.

Plain-HTTP login replay is unproven for both, and not merely unbuilt.
Tpt's login was never captured — the bot verdict's own open questions record that the session was pre-established in both HARs and that a login capture is needed to settle whether the auth path demands a reCAPTCHA token (`tptw-bot-verdict.md`, OPEN Q), and `docs/design/decisions.md:223` records the same gap.
Tes's login was likewise never captured; the wire research covers the uploader, dashboard and refresh routes, not `/authn/sign-in`.
So no capture exists on either platform from which a replay could be built, and a replay designed without one would be a guess at a security-critical path.

A headless browser on our own box is the credible unattended vehicle, and the shaped hole for it already exists.
`docs/design/decisions.md:154` deletes the browser module while stating that a future browser-driven connector re-admits one with its own evidence, and `decisions.md:223` scopes the residual browser-shaped dependency to session establishment in the broker's link step specifically.
Browserless is rejected and the reason is not preference: it is SSPL-or-commercial with the vendor's own published position putting closed-source commercial use on the paid side, and the free image's capability delta over driving Chrome directly is zero because stealth and captcha handling are Enterprise-only (`research-browserless-custody.md`).
The vehicle, if this is built, is a small chromiumoxide driver the broker launches and kills per attempt, with no persisted profile, no session recording and no screenshot artifact, because a persisted profile is an unencrypted credential store outside the vault.

An attended flow — the seller completing the sign-in themselves, challenge included — is already named in the decision record as one of the two better custody models (`docs/design/decisions.md:42`), and the type for it already exists as `CustodyModel::SellerDrivenSession` (`crates/tam-marketplace/src/lib.rs:719-720`).
Its cost is a real build: a browser we run whose viewport and input are relayed to the seller.
Its property is that no marketplace password ever enters our custody and every challenge is answered by the human the challenge is asking about.

Captcha reality, per marketplace, from the research.
Tpt's write path carries no bot-management token of any kind — an exhaustive search of 511 HAR entries found zero captcha fields, headers or query parameters on any write, and the reCAPTCHA Enterprise token minted on page load is posted only to Google and never forwarded to Tpt (`tptw-bot-verdict.md`).
But that is the write path, not the login: reCAPTCHA Enterprise v3 invisible is wired into the SPA shell under sitekey `6Lfx3XAqAAAAAP87NzenyC1jbWKA47hF90YsvN0Y` alongside a `v-3-recaptcha-migration` feature flag, which the same note reads as gating *some* flow it could not identify.
Cloudflare is the second axis and it is untested: `Set-Cookie: cf_clearance` occurs zero times across both captures, so the clearance was carried in and its issuance path to a cold datacentre client is unobserved.
Tes shows no captcha in any capture and no anti-automation clause in its binding author terms (`docs/notes/probes/01-tes-terms-antiautomation.md:13-14`), but its login gate `/authn/` is disallowed in robots.txt (`docs/research/feasibility-report.md:123`), which is a compliance question rather than a technical one.

Recommendation, Tes: attended sign-in, and it is enough on its own.
Session renewal is proven in house — `GET /api/authn/refresh-cookies` driven hourly by `probes/session-longevity.sh` held a session 200-authed for over four and a half days with zero failures, while the same account's non-renewing sealed copy expired inside a day — and the gateway now absorbs renewals from ordinary traffic on 2xx answers only (`crates/tam-session-broker/src/gateway.rs:486-491`).
So one attended sign-in is genuinely one-and-done for Tes, a stored password buys nothing, and the robots question at `/authn/` never arises because a human is at the gate.

Recommendation, Tpt: attended sign-in, and today it is the only credible option.
Unattended login is unproven on two independent axes at once, and Tpt additionally has no session-renewal route at all (`crates/tam-session-broker/src/gateway.rs:97-102`), so its session can only age.
The attended flow also mints `cf_clearance` in the same sitting, which is the exact thing the document-navigation path lacks when it answers 302 to `/Request-Authorization` from a jar that authenticates every other route (`docs/design/decisions.md:361-363`).

What reverses this: the founder-approved headless-Chrome login probe, run from production egress against Tpt's login and one document navigation.
A non-interactive JavaScript challenge makes the chromiumoxide driver viable and demotes the attended flow to a fallback; an interactive managed challenge leaves attended standing permanently.
One further unknown rides on that probe and would change the architecture rather than the flow: whether a browser-minted `cf_clearance` survives replay by the plain-HTTP engine from the same address, or is fingerprint-bound, in which case the browser becomes a permanent proxy on the document path rather than a link-time helper.

## 4. Identity exclusivity

The mechanism is built and the constraint is live.
`connection_platform_account_exclusive` is a global partial unique index on `(marketplace, platform_account_digest)` where the digest is non-null and the state is one of `linking`, `linked`, `needs_reauth` (`crates/tam-storage/migrations/0031_connection_custody.sql:59-62`), and it is the one deliberate exception to the tenant-scoped uniqueness every other constraint keeps.
The stored value is an HMAC-SHA-256 over `(marketplace || account_ref)` under a pepper the broker derives from its own key material, so a database dump yields no storefront identifiers (`0031_connection_custody.sql:8-12`).
The lock releases by the state predicate, so an unlink or a revoke frees the account without a delete.

What identifies an account server-side differs by marketplace, and only a server-asserted value is admissible — a seller-typed handle would let anyone lock a real storefront's owner out of their own account, and the broker refuses one structurally (`crates/tam-session-broker/src/vault.rs:323-335`).
Tpt's is `author { id }` on the `Store` type, present in every committed cassette (`crates/tam-session-broker/src/vault.rs:36-38`), read through the seller's own session rather than any caller-supplied id.
Tes's is not wired: `identity_source(Tes)` is `Unavailable` (`crates/tam-session-broker/src/vault.rs:46-51`) and `Vault::claim` refuses outright, so a Tes link takes no lock and Tes exclusivity is not enforced today.
The value exists — an authenticated `GET /api/tier/gmv/me` returns a `userId`, captured in `probes/local/tes-upload.har` — and turning it on needs three edits, one of which adds `/api/tier` to the gateway allow-list (`crates/tam-session-broker/src/gateway.rs:39-46`); that allow-list is a security surface, so it is question Q5 rather than an implementation detail.

What the seller sees on collision: a refusal naming the marketplace and nothing else, carried by the closed-set code `PlatformAccountAlreadyLinked` (`crates/tam-session-broker/src/protocol.rs:105-110`, `crates/tam-api/src/error.rs:71`), deliberately carrying no hint of who holds it so the constraint never becomes a directory of the platform's sellers.
The losing connection is moved to `needs_reauth` rather than left linked, because it holds a credential for a storefront another organisation owns (`crates/tam-session-broker/src/vault.rs:356-366`), and that reads `disconnected` on the page.
The residual disclosure is an existence oracle reachable only by someone who already supplied a working session for that account, and the support path is a person, not a self-service override.

One honest gap: the claim is opportunistic backfill on the first live 2xx read (`crates/tam-session-broker/src/vault.rs:338-347`), so between a successful link and that first read the connection holds no lock, and two tenants can both sit `linked` against the same store for that window.
First read wins and the loser is blocked; the window is real and should be closed by running the identity read as the last step of the link flow rather than waiting for the first item.

## 5. Reauth

Only an authentication-class failure flips the state; a refresh that merely failed advances a counter (`crates/tam-storage/migrations/0031_connection_custody.sql:32-39`).
`needs_reauth` reads `disconnected` to the seller (`crates/tam-types/src/connection.rs:210-213`), and the connections page already carries the banner telling them queued work is paused and nothing is lost (`web/src/routes/connections/+page.svelte:19,59-64`).
In-flight items park on the gate `ReauthRequired`, which is deliberately excluded from the time-gated revivable set because only a re-link clears it (`crates/tam-storage/src/jobs.rs:468-478`).

Re-linking is the same Connect action, against the existing connection row.
`LeaseRepo::revive_expired`'s re-link arm is conditioned on the connection rather than on the clock, and in one data-modifying CTE against a single snapshot it settles the stranded `write_attempt` `abandoned` and requeues the item (`crates/tam-storage/src/jobs.rs:1163-1236`).
Creates are excluded and stay parked on purpose: settling a create's attempt releases the only fence against a second live listing, and neither adapter offers an idempotent create (`crates/tam-storage/src/jobs.rs:1178-1186`).
So the seller who re-links sees revisions and removals resume and creates stay stuck, and the resolution those need — a read-back settling on what is actually on the marketplace — is named in that comment and not built.
That asymmetry must appear in the page copy, because a seller told "work resumed" who then watches a create sit parked will not read it as a fence.

## 6. What ships first

Ship the attended flow as the first seller-facing link, and hand-hold the first sellers if it is not ready, rather than shipping a paste box.
A guided cookie capture undercuts trust in a way that is concrete rather than aesthetic: it teaches sellers that pasting marketplace session tokens into a web form is a normal thing this product asks for, which is exactly the habit every credential-phishing attack against them needs, and it is not a habit that can be un-taught later by removing the box.
The operator path already exists, contacts no marketplace and is the right instrument for the first customers: `link-from-jar` reads a jar and seals it without any network call (`crates/tam-session-broker/src/main.rs:188-196`).
Hand-held onboarding for a handful of sellers is honest, reversible, and costs a support conversation; a public paste box costs the security posture the rest of this design is built to hold.

## 7. Deliberately out

No route returns session material, ever: a lease answers with a loopback endpoint, a bearer token and an expiry and nothing else (`crates/tam-session-broker/src/protocol.rs:126-134`), and the gateway never relays `set-cookie` back to a worker (`crates/tam-session-broker/src/gateway.rs:184-190`).
No credential export and no credential sharing between organisations; co-authorship is display metadata and an internal ledger entry, never a claim on another org's connection.
No third-party captcha solver, no browserless cloud, no remote-browser service, and no Enterprise image whose solving may ship page content off-host while a seller password is in the DOM.
No storing of the marketplace password by default, per Q2 below.
No seller-typed account identifier ever taking the exclusivity lock.
No per-user connection ownership and no `membership` table in this change: the org stays the tenancy boundary, and multi-affiliation is deferred with its own note.
No automated Tes login while `/authn/` sits under a robots disallow and no founder ruling exists.
No password reset, no second-factor enrolment, and no account administration performed on the seller's behalf; the gateway allow-list makes roster, account-administration and payout routes structurally unreachable and that stays true of the link path.

## 8. Founder questions

1. Captcha posture: when a marketplace login presents a challenge, do we hand it to the seller live in an attended browser, fail the link and say so, or something else — and is a third-party solving service excluded absolutely?
   Recommended: attended, and excluded absolutely, in any form including a self-hosted image that may transmit page content off-host.
2. Do we store the marketplace password after a successful sign-in, or use it and discard it?
   Recommended: use and discard — a stored session that expires is a smaller loss than a stored password that opens payouts and bank details, and Tes renewal is proven so re-links should be rare.
3. Is a browser process re-admitted to the workspace for the broker's link step only, per `decisions.md:154`'s evidence requirement?
   Recommended: yes, as a broker-launched chromiumoxide driver killed per attempt, and only after the login probe reports.
4. Tes automated login against the robots disallow on `/authn/`.
   Recommended: do not attempt it; attended sign-in puts a human at the gate and the question does not arise.
5. Activate Tes exclusivity, which requires adding `/api/tier` to the gateway allow-list?
   Recommended: yes — the one-account-one-seller line is currently unenforced for Tes, and this is the smallest change that enforces it.
6. Does the first seller-facing link ship as the attended flow only, with hand-held onboarding as the gap-filler, and no paste box at any point?
   Recommended: yes.
