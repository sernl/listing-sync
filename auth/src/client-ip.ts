import type { BetterAuthOptions } from 'better-auth';

type IpAddressOptions = NonNullable<NonNullable<BetterAuthOptions['advanced']>['ipAddress']>;

/**
 * Where better-auth reads the client's address from: the rate limiter's bucket
 * key, the session row's `ip_address`, and the audit row's (through `getIP`).
 *
 * `cf-connecting-ip` comes first because production sits behind Cloudflare and
 * then Caddy. By the time a request reaches tam-auth, `x-forwarded-for` holds
 * the Cloudflare edge that relayed it, not the teacher. Every teacher routed
 * through one edge then shared one sign-in bucket (3 requests per 10 s), so one
 * busy edge refused sign-in for everyone behind it. Cloudflare sets
 * `cf-connecting-ip` to the one address that opened the connection to it and
 * overwrites any value the client sent, so it names the real client.
 *
 * `x-forwarded-for` stays as the fallback for requests that never crossed
 * Cloudflare: the vite dev server, the local stack, and an in-cluster caller.
 * This ordering trusts `cf-connecting-ip` only as far as the origin is
 * reachable solely through Cloudflare; a path that skips the edge could set it.
 */
export const ipAddress = {
  ipAddressHeaders: ['cf-connecting-ip', 'x-forwarded-for'],
} satisfies IpAddressOptions;
