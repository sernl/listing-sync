# Probe: Tes session longevity

- date: 2026-08-25 (window open, result pending)
- method: refresh-aware hourly poll of an authenticated endpoint through a cookie jar, via a systemd user timer

## Observation

The clock is running on the founder's own captured session; the logger polls `GET /api/tier/gmv/me`, refreshing through `/api/authn/refresh-cookies` first, and records authed, challenged or expired hourly.
Early rows read authed.

## Answer to the gated question

Pending the multi-week window: whether a captured server-side session stays valid, and whether a one-time-password re-challenge ever fires, which decides the parking TTL and whether sync may be described as unattended.
Until then the product describes sync as scheduled and supervised.

## Confidence

Not yet answerable; the observation window has just opened.
