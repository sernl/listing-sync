# Abuse prevention

Status: shipped in 0.21.0 (migrations 0105, 0106). Owner: founder (operator decisions), platform (rules).

## The problem

The free plan gives five lifetime moves. The farming loop: make a free account, connect a TPT or Tes shop, spend the five moves, unlink, make the next account, repeat. Related abuse: throwaway emails, bursts of sign-ups, and a seller who is warned or limited coming back under a new account.

## What other SaaS do (research, 2026-10-05)

| Approach | What it is | Fits us? |
|---|---|---|
| Identity-anchored allowance | Key the free allowance to an external identity that is costly to multiply (a card, a phone, here the marketplace shop) rather than to the account. | **Yes, primary control.** The shop is the thing the free moves are spent on. |
| Device / browser fingerprinting | FingerprintJS OSS (40–60% accuracy client-side, BSL licence, not for production use) and ThumbmarkJS (MIT, ~80% uniqueness) hash canvas, audio, fonts and WebGL. [ThumbmarkJS](https://github.com/thumbmarkjs/thumbmarkjs), [ThumbmarkJS vs FingerprintJS](https://www.thumbmarkjs.com/content/thumbmarkjs-vs-fingerprintjs-alternative/) | **No browser fingerprinting.** A script collecting canvas and audio data is the kind of collection IPP 1 asks us to justify, and Safari noise makes it unreliable. We use the desktop app's own install id instead: we already hold it (`device`), and the app is where moves are spent. |
| Disposable-email lists | `disposable-email-domains` (CC0, ~9,200 domains) and `mailchecker` (55k domains). [disposable-email-domains](https://github.com/disposable-email-domains/disposable-email-domains), [mailchecker crate](https://crates.io/crates/mailchecker) | **Yes, bundled.** `crates/tam-api/data/disposable_email_domains.txt`, pinned at upstream commit `db468d42`. Bundled rather than fetched, so sign-up sends nobody's address anywhere. The `disposable_email` / `mailchecker` crates were not taken: one more dependency for a list lookup we can do in a dozen lines. |
| IP reputation / ASN | ipinfo, MaxMind GeoLite2-ASN through the `maxminddb` crate: flag datacenter and VPN ranges, cluster by subnet or ASN. [maxminddb](https://crates.io/crates/maxminddb), [Stripe on velocity by IP/subnet/ASN](https://stripe.com/resources/more/how-to-prevent-free-trial-abuse-in-saas-and-ai-products) | **No.** It is third-party data about our users, and teachers on school and home networks look like everything else. We keep a keyed hash of the exact sign-in address and nothing about where it is. |
| Velocity rules | Limit accounts per IP or per email pattern in a window. Cheap; beaten by VPNs, so a signal rather than a wall. [cside on velocity limits](https://cside.com/blog/signup-shield-multi-account-fraud-detection) | **Yes.** 5 sign-ups per address per day are allowed; the 6th is refused. 3 in a day raises a flag. |
| Graph linking | Accounts sharing a card, device, or address form a cluster. Stripe's card `fingerprint` is stable across customers within one Stripe account. [Stripe card fingerprint](https://dev.to/mihirkanzariya/detecting-affiliate-self-referrals-with-stripes-payment-method-fingerprint-1clj), [Stripe Radar free-trial abuse](https://docs.stripe.com/radar/free-trial-abuse) | **Yes.** The linkage ledger plus the scorer. |

Privacy posture (NZ Privacy Act 2020): collect only what the purpose needs (IPP 1) and keep it no longer than needed (IPP 9). [Privacy Act 2020 IPPs](https://fpf.org/blog/a-deep-dive-into-new-zealands-new-privacy-law-extraterritorial-effect-cross-border-data-transfers-restrictions-and-new-powers-of-the-privacy-commissioner/), [IPP 9 retention](https://sprintlaw.co.nz/articles/privacy-act-2020-in-new-zealand-the-13-information-privacy-principles/). So we read no third-party data and run no fingerprinting script, every signal is a keyed hash, and each kind has a retention period (below). Terms §5 `/terms/#one-account` and Privacy `/privacy/#abuse` say the same.

## The primary control: free moves belong to the shop

`storefront_allowance` (0085) is keyed `(marketplace, platform_account_digest)`. The digest is HMAC-SHA-256 under a KEK-derived pepper over the marketplace's own seller id (TPT store id, Tes seller id) that the desktop app reports on check-in (`tam_secrets::account_digest`). A second account binding the same shop inserts nothing and is credited nothing.

Verified end to end in `crates/tam-storage/tests/abuse.rs::one_shops_free_moves_are_granted_once_whichever_account_names_it`. Three accounts bind one shop through the real check-in path, with unlinks between them. Only the first is credited.

Holes found and their status:

1. **Account deletion re-armed the shop. Fixed.** `erase_organisation` (0092) deletes every row with an `org_id` column, `storefront_allowance` included. A deleted account therefore freed its shop's five for the next account. `storefront_grant_record` (0105) has no `org_id`, so erasure skips it. It records `(marketplace, digest, granted_at)`, is written in the grant's own transaction, and is checked before any credit. It is backfilled from every existing claim.
2. **Unlinking. Not a hole.** The allowance row is never deleted. Unlinking releases the exclusivity lock and nothing else.
3. **Missing digest. Not a hole.** No digest means no grant. TPT reads its store id on the first import; Tes reads it at sign-in; Etsy (OAuth) does not bind through the heartbeat and is never credited free moves.
4. **Verified email. Already required.** The session exchange refuses an identity assertion with `email_verified = false` (`auth.rs`), so no device registers and no shop binds before verification.
5. **Client-asserted seller id. Residual.** The app reports the seller id, so a modified client could report a fresh fake id per account. The moves are still spent against the real shop through the real session. The same install id across those accounts (`shared_device`) and the same sign-in address are what catch it. This is noted here and not engineered further.
6. **Key rotation. Operational.** `ACCOUNT_KEY_VERSION` and the KEK are both inside the digest. Rotating either changes every shop's digest, which makes every shop look new to `storefront_grant_record`. A rotation must re-key `storefront_grant_record` (and `banned_identity` shop rows) in the same change. `docs/notes/runbooks/` has no rotation runbook yet; this paragraph is the requirement.

## Linkage ledger (0105 `account_link_signal`)

One row per `(org_id, kind, value_hash)` with `first_seen` and `last_seen`. Global (no RLS), because the scorer compares across tenants. `org_id` is real, so an erasure deletes the account's signals. Append-only: a trigger refuses any change except moving `last_seen` forward. The backoffice role can read it.

| kind | value (always a 32-byte keyed digest) | written by | links accounts? | retention |
|---|---|---|---|---|
| `shop_digest` | the 0031 shop digest as-is | `device.rs bind_storefront`, inside the bind transaction | yes | life of account |
| `device_fingerprint` | `abuse_digest(kek, "device_fingerprint", device id)` | `POST /devices` (register) | yes | life of account |
| `ip` | digest of `cf-connecting-ip` (fallback `x-forwarded-for`) | session exchange (every sign-in) | yes | 90 days after last seen |
| `user_agent_hash` | digest of the User-Agent | session exchange | no (context only) | 90 days after last seen |
| `email_domain` | digest of the part after `@` | first exchange of a new tenant, from `account_consent` | no (context only) | life of account |
| `payment_fingerprint` | digest of Stripe `payment_method_details.card.fingerprint` | `charge.succeeded` webhook + nightly sweep of `payment_event` | yes | life of account |

`abuse_digest` is HMAC-SHA-256 under `HMAC(KEK, "tam:abuse-signal:v1")` over `kind ‖ 0x1f ‖ value` (`tam-secrets`). A deployment with no KEK records nothing.

## Scorer (0106 `abuse_flag`)

On-event (bind, device register, sign-in, card) for the clusters that event touches, and nightly over everything (`tam-server/src/abuse.rs`). The nightly pass also harvests cards and prunes.

| rule | kind | score |
|---|---|---|
| a shop digest held by ≥ 2 accounts | `shared_shop` | 60 |
| a device id held by ≥ 2 accounts | `shared_device` | 50 |
| an unlink within 7 days of the shop's free-move grant, after spending moves (remaining free moves are zeroed at once) | `quick_unlink` | 50 |
| a card held by ≥ 2 accounts | `shared_payment` | 40 |
| signed up with a throwaway domain (caught after the fact) | `disposable_email` | 40 |
| ≥ 3 accounts whose sign-up address (seen within an hour of creation) matches, within a day of each other | `signup_burst` | 30 |

There is one open flag per `(org, kind)`. Evidence an operator has already decided is not raised again: a flag is raised only if the sharing began after the last decision of that kind. An organisation's score is the sum of its open flags. **A person reviews every flag. Nothing is warned, limited or banned automatically.**

## Enforcement

- **Sign-up screen** (`POST /internal/abuse/screen`, identity service → tam-server, same shared secret as consent). Every self sign-up path (`/sign-up/email`, `/sign-in/social`, `/callback/:id`) is screened in `auth/src/abuse.ts` before the user row is written. The screen answers 422 with one of three refusals: throwaway domain ("Please use a school or personal email address."), a 6th sign-up from one address in 24 h ("Too many accounts were made from this network today. Please try again tomorrow or email contact@teachouse.io."), or a banned email ("This account is suspended. Email contact@teachouse.io."). If tam-server is unreachable, the sign-up is allowed and logged (fail open).
- **Standing** = the action of the organisation's most recently decided flag (`abuse_standing()` SQL function): `none | warn | limit | ban`. An operator decision resolves all of that organisation's open flags.
- **Warn**: a mail to the organisation's people.
- **Limit**: remaining free moves are zeroed (an `operator` ledger debit). Further storefront grants are withheld. No new marketplace connection: the heartbeat re-links an existing `linking`/`needs_reauth` connection but creates or revives nothing else.
- **Ban**: everything a limit does, plus:
  - every `user_session` of the organisation is deleted;
  - every authenticated route and the session exchange answer **403 `account_suspended`** "This account is suspended. Email contact@teachouse.io.";
  - `banned_identity` refuses for 24 months: the members' emails (keyed digest, from `account_consent`), every shop the org claimed or bound, its device ids, and its cards;
  - a banned shop fails the check-in for any account (403);
  - a banned device cannot register;
  - a banned email cannot sign up;
  - a mail is sent.
  `banned_identity` has no `org_id` column, so deleting the account does not lift the ban. Paid move balances are frozen, not deleted (Terms §5).
- **Dismiss** (or Warn/Limit on a banned org) lifts the ban and deletes the identities it wrote.

## Operator surface

`/admin/abuse` (nav after Users) has:
- counters;
- the flagged-orgs table (score, flag kinds, shared signal kinds, linked-org count, standing);
- a cluster sheet (flags, signals with first 12 hex characters and shared-with count, linked orgs);
- Dismiss / Warn / Limit / Ban, where Ban needs a reason and a confirmation;
- search by email (via `account_consent`), IP (digested), `tpt:<store id>` / `tes:<seller id>` (digested like a bind), a ≥ 12-character hex digest prefix, or org name/slug.

Routes:
- `GET /v1/admin/abuse/flags` (backoffice pool)
- `GET /v1/admin/abuse/orgs/{org}` (backoffice pool)
- `POST /v1/admin/abuse/flags/{id}/{action}` (application pool)

## Retention summary

- IP and browser digests: 90 days after last seen.
- Other signals and flags: deleted with the account.
- `banned_identity`: 24 months from the ban, or until lifted.
- `storefront_grant_record`: kept as long as the free plan exists.

The nightly pass enforces the timed limits.

## Refreshing the throwaway list

```
curl -sSfL https://raw.githubusercontent.com/disposable-email-domains/disposable-email-domains/main/disposable_email_blocklist.conf
```
Replace the body of `crates/tam-api/data/disposable_email_domains.txt` with the result and keep the two header lines, updating the commit sha. Then check that no school or mailbox provider (gmail.com, outlook.com, *.school.nz) appeared.
