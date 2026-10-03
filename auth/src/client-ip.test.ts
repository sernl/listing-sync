import assert from 'node:assert/strict';
import { test } from 'node:test';
import { getIP } from 'better-auth/api';
import { ipAddress } from './client-ip.ts';

const resolve = (headers: Record<string, string>): string | null =>
  getIP(new Headers(headers), { advanced: { ipAddress } });

test('a request through Cloudflare resolves to the address Cloudflare saw', () => {
  assert.equal(
    resolve({ 'cf-connecting-ip': '203.0.113.7', 'x-forwarded-for': '172.68.10.20' }),
    '203.0.113.7',
  );
});

test('two teachers behind one Cloudflare edge land in different buckets', () => {
  const edge = { 'x-forwarded-for': '172.68.10.20' };
  assert.notEqual(
    resolve({ ...edge, 'cf-connecting-ip': '203.0.113.7' }),
    resolve({ ...edge, 'cf-connecting-ip': '198.51.100.9' }),
  );
});

test('a request that never crossed Cloudflare falls back to x-forwarded-for', () => {
  assert.equal(resolve({ 'x-forwarded-for': '192.0.2.44' }), '192.0.2.44');
});

test('a malformed cf-connecting-ip falls through to x-forwarded-for', () => {
  assert.equal(
    resolve({ 'cf-connecting-ip': 'not-an-address', 'x-forwarded-for': '192.0.2.44' }),
    '192.0.2.44',
  );
});
