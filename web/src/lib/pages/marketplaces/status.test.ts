import { describe, expect, it } from 'vitest';
import type { DeviceView } from '$lib/api';
import { statusEvents } from './status';

const device = (name: string, revoked: number | null, used: number): DeviceView => ({
	id: name,
	name,
	os: 'linux',
	arch: 'x86_64',
	app_version: '0.12.0',
	first_seen_at: 0,
	last_seen_at: used,
	revoked_at: revoked,
	wipe_outstanding: false,
	runs_sourced_payloads: true,
	sessions: [
		{ marketplace: 'Tpt', account_label: null, linked_at: 10, last_used_at: used, status: 'connected' },
		{ marketplace: 'Tes', account_label: null, linked_at: 20, last_used_at: 90, status: 'signed_out' }
	]
});

describe('the recent events on the status page', () => {
	it('lists pauses and sessions newest first, skipping signed-out machines and ended sessions', () => {
		const events = statusEvents(
			[
				{ inventory: 'Tpt', marketplace: 'Tpt', halted: true, reason: 'TPT is down', raised_at: 50 },
				{ inventory: 'Tes', marketplace: 'Tes', halted: false }
			],
			[device('Laptop', null, 40), device('Old PC', 5, 99)]
		);
		expect(events.map((event) => [event.at, event.kind, event.what])).toEqual([
			[50, 'paused', 'Sending paused: TPT is down'],
			[40, 'used', 'Used by the app on Laptop'],
			[10, 'signed_in', 'Signed in on Laptop']
		]);
	});

	it('keeps only the newest few', () => {
		const devices = Array.from({ length: 6 }, (_, n) => device(`M${n}`, null, 100 + n));
		expect(statusEvents([], devices, 3).map((event) => event.at)).toEqual([105, 104, 103]);
	});
});
