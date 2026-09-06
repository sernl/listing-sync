// The `/v1/connections` wire shape, and every consumer of it read against the
// answer the server actually sends.
//
// Written after a live crash: the route answers an envelope, four queries
// under the one `['connections']` cache key held two shapes of it, and
// whichever refetched last decided which one the Resources board's `find` was
// handed. The board threw `connections.find is not a function` inside a
// `$derived`, so the render boundary drew its sentence instead of the page,
// and the sentence stayed up across navigation. The account that showed it had
// connections and resources; the test accounts had neither, so nothing on the
// way to production ever called `find`.
//
// WIRE below is transcribed from `ConnectionsView` and `ConnectionView` in
// `crates/tam-api/src/resources.rs`, including the `skip_serializing_if` that
// omits `authorship` rather than nulling it.

import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it, vi, afterEach } from 'vitest';
import { api, connectionList, type ConnectionView } from '$lib/api';
import { rowFor } from '$lib/inventory';
import { connectionFor } from '$lib/publish-readiness';
import { targetAuthorship } from '$lib/sync-request';
import { marketplaceRows, signInStates } from '$lib/devices-view';
import { importCards } from '$lib/pages/import/import-view';
import { cards } from '$lib/pages/automations/landing';
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
		expect(targetAuthorship(held)).toEqual({ kind: 'declared', name: 'Kauri Classroom' });
		expect(targetAuthorship(none)).toEqual({ kind: 'unrecorded' });
	});

	it('builds the marketplaces rows and sign-in states from the list', () => {
		for (const connections of [held, none]) {
			expect(signInStates([device()], connections, NOW).length).toBeGreaterThan(0);
			expect(marketplaceRows([device()], connections, NOW).length).toBeGreaterThan(0);
		}
		expect(
			marketplaceRows([device()], held, NOW).find((row) => row.marketplace === 'Tpt')?.authorship
		).toEqual({ state: 'declared', name: 'Kauri Classroom', attested_at: NOW - 2_592_000_000 });
	});

	it('builds the import cards from the list, and from an unread one', () => {
		expect(importCards(held).length).toBeGreaterThan(0);
		expect(importCards(none).length).toBeGreaterThan(0);
		expect(importCards(null).length).toBeGreaterThan(0);
	});

	it('builds the automations cards from the list', () => {
		for (const connections of [held, none]) {
			expect(cards({ connections, requests: [], openQuestions: 0 }).length).toBeGreaterThan(0);
		}
	});
});

// The bug was not one wrong line, it was two query functions under one cache
// key. TanStack keeps one entry per key and hands it to every observer, so a
// second shape written under `['connections']` is read by the consumers of the
// first, and which one wins is decided by whichever page was navigated to
// last. Nothing about that is visible in the file being edited, so the
// invariant is asserted over the tree.
describe('one cache key, one shape', () => {
	function sources(dir: string): string[] {
		return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
			const path = join(dir, entry.name);
			if (entry.isDirectory()) {
				return sources(path);
			}
			return entry.name.endsWith('.svelte') || entry.name.endsWith('.ts') ? [path] : [];
		});
	}

	it('reads queryKeys.connections through exactly one query function', () => {
		const found = sources(new URL('..', import.meta.url).pathname).flatMap((path) =>
			[...readFileSync(path, 'utf8').matchAll(/queryKey: queryKeys\.connections,\s*\n\s*(queryFn:[^\n]*)/g)].map(
				(match) => ({ path, queryFn: match[1].trim() })
			)
		);
		expect(found.length).toBeGreaterThan(1);
		expect([...new Set(found.map((entry) => entry.queryFn))]).toEqual([
			'queryFn: () => api.connections()'
		]);
	});
});
