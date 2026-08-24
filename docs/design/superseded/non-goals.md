# Non-goals

Superseded on 2026-08-25 by the non-goals section of [`2026-08-25-listing-sync-design.md`](2026-08-25-listing-sync-design.md).
This document is retained for its reasoning and is not live; do not implement from it.

A deferral is about timing and a non-goal is a commitment.
The items below are written down so they cannot return as a growth idea in month nine, when the pressure to add one more feature is highest and the person who understood why it was excluded is the same person under the pressure.
The deferred list lives with the milestone plan; nothing here moves onto it.

## The surfaces deliberately cut

There is no node-graph mapping canvas.
The primary mapping surface is a virtualised product-by-marketplace table, which is what every incumbent in the adjacent feed-management category ships and what actually supports search, bulk selection and the query a seller genuinely asks, which is show me everything that failed on Tes.
The canvas loses on three counts at once: two thousand products across five destinations is roughly two thousand nodes and ten thousand edges converging on five points, where the leading library's own stress test tops out at 625 nodes and a maintainer has said it is not intended for that scale; the pricing for commercial use could not be established because the vendor's pricing page returns 404; and it is unusable on a phone, which is where the mobile client has to work.

There is no public developer API until a real third-party consumer exists and asks for one.
The internal API is built properly regardless, because that costs almost nothing and is unpleasant to retrofit: a nested versioned router, an OpenAPI document, idempotency keys on every request that starts a sync, long-running work modelled as an operation resource rather than a blocking call, and cursor pagination.
What does not exist is a published contract, per-tier metering, rate limiting by plan or generated SDKs, and none of that gets harder by waiting.

There are no desktop clients.
The server-side architecture removes the need for one, and shipping one would reintroduce a recurring cost the product has no reason to carry: an annual developer-programme membership and a code-signing certificate whose private key cannot be copied into a build secret, paid native runners for two operating systems, a Mac the founder must own to debug one of the three webview engines, and, for a sole trader outside North America, a company-formation decision forced by one signing service's location requirement.
The web client is first and an Android client is second, and mobile is a full client rather than a read-only one only because the client performs no automation.

## The permanent refusals

The product never enumerates or bulk-extracts marketplace data.
This is the sharpest line in the design and it is enforced in types rather than in a policy document: a marketplace read is constructible only from a first-party export the marketplace itself provisions for the seller, or from a receipt proving an authorised write just landed, so link-following, listing pages, search and pagination have no representation in the code.
The consequences are deliberate and they are not small.
There is no competitor intelligence, no category-wide analytics, no keyword or search-volume research derived from marketplace data, no price monitoring of other sellers, and no ranking of anything.
Analytics exists, and it is built strictly from data the seller already owns and can export themselves.

Related and equally permanent: the product never touches another seller's or another buyer's assets.
It reads, ranks, messages, follows, reviews and scrapes nothing belonging to anyone but the account holder whose session it is acting in.
That line, rather than visibility or volume, is what predicts which tools in adjacent markets were killed by platform attention: the ones that automated actions against other users were killed, and the ones that published content on the account holder's own assets were not.
It belongs in the public product description and it is never crossed.

The product never manipulates an identifier to disguise its origin.
No spoofed user agent, no residential proxying, no fingerprint masking, no datacentre-IP concealment, and no shared identity fronting for software.
Egress is fixed and declared, the user agent names the product and carries a contact address, and where a real browser engine is used it is not disguised as a human-driven one.
The rule generalises usefully and is worth stating in that form: the engine may never present a value about itself that it does not believe to be true.

There is no CAPTCHA-solving integration under any circumstances, which also rules out the managed browser providers that meter solves as a billable feature.

The product never requests mailbox access.
Not IMAP, not a mail-provider OAuth scope, not for one-time-password automation and not for anything else, because mailbox access confers password reset on every service the seller uses.
Relaying a one-time password is always a human step: the seller reads the code from their own inbox and types it into one narrow field.

Destructive operations are absent from the action vocabulary in phase one rather than disabled.
There is no delete verb for the interpreter to reach, so no configuration, valid or forged, can express one.
Deletion is permanent on at least one target marketplace and invisible to that marketplace's own abuse signals, so the loss falls entirely on the seller.

The product never captures page content from a marketplace.
No DOM snapshots, no session replay, not masked, not opt-in, not only on failure.
The objection is contractual before it is a privacy question: a page snapshot uploaded to our servers reproduces a third party's user interface and computer code onto another computer for a commercial purpose, which is squarely inside the clause that matters most, and masking does not rescue it because the retained structure is the expression the clause protects.
Diagnostics are structural instead: client version, adapter and step identity, a closed failure code, matched-node counts bucketed to none, one or many, a fixed vector of expected-anchor booleans, timings, and a URL reduced to origin plus route template.
This one will be proposed again by whoever is debugging at two in the morning, which is why it is written here with the reason that holds rather than the privacy argument that does not.

Sync is never agent-driven.
It is deterministic and cron-scheduled.
Large language models are confined to generating listing copy and to rediscovering a selector after a marketplace changes its markup, and no model decides what to sync, whether a sync succeeded, or how to recover.
A model may not author a taxonomy edge either, because a wrong edge is invisible, durable, and applies to every future product carrying that term, which is a different risk from a wrong sentence the seller reads before accepting it.

Nothing publishes without a diff the seller can see.
Dry run is the default publish mode, every generated field is a proposal with a field-level diff, and auto-publish is earned per seller, per field, per inventory after a run of clean accepts.

## What the product is not

It is not a marketplace and it never holds a buyer relationship.
It is not a payment intermediary for the seller's own sales; the only money it touches is its own subscription.
It is not an accounting or tax product, and reconciling a seller's earnings across channels is a reporting surface over their own exports, not a ledger the product warrants.
It is not a content generator sold on the strength of its generation, because that capability is already sold to this persona as a one-time purchase and pricing against it is a losing position.
It does not host, sell, or take a share of the seller's resources.

One go-to-market commitment belongs here rather than in a marketing plan, because breaking it is a violation by the account holder who breaks it.
The product is never promoted inside a marketplace's own seller forum, never marketed to sellers using contact details obtained through a marketplace, and never sold against a seller list assembled by scraping one.
Everything off-platform is expressly permitted and is where all of it happens.
