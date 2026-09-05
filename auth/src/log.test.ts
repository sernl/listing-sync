import assert from 'node:assert/strict';
import { test } from 'node:test';
import { transportWarning } from './log.ts';

const SUBJECT = 'Reset your Teachouse password';
const LINK = 'https://teachouse.example/reset-password?token=SECRET-TOKEN';

test('production names the message and never the token', () => {
  const line = transportWarning(SUBJECT, LINK, 'production');
  assert.ok(line.includes(SUBJECT), 'the line must say which message was dropped');
  assert.ok(!line.includes('SECRET-TOKEN'), 'a reset token reached the log');
  assert.ok(!line.includes('teachouse.example'), 'the link reached the log');
});

test('development prints the link, which is the only place development can get it', () => {
  const line = transportWarning(SUBJECT, LINK, 'development');
  assert.ok(line.includes(SUBJECT));
  assert.ok(line.includes(LINK), 'development lost its only route to the flow');
});

test('no mode prints the recipient, because no mode is given it', () => {
  assert.ok(!transportWarning(SUBJECT, LINK, 'development').includes('@'));
  assert.ok(!transportWarning(SUBJECT, LINK, 'production').includes('@'));
});
