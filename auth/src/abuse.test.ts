import assert from 'node:assert/strict';
import { test } from 'node:test';
import { isAPIError } from 'better-auth/api';
import {
  type AbuseDeps,
  DISPOSABLE_EMAIL_REFUSAL,
  defaultScreenUrl,
  IP_VELOCITY_REFUSAL,
  SUSPENDED_REFUSAL,
  screenNewUser,
  screenSignUp,
} from './abuse.ts';

const SINK = { url: 'http://127.0.0.1:8080/internal/abuse/screen', secret: 's3cret' };
const WHO = { email: 'a@example.test', ipAddress: '203.0.113.7' };

interface Sent {
  readonly url: string;
  readonly init: RequestInit;
}

const refusal = (message: string, kind: string): Response =>
  Response.json(
    { errors: [{ message, code: 'signup_refused', detail: { refusal: kind } }] },
    { status: 422 },
  );

const harness = (answer: () => Promise<Response>, overrides: Partial<AbuseDeps> = {}) => {
  const sent: Sent[] = [];
  const logged: string[] = [];
  const deps: AbuseDeps = {
    sink: SINK,
    send: async (url, init) => {
      sent.push({ url, init });
      return answer();
    },
    log: (message) => {
      logged.push(message);
    },
    ...overrides,
  };
  return { deps, sent, logged };
};

const refusedWith = (message: string) => (error: unknown): boolean =>
  isAPIError(error) &&
  error.statusCode === 422 &&
  error.body?.code === 'SIGNUP_REFUSED' &&
  error.body.message === message;

test('a 204 lets the sign-up through without a word in the log', async () => {
  const { deps, logged } = harness(async () => new Response(null, { status: 204 }));
  await screenSignUp(deps, WHO);
  assert.deepEqual(logged, []);
});

test('a throwaway email domain is refused with the sentence tam-server served', async () => {
  const { deps } = harness(async () => refusal(DISPOSABLE_EMAIL_REFUSAL, 'disposable_email'));
  await assert.rejects(screenSignUp(deps, WHO), refusedWith(DISPOSABLE_EMAIL_REFUSAL));
});

test('too many accounts from one network is refused with its own sentence', async () => {
  const { deps } = harness(async () => refusal(IP_VELOCITY_REFUSAL, 'ip_velocity'));
  await assert.rejects(screenSignUp(deps, WHO), refusedWith(IP_VELOCITY_REFUSAL));
});

test('a banned identity is refused as suspended', async () => {
  const { deps } = harness(async () => refusal(SUSPENDED_REFUSAL, 'suspended'));
  await assert.rejects(screenSignUp(deps, WHO), refusedWith(SUSPENDED_REFUSAL));
});

test('a 422 whose body cannot be read still refuses, with the email sentence', async () => {
  const { deps } = harness(async () => new Response('not json', { status: 422 }));
  await assert.rejects(screenSignUp(deps, WHO), refusedWith(DISPOSABLE_EMAIL_REFUSAL));
});

test('a 404 lets the sign-up through and logs it once', async () => {
  const { deps, logged } = harness(async () => new Response(null, { status: 404 }));
  await screenSignUp(deps, WHO);
  assert.equal(logged.length, 1);
  assert.match(logged[0] ?? '', /404/u);
});

test('a 503 lets the sign-up through and logs it once', async () => {
  const { deps, logged } = harness(async () => new Response(null, { status: 503 }));
  await screenSignUp(deps, WHO);
  assert.equal(logged.length, 1);
});

test('an unreachable screen lets the sign-up through and logs it once', async () => {
  const { deps, logged } = harness(async () => {
    throw new TypeError('fetch failed');
  });
  await screenSignUp(deps, WHO);
  assert.equal(logged.length, 1);
});

test('no screen configured lets the sign-up through, logs it, and sends nothing', async () => {
  const { deps, sent, logged } = harness(async () => new Response(null, { status: 204 }), {
    sink: undefined,
  });
  await screenSignUp(deps, WHO);
  assert.equal(sent.length, 0);
  assert.equal(logged.length, 1);
});

test('the request carries the secret and the email, with no address as null', async () => {
  const { deps, sent } = harness(async () => new Response(null, { status: 204 }));
  await screenSignUp(deps, { email: 'b@example.test', ipAddress: undefined });
  assert.equal(sent.length, 1);
  const [request] = sent;
  assert.ok(request !== undefined);
  assert.equal(request.url, SINK.url);
  assert.equal(request.init.method, 'POST');
  const headers = new Headers(request.init.headers);
  assert.equal(headers.get('x-tam-internal-secret'), 's3cret');
  assert.equal(headers.get('content-type'), 'application/json');
  assert.deepEqual(JSON.parse(String(request.init.body)), {
    email: 'b@example.test',
    ip_address: null,
  });
});

test('the create hook screens sign-ups with the client address', async () => {
  const { deps, sent } = harness(async () => new Response(null, { status: 204 }));
  for (const path of ['/sign-up/email', '/sign-in/social', '/callback/:id']) {
    await screenNewUser(deps, { email: 'a@example.test' }, { path, ipAddress: '203.0.113.7' });
  }
  assert.equal(sent.length, 3);
  assert.deepEqual(JSON.parse(String(sent[0]?.init.body)), {
    email: 'a@example.test',
    ip_address: '203.0.113.7',
  });
});

test('the create hook refuses a sign-up the screen refused', async () => {
  const { deps } = harness(async () => refusal(IP_VELOCITY_REFUSAL, 'ip_velocity'));
  await assert.rejects(
    screenNewUser(
      deps,
      { email: 'a@example.test' },
      { path: '/sign-up/email', ipAddress: undefined },
    ),
    refusedWith(IP_VELOCITY_REFUSAL),
  );
});

test('an account an administrator creates is not screened', async () => {
  const { deps, sent } = harness(async () => refusal(SUSPENDED_REFUSAL, 'suspended'));
  const user = { email: 'c@example.test' };
  await screenNewUser(deps, user, null);
  await screenNewUser(deps, user, { path: '/admin/create-user', ipAddress: undefined });
  await screenNewUser(deps, user, { path: undefined, ipAddress: undefined });
  assert.equal(sent.length, 0);
});

test('the screen URL defaults to the consent URL beside it', () => {
  assert.equal(
    defaultScreenUrl('http://127.0.0.1:8080/internal/consent'),
    'http://127.0.0.1:8080/internal/abuse/screen',
  );
  assert.equal(defaultScreenUrl('http://tam:8080/'), 'http://tam:8080/internal/abuse/screen');
  assert.equal(defaultScreenUrl(undefined), undefined);
});
