import assert from 'node:assert/strict';
import { test } from 'node:test';
import { defaultPasskeyRpId } from './rp-id.ts';

test('the console subdomain binds passkeys to the apex', () => {
  assert.equal(defaultPasskeyRpId('dash.teachouse.io'), 'teachouse.io');
  assert.equal(defaultPasskeyRpId('staging.dash.teachouse.io'), 'teachouse.io');
});

test('a host that is already the registrable domain is its own relying party', () => {
  assert.equal(defaultPasskeyRpId('teachouse.io'), 'teachouse.io');
  assert.equal(defaultPasskeyRpId('localhost'), 'localhost');
});

test('an IP address is never shortened', () => {
  assert.equal(defaultPasskeyRpId('127.0.0.1'), '127.0.0.1');
  assert.equal(defaultPasskeyRpId('10.20.30.40'), '10.20.30.40');
  assert.equal(defaultPasskeyRpId('[::1]'), '[::1]');
});
