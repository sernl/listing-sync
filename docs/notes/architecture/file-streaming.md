# Opening an imported file: the device-served stream

Status: shipped in 0.16.0. Replaces the 0.15.0 server copy.
Why: `docs/notes/design/research/2026-09-29-file-custody.md`.
Code: `crates/tam-api/src/broker.rs`, `crates/tam-storage/src/device_stream.rs`, migration `0099_device_streams.sql`, device side `apps/desktop/src-tauri/src/serve.rs` (+ `android_serve.rs`), shared claims `crates/tam-domain/src/serve.rs`.

## The rule

A file imported from a marketplace never rests on Teachouse's servers. It lives on the seller's devices.
A file the seller uploads through the console is stored by Teachouse (sealed blob) and is served from the blob store as before.
`FileBytes::Sourced` versus `FileBytes::Held` is the line, and the content routes branch on it.

## Flow

```mermaid
sequenceDiagram
    participant B as Browser
    participant A as tam-server pod A
    participant P as Postgres
    participant C as tam-server pod B
    participant D as Seller's device
    D->>C: GET /v1/devices/{d}/streams?wait_ms=25000 (long poll)
    B->>A: GET /v1/products/{p}/files/{f}/content (Range)
    A->>P: holders of hash; pick one polled < 60 s ago
    A->>P: INSERT device_stream (ask + capability + pod=A) ; NOTIFY device_stream
    P-->>C: notification "org:device"
    C->>P: claim unclaimed asks for d
    C-->>D: {requests:[{stream, hash, first, last, capability}]}
    D->>D: verify capability, open sealed library, cut [first,last]
    D->>C: POST /v1/devices/{d}/streams/{s} (bytes)
    C->>A: same POST, x-teachouse-forwarded (pod=A ≠ C)
    A-->>B: 206 + bytes, as they arrive
    A->>P: DELETE device_stream row
```

- **Why long poll.** Devices sit behind NAT; the server cannot dial them. The device's own outbound HTTPS is the only channel that always exists, through Cloudflare, on every platform. The desktop app's iroh endpoint has relays disabled, so it cannot carry this either.
- **Serving vs online.** `device.stream_polled_at` is stamped on each poll. A holder that polled within 60 s is *serving*; the broker asks the most recent one. If none is serving, the answer is `409 device_offline` naming the holder seen most recently, straight away, without waiting. `last_seen_at` stays the heartbeat's alone because it measures whether a revocation reached the device.
- **Windows.** One ask covers at most 32 MiB (`WINDOW_BYTES`). Cloudflare refuses request bodies over 100 MB, and a device's answer is one request body. A whole-file download of a larger file is a series of asks, opened as each one finishes. pdf.js range reads (64 KiB) each fit in one window.
- **Range.** The broker answers `Range` against the length the catalogue records (`observed_byte_len`), so `Content-Range` and `Content-Length` are known before the device sends a byte, and pdf.js can page.
- **Memory and backpressure.** Between the device's upload and the browser's download sits a `tokio::mpsc` channel of 8 frames. When the browser reads slowly, the upload handler waits on `send`, hyper stops reading the device's socket, and TCP pushes back on the device. A process holds at most 64 asks (`STREAMS_MAX`); more get a 503.
- **Exactness.** The device must send exactly `last-first+1` bytes. Too many or too few ends the browser's body with an error, so the browser sees a broken download rather than a wrong file, and the device gets a 422. The digest can't be checked on a range. The device reads from its own sealed library, which is keyed and verified by digest.
- **Timeouts.** Pickup is 15 s (from writing the ask to the device starting its answer), then `409 device_offline`. A gap of 30 s between frames ends the body. Both are `Config::broker_timeouts`.
- **Probe.** `?probe=1` answers `204` when a holder is serving, and otherwise gives the error a read would get. The console probes before handing a URL to pdf.js or a download link, which would otherwise swallow the sentence.

## Two server processes

tam-server runs two replicas. The browser's request and the device's poll or answer can land on different pods.

- **Wake-up.** Asks are written to `device_stream` and announced with `NOTIFY device_stream, '<org>:<device>'`. Every pod `LISTEN`s once per database, and a waiting poll also looks again every 5 s in case a notification was lost.
- **Bytes.** The ask records the address of the pod holding the browser (`--broker-advertise`, the pod IP from the downward API). An answer that lands on the other pod is streamed to that address with `x-teachouse-forwarded: 1`, and is never forwarded twice. Without `--broker-advertise`, an answer landing on the wrong pod gets `410` and the browser sees the device as offline. Only a single-replica deployment may leave the flag out.

## Authority

- The device authenticates to the server with its own session, as on every other device call.
- The server authorises the device's answer with a capability: an EdDSA JWS signed with the entitlement key. Its claims are `tam_domain::serve::Claims`: org, device, stream id, file digest, first and last byte, expiry after 120 s, and audience `tam-desktop/serve`, so it can't be replayed as an entitlement token or the other way round. The device checks it against its embedded keys, refuses a stream id it has already answered, and refuses anything that doesn't match the request exactly. The server takes an answer only if its session is the ask's org, its path names the ask's device, and its capability header equals the one issued.
- No seller credential leaves the device. The server holds no marketplace session and never sees the device's library key.

## What is stored

- `device_stream`: the ask's metadata (file digest, byte range, capability, pod, times), deleted when the answer ends or the pickup times out, and useless after `expires_at`. It has RLS like every tenant table.
- Nothing else. No byte of an imported file is written to Postgres, the blob store or a disk on the server.

## Limits

- The seller's device must be on with the app open. Android keeps answering with the screen off only while its foreground service runs, which is until 10 minutes after the app was last used or last served a file (`android-client.md`).
- Throughput is the device's upload speed. A 60 MB download from a phone on mobile data is slow.
- Clients older than 0.16.0 don't poll for streams, so their files show as offline.
