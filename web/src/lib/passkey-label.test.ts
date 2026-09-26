import { describe, expect, it } from 'vitest';
import { passkeyLabel, passkeyReach } from './passkey-label';

describe('the passkey label', () => {
	it('uses the stored name when there is one', () => {
		expect(passkeyLabel({ name: 'Work laptop' })).toBe('Work laptop');
	});

	it('trims the stored name rather than rendering its padding', () => {
		expect(passkeyLabel({ name: '  Work laptop \n' })).toBe('Work laptop');
	});

	it('falls back without naming a device it cannot know', () => {
		for (const stored of [undefined, null, '', '   ']) {
			expect(passkeyLabel({ name: stored })).toBe('Unnamed passkey');
		}
	});
});

describe('the passkey reach', () => {
	it('distinguishes a synced credential from one held on a single device', () => {
		expect(passkeyReach({ backedUp: true })).not.toBe(passkeyReach({ backedUp: false }));
		expect(passkeyReach({ backedUp: true })).toMatch(/other devices/i);
		expect(passkeyReach({ backedUp: false })).toMatch(/only on the device/i);
	});

	it('treats an absent backup state as not synced, which is the safe reading', () => {
		expect(passkeyReach({})).toBe(passkeyReach({ backedUp: false }));
	});
});
