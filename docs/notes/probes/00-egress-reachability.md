# Probe: egress reachability

- date: 2026-08-25
- egress: local host, public IP 47.72.82.42, ASN "One New Zealand Group Limited" (consumer ISP, New Zealand)
- account: unauthenticated / public
- method: `probes/reachability.sh` against Tes and TPT, plain and browser user agents

## Observation

Tes and TPT both return HTTP 200 to a plain `curl` from this consumer-ISP egress.

```
https://www.tes.com/teaching-resources                 200  curl/8.0
https://www.teacherspayteachers.com/                   200  curl/8.0
https://www.teacherspayteachers.com/                   200  Mozilla/5.0 ... Chrome/151.0.0.0
egress: {"ip":"47.72.82.42","asn_org":"One New Zealand Group Limited","country":"New Zealand"}
```

Earlier research measured TPT returning HTTP 403 from AI-vendor datacentre infrastructure, and TPT's `robots.txt` names `GPTBot`, `meta-externalagent`, `CCBot`, `ImagesiftBot` and `Applebot-Extended` in explicit stanzas.
Read together, the 403 was an AI-vendor block by user-agent, not a generic datacentre block.

## Answer to the gated question

Egress reachability is settled for the current consumer-ISP host: both marketplaces serve it.
The remaining question is conditional and unowned until the box moves: if the eventual production host sits in a commercial datacentre, rerun `probes/reachability.sh` against TPT before relying on it, because a datacentre ASN may be treated differently from a residential one.
Egress is fixed and declared in either case.

## Confidence

High for the current host, which is directly measured and reproduced across two user agents.
The datacentre conditional is explicitly unmeasured and flagged for re-probe.
