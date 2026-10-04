import assert from 'node:assert/strict';
import { test } from 'node:test';
import { FRESH_SECONDS, judge, parseProof } from './offboarding.ts';

const SUBJECT = '0b0b0b0b-0b0b-0b0b-0b0b-0b0b0b0b0b0b';
const NOW = new Date('2026-10-04T00:00:00Z');
const ago = (seconds: number): Date => new Date(NOW.getTime() - seconds * 1000);

test('a password account is judged on its password alone', () => {
  const fresh = { userId: SUBJECT, createdAt: ago(10) };
  assert.deepEqual(judge(SUBJECT, { hasPassword: true, passwordMatches: true, session: undefined }, NOW), {
    kind: 'confirmed',
  });
  assert.deepEqual(judge(SUBJECT, { hasPassword: true, passwordMatches: false, session: fresh }, NOW), {
    kind: 'refused',
    status: 403,
    refusal: 'password',
  });
  assert.deepEqual(
    judge(SUBJECT, { hasPassword: true, passwordMatches: undefined, session: fresh }, NOW),
    { kind: 'refused', status: 422, refusal: 'password_required' },
    'a fresh sign-in does not stand in for the password',
  );
});

test('an account with no password needs a sign-in of its own from the last five minutes', () => {
  const judged = (session: { userId: string; createdAt: Date } | undefined) =>
    judge(SUBJECT, { hasPassword: false, passwordMatches: undefined, session }, NOW).kind;
  assert.equal(judged({ userId: SUBJECT, createdAt: ago(FRESH_SECONDS - 1) }), 'confirmed');
  assert.equal(judged({ userId: SUBJECT, createdAt: ago(FRESH_SECONDS) }), 'refused');
  assert.equal(judged({ userId: 'someone-else', createdAt: ago(1) }), 'refused');
  assert.equal(judged(undefined), 'refused');
});

test('a malformed body is an empty proof', () => {
  assert.deepEqual(parseProof(null), { password: undefined, sessionCookie: undefined });
  assert.deepEqual(parseProof({ password: 7, sessionCookie: '' }), {
    password: undefined,
    sessionCookie: undefined,
  });
  assert.deepEqual(parseProof({ password: 'hunter2', sessionCookie: 'a=b' }), {
    password: 'hunter2',
    sessionCookie: 'a=b',
  });
});
