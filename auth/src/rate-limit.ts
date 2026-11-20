import type { BetterAuthOptions } from 'better-auth';

type RateLimitOptions = NonNullable<BetterAuthOptions['rateLimit']>;

/**
 * better-auth's rate limiter, as tam-auth runs it.
 *
 * `/ok` is exempt because it is the health route. Kubernetes probes it every
 * few seconds from the kubelet, with no `cf-connecting-ip` or
 * `x-forwarded-for`, so every probe from every node lands in the one shared
 * no-client-address bucket. The database store restarts the window on each
 * allowed request, so a steady stream of probes never lets the count lapse:
 * it climbs to the limit and stays there, and the kubelet then restarts a
 * healthy pod on 429 (better-auth 1.7.2, `api/rate-limiter`). `/ok` reads
 * nothing and writes nothing, so leaving it unthrottled costs nothing a
 * throttle would protect. Every other path keeps the default rules.
 */
export const rateLimit = {
  enabled: true,
  storage: 'database',
  customRules: { '/ok': false },
} satisfies RateLimitOptions;
