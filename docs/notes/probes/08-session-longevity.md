# Probe: Tes session longevity

- date: 2026-08-25 (window opened); 2026-09-19 (answered by the TPT→TES acceptance trial)
- method: refresh-aware hourly poll of an authenticated endpoint through a cookie jar, via a systemd user timer

## Observation

The clock ran on the founder's own captured session; the logger polls `GET /api/tier/gmv/me`, refreshing through `/api/authn/refresh-cookies` first, and records authed, challenged or expired hourly.
The refresh-aware poll kept reading authed.

The acceptance trial then ran the same session through the device, which was not refresh-aware: it captured 14 cookies at 08:12 UTC, sent that snapshot on every call, and from 12:15 every Tes call answered as a lapsed session — about four hours.
The two observations together are the answer, and the difference between them is the whole of it: the probe persisted Tes's `Set-Cookie` with `curl -b/-c` and called the refresh route; the device discarded every `Set-Cookie` at the transport seam and never called it.

## Answer to the gated question

A captured Tes session stays valid indefinitely *if it is refreshed*, and lapses in about four hours if it is not.
No one-time-password re-challenge fired in either window, so the re-challenge half of the question remains unobserved rather than answered.

The parking TTL therefore rests on the refresh rather than on the capture, and the device now performs it: the live transport holds the cookies and merges Tes's rotations into them, the stored jar is rewritten from the live client after every session-routed request, and the five-minute check-in renews and re-proves each held session.
A session nobody has proven in a quarter of an hour is reported as signed out rather than connected, which is what stops the device claiming work it cannot settle.

Sync may be described as scheduled and supervised, as before; what has changed is that an unattended stretch no longer silently ends the session.

## Confidence

High on the four-hour unrefreshed lifetime and on the refresh keeping a session alive — both observed directly, on the same account, by two independent methods.
Unknown on the re-challenge, which neither window provoked.
