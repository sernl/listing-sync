import assert from 'node:assert/strict';
import { test } from 'node:test';
import { isAPIError } from 'better-auth/api';
import {
  CONSENT_REFUSAL,
  type Consent,
  type ConsentDeps,
  consentHooks,
  PendingConsents,
  parseConsent,
} from './consent.ts';

const AGREED = { terms_privacy: true, ip_ownership: true, age_18: true, version: '2026-10-04' };
const SINK = { url: 'http://127.0.0.1:8080/internal/consent', secret: 's3cret' };
const WHERE = { ipAddress: '203.0.113.7', userAgent: 'Mozilla/5.0 (test)' };

interface Sent {
  readonly url: string;
  readonly init: RequestInit;
}

const harness = (status = 204, overrides: Partial<ConsentDeps> = {}) => {
  const sent: Sent[] = [];
  const removed: string[] = [];
  const carried: Consent[] = [];
  const hooks = consentHooks({
    sink: SINK,
    pending: new PendingConsents(),
    carry: async (consent) => {
      carried.push(consent);
    },
    carried: async () => carried[0],
    removeUser: async (id) => {
      removed.push(id);
    },
    send: async (url, init) => {
      sent.push({ url, init });
      return new Response(null, { status });
    },
    newId: () => '11111111-1111-4111-8111-111111111111',
    log: () => undefined,
    ...overrides,
  });
  return { hooks, sent, removed, carried };
};

const refusedWith422 = (error: unknown): boolean =>
  isAPIError(error) &&
  error.statusCode === 422 &&
  (error.body as { message?: unknown } | undefined)?.message === CONSENT_REFUSAL;

test('a sign-up with no consent is refused with 422 before anything is created', async () => {
  const { hooks } = harness();
  await assert.rejects(
    hooks.before({ path: '/sign-up/email', body: { email: 'a@example.test' } }),
    refusedWith422,
  );
});

test('a sign-up with a box left unticked is refused with 422', async () => {
  const { hooks } = harness();
  for (const unticked of ['terms_privacy', 'ip_ownership', 'age_18']) {
    await assert.rejects(
      hooks.before({ path: '/sign-up/email', body: { consent: { ...AGREED, [unticked]: false } } }),
      refusedWith422,
      `${unticked} false must refuse`,
    );
  }
  await assert.rejects(
    hooks.before({ path: '/sign-up/email', body: { consent: { ...AGREED, version: 'latest' } } }),
    refusedWith422,
    'a version that is not a date must refuse',
  );
});

test('a ticked sign-up passes the before hook, and other paths are untouched', async () => {
  const { hooks } = harness();
  await hooks.before({ path: '/sign-up/email', body: { consent: AGREED } });
  await hooks.before({ path: '/sign-in/email', body: { email: 'a@example.test' } });
  await hooks.before({ path: '/sign-in/social', body: { provider: 'google' } });
});

test('a social sign-up must carry the boxes, and they ride the OAuth state', async () => {
  const { hooks, carried } = harness();
  await assert.rejects(
    hooks.before({ path: '/sign-in/social', body: { provider: 'google', requestSignUp: true } }),
    refusedWith422,
  );
  await hooks.before({
    path: '/sign-in/social',
    body: { provider: 'google', requestSignUp: true, additionalData: { consent: AGREED } },
  });
  assert.deepEqual(carried, [AGREED]);
});

test('the happy path creates the account under a chosen id and posts the agreement', async () => {
  const { hooks, sent, removed } = harness();
  const created = await hooks.createUser(
    { email: 'a@example.test' },
    { path: '/sign-up/email', body: { consent: AGREED } },
  );
  assert.deepEqual(created, { data: { id: '11111111-1111-4111-8111-111111111111' } });

  const outcome = await hooks.complete('11111111-1111-4111-8111-111111111111', WHERE);
  assert.equal(outcome, 'recorded');
  assert.equal(removed.length, 0);
  assert.equal(sent.length, 1);
  const [call] = sent;
  assert.equal(call?.url, SINK.url);
  assert.equal(call?.init.method, 'POST');
  assert.equal(
    (call?.init.headers as Record<string, string> | undefined)?.['x-tam-internal-secret'],
    SINK.secret,
  );
  assert.deepEqual(JSON.parse(String(call?.init.body)), {
    subject: '11111111-1111-4111-8111-111111111111',
    email: 'a@example.test',
    ip_address: '203.0.113.7',
    user_agent: 'Mozilla/5.0 (test)',
    consent: AGREED,
  });

  assert.equal(
    await hooks.complete('11111111-1111-4111-8111-111111111111', WHERE),
    'not-created',
    'an agreement is written once',
  );
});

test('a callback creates the account only with the agreement the OAuth state carried', async () => {
  const bare = harness();
  assert.equal(
    await bare.hooks.createUser({ email: 'b@example.test' }, { path: '/callback/:id' }),
    false,
  );

  const { hooks, carried, sent } = harness();
  carried.push(AGREED as Consent);
  const created = await hooks.createUser({ email: 'b@example.test' }, { path: '/callback/:id' });
  assert.ok(created !== false && created !== undefined);
  assert.equal(await hooks.complete(created.data.id, WHERE), 'recorded');
  assert.equal(sent.length, 1);
});

test('an account an administrator creates is not a sign-up', async () => {
  const { hooks } = harness();
  assert.equal(await hooks.createUser({ email: 'c@example.test' }, null), undefined);
  assert.equal(
    await hooks.createUser({ email: 'c@example.test' }, { path: '/admin/create-user' }),
    undefined,
  );
});

test('a failed write removes the account it was for', async () => {
  const { hooks, removed } = harness(503);
  const created = await hooks.createUser(
    { email: 'a@example.test' },
    { path: '/sign-up/email', body: { consent: AGREED } },
  );
  assert.ok(created !== false && created !== undefined);
  assert.equal(await hooks.complete(created.data.id, WHERE), 'failed');
  assert.deepEqual(removed, [created.data.id]);
});

test('no sink configured fails closed', async () => {
  const { hooks, removed, sent } = harness(204, { sink: undefined });
  const created = await hooks.createUser(
    { email: 'a@example.test' },
    { path: '/sign-up/email', body: { consent: AGREED } },
  );
  assert.ok(created !== false && created !== undefined);
  assert.equal(await hooks.complete(created.data.id, WHERE), 'failed');
  assert.equal(sent.length, 0);
  assert.deepEqual(removed, [created.data.id]);
});

test('a user the hooks did not create is left alone', async () => {
  const { hooks, sent } = harness();
  assert.equal(await hooks.complete('22222222-2222-4222-8222-222222222222', WHERE), 'not-created');
  assert.equal(await hooks.complete(undefined, WHERE), 'not-created');
  assert.equal(sent.length, 0);
});

test('a pending agreement nothing claimed ages out', () => {
  let now = 0;
  const pending = new PendingConsents(() => now);
  pending.stash('a', AGREED as Consent, undefined);
  now = 11 * 60 * 1000;
  pending.stash('b', AGREED as Consent, undefined);
  assert.equal(pending.size, 1);
  assert.equal(pending.take('a'), undefined);
});

test('the parser accepts only all three boxes and a date', () => {
  assert.deepEqual(parseConsent(AGREED), AGREED);
  assert.equal(parseConsent(undefined), undefined);
  assert.equal(parseConsent({ ...AGREED, age_18: 'true' }), undefined);
});
