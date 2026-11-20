import assert from 'node:assert/strict';
import { test } from 'node:test';
import { betterAuth } from 'better-auth';
import { ipAddress } from './client-ip.ts';
import { rateLimit } from './rate-limit.ts';

// No `database`, so better-auth runs on its memory adapter, and `storage:
// 'database'` then goes through the same database-backed limiter production
// uses, against that adapter. Requests carry no client address, as a kubelet
// probe does, so they all share the one no-client-address bucket.
const instance = (limits: typeof rateLimit | Omit<typeof rateLimit, 'customRules'>) =>
  betterAuth({
    baseURL: 'http://localhost:3000',
    secret: 'rate-limit-test-secret-of-at-least-32-chars',
    emailAndPassword: { enabled: true },
    advanced: { ipAddress },
    logger: { disabled: true },
    rateLimit: limits,
  });

const statuses = async (
  auth: { handler: (request: Request) => Promise<Response> },
  count: number,
  request: () => Request,
): Promise<number[]> => {
  const seen: number[] = [];
  for (let index = 0; index < count; index += 1) {
    seen.push((await auth.handler(request())).status);
  }
  return seen;
};

const ok = (): Request => new Request('http://localhost:3000/api/auth/ok');

const signIn = (): Request =>
  new Request('http://localhost:3000/api/auth/sign-in/email', {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: 'http://localhost:3000' },
    body: JSON.stringify({ email: 'nobody@example.invalid', password: 'not-the-password' }),
  });

test('a burst of health probes is never answered 429', async () => {
  const seen = await statuses(instance(rateLimit), 250, ok);
  assert.deepEqual([...new Set(seen)], [200]);
});

test('the same burst without the exemption is throttled, so the burst reaches the limiter', async () => {
  const { customRules: _exempt, ...unexempted } = rateLimit;
  const seen = await statuses(instance(unexempted), 250, ok);
  assert.ok(seen.includes(429), `no 429 in ${seen.length} requests`);
});

test('sign-in is still throttled in the same configuration', async () => {
  const seen = await statuses(instance(rateLimit), 5, signIn);
  assert.equal(seen.at(-1), 429);
});
