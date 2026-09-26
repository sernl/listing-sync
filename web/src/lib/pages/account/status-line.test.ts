import { describe, expect, it } from 'vitest';
import type { DeviceView, InventoryStatus } from '$lib/api';
import { statusRows } from './status-line';

const NOW = Date.UTC(2026, 8, 5, 12);

function entry(partial: Partial<InventoryStatus> = {}): InventoryStatus {
	return { inventory: 'Tes', marketplace: 'Tes', halted: false, ...partial };
}

const EVERY: InventoryStatus[] = [
	entry(),
	entry({ inventory: 'Tpt', marketplace: 'Tpt' }),
	entry({ inventory: 'Etsy', marketplace: 'Etsy' })
];

function device(
	marketplace: DeviceView['sessions'][number]['marketplace'],
	last_used_at: number,
	partial: Partial<DeviceView> = {}
): DeviceView {
	return {
		id: 'd-1',
		name: 'Laptop',
		os: 'windows',
		arch: 'x86_64',
		app_version: '1.0.0',
		first_seen_at: 0,
		last_seen_at: last_used_at,
		revoked_at: null,
		wipe_outstanding: false,
		runs_sourced_payloads: true,
		sessions: [
			{ marketplace, account_label: null, linked_at: 0, last_used_at, status: 'connected' }
		],
		...partial
	};
}

describe('the marketplace status rows', () => {
	it('draws one row per marketplace, in the console’s own platform order', () => {
		expect(statusRows(EVERY, [], NOW).map((row) => row.marketplace)).toEqual([
			'Tpt',
			'Tes',
			'Etsy'
		]);
	});

	it('says nothing beyond the pill while a marketplace is working', () => {
		const [row] = statusRows([entry()], [], NOW);
		expect(row.label).toBe('Working');
		expect(row.tone).toBe('ok');
		expect(row.why).toBe('');
	});

	it('carries the recorded reason and how long the pause has stood', () => {
		const [row] = statusRows(
			[
				entry({
					halted: true,
					reason: 'the write endpoint answered 503 four times running',
					raised_at: NOW - 3 * 3_600_000
				})
			],
			[],
			NOW
		);
		expect(row.label).toBe('Paused');
		expect(row.tone).toBe('bad');
		expect(row.why).toBe(
			'Sending here is paused: the write endpoint answered 503 four times running. Paused 3 h ago.'
		);
	});

	// A halt with no reason is a fact about the record, not about the
	// marketplace, so it says so rather than reading as an empty sentence.
	it('says so where the halt carries no reason, and omits an age never stamped', () => {
		const [row] = statusRows([entry({ halted: true })], [], NOW);
		expect(row.why).toBe('Sending here is paused: no reason recorded.');
	});

	// The marketplace this console cannot write to is not "working": nothing
	// is sent there, so a green pill would be a claim about a path that does
	// not exist.
	it('says a marketplace with no adapter is coming rather than working', () => {
		const etsy = statusRows(EVERY, [], NOW).find((row) => row.marketplace === 'Etsy');
		expect(etsy?.label).toBe('Coming soon');
		expect(etsy?.tone).toBe('soon');
		expect(etsy?.why).toContain('cannot send resources here yet');
	});
});

describe('when the seller’s own device last worked with a marketplace', () => {
	it('names it where a device holds a live session', () => {
		const [row] = statusRows([entry()], [device('Tes', NOW - 2 * 3_600_000)], NOW);
		expect(row.checked).toBe('Last used by your Teachouse app 2 h ago');
	});

	it('answers the newest across the seller’s devices', () => {
		const rows = statusRows(
			[entry()],
			[
				device('Tes', NOW - 5 * 3_600_000),
				device('Tes', NOW - 1 * 3_600_000, { id: 'd-2' })
			],
			NOW
		);
		expect(rows[0].checked).toBe('Last used by your Teachouse app 1 h ago');
	});

	// A machine the seller signed out of, and a session that ended, both
	// record that the path stopped being used. Neither is evidence it works.
	it('reads nothing from a revoked device or an ended session', () => {
		const revoked = device('Tes', NOW - 3_600_000, { revoked_at: NOW });
		const ended = device('Tes', NOW - 3_600_000, {
			id: 'd-3',
			sessions: [
				{
					marketplace: 'Tes',
					account_label: null,
					linked_at: 0,
					last_used_at: NOW - 3_600_000,
					status: 'signed_out'
				}
			]
		});
		expect(statusRows([entry()], [revoked, ended], NOW)[0].checked).toBe('');
	});

	it('says nothing at all where the page is read with no device list', () => {
		expect(statusRows([entry()], [], NOW)[0].checked).toBe('');
	});

	it('never claims a check for a marketplace this console cannot reach', () => {
		const rows = statusRows(EVERY, [device('Etsy', NOW - 3_600_000)], NOW);
		expect(rows.find((row) => row.marketplace === 'Etsy')?.checked).toBe('');
	});
});
