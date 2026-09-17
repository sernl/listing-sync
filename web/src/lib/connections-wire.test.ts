// Regression fixture for the ConnectionsView envelope. Mixed cache shapes once
// caused Resources to throw `connections.find is not a function`.

import { describe, expect, it, vi, afterEach } from 'vitest';
import { api, connectionList, type ConnectionView } from '$lib/api';
import { rowFor } from '$lib/inventory';
import { connectionFor } from '$lib/publish-readiness';
import { targetAuthorship } from '$lib/sync-request';
import { marketplaceRows } from '$lib/devices-view';
import type { DeviceView, ProductHead } from '$lib/api';

const NOW = 1_788_000_000_000;

const WIRE = {
	connections: [
		{
			id: '5f9c2a1e-0d3b-4f6a-9c21-2b7f4d8e1a03',
			marketplace: 'Tpt',
			transport: 'SellerDevice',
			state: 'linked',
			status: 'connected',
			created_at: NOW - 5_184_000_000,
			updated_at: NOW - 86_400_000,
			authorship: { state: 'declared', name: 'Kauri Classroom', attested_at: NOW - 2_592_000_000 }
		},
		{
			id: '7b1d4c8a-6e25-4a70-8f13-9c0a5e2d6b44',
			marketplace: 'Tes',
			transport: 'SellerDevice',
			state: 'needs_reauth',
			status: 'disconnected',
			created_at: NOW - 5_184_000_000,
			updated_at: NOW - 3_600_000
		}
	]
};

function jsonResponse(status: number, body: unknown): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

function device(partial: Partial<DeviceView> = {}): DeviceView {
	return {
		id: 'd1',
		name: 'staffroom-laptop',
		os: 'windows',
		arch: 'x86_64',
		app_version: '0.1.0',
		first_seen_at: NOW - 100_000,
		last_seen_at: NOW - 1_000,
		revoked_at: null,
		wipe_outstanding: false,
		runs_sourced_payloads: false,
		sessions: [],
		...partial
	};
}

function head(id: string): ProductHead {
	return { id, title: 'Fractions pack', price: 'Free', created_at: 1, updated_at: 2 };
}

afterEach(() => {
	vi.unstubAllGlobals();
});

describe('the connections wire', () => {
	it('hands every caller the list rather than the envelope it arrives in', async () => {
		vi.stubGlobal(
			'fetch',
			vi.fn(async () => jsonResponse(200, WIRE))
		);
		const held = await api.connections();
		expect(Array.isArray(held)).toBe(true);
		expect(held.map((entry) => entry.marketplace)).toEqual(['Tpt', 'Tes']);
		// The absent declaration stays absent: a surface that serves none is not
		// a seller who declared none.
		expect(held[0].authorship).toEqual({
			state: 'declared',
			name: 'Kauri Classroom',
			attested_at: NOW - 2_592_000_000
		});
		expect(held[1].authorship).toBeUndefined();
	});

	it('reads an empty list as empty rather than as unread', () => {
		expect(connectionList({ connections: [] })).toEqual([]);
	});

	// Each of these is a shape that reached a consumer at some point in this
	// bug's life, or could: the envelope opened twice, the answer of a route
	// that failed into HTML, a 204 parsed as undefined.
	it.each([
		['the list already opened, handed back a second time', WIRE.connections],
		['an envelope holding something else', { connections: { Tpt: {} } }],
		['an answer with no list in it', {}],
		['nothing', undefined],
		['null', null],
		['a string', 'signed in nowhere']
	])('raises rather than degrading to empty on %s', (_name, answer) => {
		expect(() => connectionList(answer)).toThrow(/list of connections/);
	});
});

describe('every consumer of the list, on the real shape', () => {
	const held: ConnectionView[] = connectionList(WIRE);
	const none: ConnectionView[] = connectionList({ connections: [] });

	it('draws a resources row with a connection present and with none', () => {
		for (const connections of [held, none]) {
			const row = rowFor({
				product: head('p1'),
				mappings: [],
				work: new Map(),
				connections,
				statuses: []
			});
			expect(row.chips.length).toBeGreaterThan(0);
		}
		expect(
			rowFor({
				product: head('p1'),
				mappings: [],
				work: new Map(),
				connections: held,
				statuses: []
			}).chips.find((chip) => chip.inventory === 'Tpt')
		).toBeDefined();
	});

	it('matches a connection to an inventory, and answers undefined for none', () => {
		expect(connectionFor('Tpt', held)?.marketplace).toBe('Tpt');
		expect(connectionFor('Tpt', none)).toBeUndefined();
	});

	it('reads the declaration standing off the list', () => {
		expect(targetAuthorship(held, 'Tpt')).toEqual({ kind: 'declared', name: 'Kauri Classroom' });
		expect(targetAuthorship(none, 'Tpt')).toEqual({ kind: 'unrecorded' });
	});

	it('preserves seller authorship when building marketplace rows', () => {
		expect(
			marketplaceRows([device()], held, NOW).find((row) => row.marketplace === 'Tpt')?.authorship
		).toEqual({ state: 'declared', name: 'Kauri Classroom', attested_at: NOW - 2_592_000_000 });
	});

});

