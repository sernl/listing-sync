// What has to hold for a resource just created to be on the board without a
// reload, pinned at the two places it can fail: the row itself, and the cache
// entry the board reads.
//
// The board's catalogue is keyed `['products', <label or null>]` and the create
// form invalidates `['products']`. Those are two different keys and it is only
// TanStack's prefix matching that connects them, so the connection is asserted
// here rather than assumed: an invalidation that stopped reaching the board
// would leave a seller looking at a catalogue without the resource they had
// just made, and nothing in the page would say so.

import { QueryClient } from '@tanstack/svelte-query';
import { describe, expect, it } from 'vitest';
import type { ProductHead } from '$lib/api';
import { rowFor } from '$lib/inventory';
import { EMPTINESS_KEY } from '$lib/pages/export/api';
import { queryKeys } from '$lib/query';
import { refreshAfterCreate } from './after-create';
import { NO_RESOURCE_FILTERS, countsFor, matchesResource, sortRows } from './list';

const NOW = 1_788_000_000_000;

function head(id: string, title: string): ProductHead {
	return {
		id,
		title,
		price: { Paid: { minor_units: 275, currency: 'Gbp' } },
		created_at: NOW,
		updated_at: NOW
	};
}

/** A resource on no marketplace at all: created, and cross-listed nowhere. */
const UNLISTED = head('p-lonely', 'Number bonds to twenty');

describe('a resource carrying no mapping', () => {
	it('is a row the board draws rather than a state it renders nothing for', () => {
		const row = rowFor({
			product: UNLISTED,
			mappings: [],
			work: new Map(),
			connections: [],
			statuses: []
		});
		expect(row.mapped.size).toBe(0);
		expect(row.chips.length).toBeGreaterThan(0);
		expect(row.chips.every((chip) => chip.state === 'not_listed')).toBe(true);
		expect(row.attention).toEqual([]);
	});

	it('survives the filters the board opens on, and is counted', () => {
		const row = rowFor({
			product: UNLISTED,
			mappings: [],
			work: new Map(),
			connections: [],
			statuses: []
		});
		expect(matchesResource(row, NO_RESOURCE_FILTERS)).toBe(true);
		expect(sortRows([row], 'updated_desc')).toHaveLength(1);
		const counts = countsFor([row], true);
		expect(counts.all).toBe(1);
		expect(counts.not_listed).toBe(1);
		expect(counts.attention).toBe(0);
	});
});

describe('the create flow invalidation and the board cache entry', () => {
	it('marks the unfiltered catalogue stale, so a remount refetches it', async () => {
		const client = new QueryClient();
		client.setQueryData(queryKeys.catalogue(null), []);
		expect(client.getQueryState(queryKeys.catalogue(null))?.isInvalidated).toBe(false);

		// The function `ResourceForm` calls after `POST /v1/products` answers,
		// rather than a second copy of what it does: a copy would keep passing
		// with the form's own invalidation deleted.
		await refreshAfterCreate(client);

		expect(client.getQueryState(queryKeys.catalogue(null))?.isInvalidated).toBe(true);
		client.clear();
	});

	it('reaches a label-narrowed catalogue and the export probe under the same prefix', async () => {
		const client = new QueryClient();
		const narrowed = queryKeys.catalogue(JSON.stringify(['maths']));
		client.setQueryData(narrowed, []);
		client.setQueryData(EMPTINESS_KEY, { empty: true });

		await refreshAfterCreate(client);

		expect(client.getQueryState(narrowed)?.isInvalidated).toBe(true);
		expect(client.getQueryState(EMPTINESS_KEY)?.isInvalidated).toBe(true);
		client.clear();
	});

	it('lists the new resource on the next read, with no reload in between', async () => {
		// Held fresh for ever on purpose. The app leaves `staleTime` at zero, so
		// a mount would re-read whatever happened, and a test run that way would
		// pass with the invalidation deleted: it would be measuring TanStack's
		// default rather than this code. Pinned, the only thing that can make
		// the second read happen is the invalidation itself.
		const client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity } } });
		let catalogue: ProductHead[] = [head('p-fractions', 'Fractions on a number line')];
		const walk = () => Promise.resolve(catalogue);

		const before = await client.fetchQuery({
			queryKey: queryKeys.catalogue(null),
			queryFn: walk
		});
		expect(before).toHaveLength(1);

		// The create lands on the server, then the form invalidates.
		catalogue = [...catalogue, UNLISTED];
		await refreshAfterCreate(client);

		// What mounting the board again does: the entry is stale, so it is read
		// rather than served from what the cache already held.
		const after = await client.fetchQuery({
			queryKey: queryKeys.catalogue(null),
			queryFn: walk
		});
		expect(after.map((product) => product.id)).toContain(UNLISTED.id);
		client.clear();
	});

	it('marks the mappings a create may have written, not only the catalogue', async () => {
		const client = new QueryClient();
		client.setQueryData(queryKeys.mappings, []);

		await refreshAfterCreate(client);

		expect(client.getQueryState(queryKeys.mappings)?.isInvalidated).toBe(true);
		client.clear();
	});
});
