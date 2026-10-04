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
// keeps better-auth's own default, which for Microsoft is unverified. Both
// must also refuse to create an account implicitly, or a sign-in from the
// sign-in page would make an account nobody agreed to the terms for.
test('both social providers hand their profile through the mapper and never sign up implicitly', () => {
  const source = readFileSync(join(dirname(fileURLToPath(import.meta.url)), 'auth.ts'), 'utf8');
  for (const provider of ['google', 'microsoft']) {
    const wiring = new RegExp(
      `${provider}: \\{\\s*\\.\\.\\.env\\.${provider},\\s*mapProfileToUser: vouchedByProvider,\\s*disableImplicitSignUp: true,?\\s*\\}`,
      'u',
    );
    assert.ok(
      wiring.test(source),
      `the ${provider} provider must map its profile through vouchedByProvider and disable implicit sign-up`,
    );
  }
});
