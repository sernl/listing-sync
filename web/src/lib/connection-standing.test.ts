import { describe, expect, it } from 'vitest';
import type { ConnectionView } from '$lib/api';
import {
	CONNECTION_OFF,
	anyConnectionStands,
	connectionIsLinked,
	connectionStands,
	standingMarketplaces
} from './connection-standing';

function connection(partial: Partial<ConnectionView> = {}): ConnectionView {
	return {
		id: 'c-1',
		marketplace: 'Tes',
		state: 'linked',
		status: 'connected',
		created_at: 0,
		updated_at: 0,
		...partial
	};
}

// This module is the single answer three screens read, so its own test is the
// one that fails closest to a change in it. The consumer tests in
// `import-view.test.ts` and `sync-request.test.ts` would fail too, but each of
// them would report a badge or a migrate source rather than the fact that
// moved.
describe('which stored states mean the seller still has a connection', () => {
	it('counts a connection that is linked', () => {
		expect(connectionStands(connection({ state: 'linked' }))).toBe(true);
	});

	// The distinction the whole module exists for: "unwell" is not "off". It is
	// the seller's own device that discovers a sign-in is needed, and a
	// connection in flight is one they are already making.
	it('counts one that is unwell or in flight, which is not the same as gone', () => {
		expect(connectionStands(connection({ state: 'needs_reauth' }))).toBe(true);
		expect(connectionStands(connection({ state: 'linking' }))).toBe(true);
	});

	it('counts neither of the two states that mean it was given up', () => {
		expect(connectionStands(connection({ state: 'unlinked' }))).toBe(false);
		expect(connectionStands(connection({ state: 'revoked' }))).toBe(false);
	});

	it('names exactly those two, so a screen cannot read a third list', () => {
		expect([...CONNECTION_OFF].sort()).toEqual(['revoked', 'unlinked']);
	});

	it('counts an absent connection as no connection rather than throwing', () => {
		expect(connectionStands(null)).toBe(false);
		expect(connectionStands(undefined)).toBe(false);
	});
});

describe('whether the seller has any marketplace at all', () => {
	// The defect this replaces: the migration page asked `migrateSource`, which
	// is TES-only, and titled a banner "No marketplace is connected" for a
	// seller who had connected TPT — the ordinary first connection, since TPT
	// is where a migration writes.
	it('is true for a marketplace this screen cannot itself use', () => {
		expect(anyConnectionStands([connection({ marketplace: 'Tpt' })])).toBe(true);
		expect(anyConnectionStands([connection({ marketplace: 'Etsy' })])).toBe(true);
	});

	it('is false with no connections, and false with only given-up ones', () => {
		expect(anyConnectionStands([])).toBe(false);
		expect(
			anyConnectionStands([
				connection({ marketplace: 'Tes', state: 'unlinked' }),
				connection({ marketplace: 'Tpt', state: 'revoked' })
			])
		).toBe(false);
	});

	it('is true where one of several still stands', () => {
		expect(
			anyConnectionStands([
				connection({ marketplace: 'Tes', state: 'revoked' }),
				connection({ marketplace: 'Tpt', state: 'needs_reauth' })
			])
		).toBe(true);
	});
});

describe('whether a machine will reconnect this by itself', () => {
	// Narrower than `connectionStands` on purpose, and the two must not be
	// confused: `derive_link` writes `linked` when a live device reports a
	// session and lifts an `unlinked` row back to it on the next beat, so
	// `linked` is the state that says a machine is still reporting. A
	// `needs_reauth` row stands but nothing is reporting it.
	it('is true only for a linked connection', () => {
		expect(connectionIsLinked(connection({ state: 'linked' }))).toBe(true);
		for (const state of ['needs_reauth', 'linking', 'unlinked', 'revoked']) {
			expect(connectionIsLinked(connection({ state })), state).toBe(false);
		}
	});

	it('is false with no connection', () => {
		expect(connectionIsLinked(null)).toBe(false);
		expect(connectionIsLinked(undefined)).toBe(false);
	});
});

describe('the marketplaces a screen may treat as the seller’s', () => {
	it('drops the given-up ones and keeps the rest', () => {
		const held = standingMarketplaces([
			connection({ marketplace: 'Tes', state: 'linked' }),
			connection({ marketplace: 'Tpt', state: 'unlinked' }),
			connection({ marketplace: 'Etsy', state: 'needs_reauth' })
		]);
		expect([...held].sort()).toEqual(['Etsy', 'Tes']);
	});

	it('is empty for a seller with nothing', () => {
		expect(standingMarketplaces([]).size).toBe(0);
	});
});
