import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { vouchedByProvider } from './provider-profile.ts';

test('an address the provider supplied is verified, whatever the provider said of it', () => {
  assert.deepEqual(vouchedByProvider({ email: 'sam@example.test' }), { emailVerified: true });
});

test('no address is nothing to vouch for', () => {
  assert.deepEqual(vouchedByProvider({}), { emailVerified: false });
  assert.deepEqual(vouchedByProvider({ email: '' }), { emailVerified: false });
  assert.deepEqual(vouchedByProvider({ email: '   ' }), { emailVerified: false });
});

// auth.ts opens a database pool at import, so the wiring is read as text
// rather than imported. Both providers must name the mapper: one that does not
// keeps better-auth's own default, which for Microsoft is unverified.
test('both social providers hand their profile through the mapper', () => {
  const source = readFileSync(join(dirname(fileURLToPath(import.meta.url)), 'auth.ts'), 'utf8');
  for (const provider of ['google', 'microsoft']) {
    assert.ok(
      source.includes(`${provider}: { ...env.${provider}, mapProfileToUser: vouchedByProvider }`),
      `the ${provider} provider must map its profile through vouchedByProvider`,
    );
  }
});
