import { describe, expect, it } from 'vitest';
import {
	ATTENTION_STATES,
	bulkTarget,
	chipFor,
	inventoryTally,
	matchesFilters,
	newestWork,
	NO_FILTERS,
	onSellerDevice,
	rowFor,
	runsFor,
	stripFor,
	type WorkItem
} from './inventory';
import type {
	ConnectionView,
	InventoryStatus,
	ItemView,
	MappingHead,
	ProductHead
} from './api';
import type { InventoryId } from './generated/vocab';

function product(id: string, title = 'Fractions pack'): ProductHead {
	return { id, title, price: 'Free', created_at: 1, updated_at: 2 };
}

function mapping(partial: Partial<MappingHead> & { inventory: InventoryId }): MappingHead {
	return {
		id: `m-${partial.inventory}`,
		product: 'p1',
		binding_state: 'bound',
		lifecycle_state: 'live',
		updated_at: 5,
		listing_url: null,
		...partial
	};
}

function item(partial: Partial<ItemView>): ItemView {
	return {
		item: 'i1',
		mapping: 'm-Tpt',
		state: 'queued',
		attempt_count: 1,
		created_at: 10,
		...partial
	};
}

function work(partial: Partial<ItemView>, job = 'j1'): WorkItem {
	return { job, item: item(partial) };
}

function connection(
	marketplace: ConnectionView['marketplace'],
	status: ConnectionView['status'] = 'connected'
): ConnectionView {
	return {
		id: `c-${marketplace}`,
		marketplace,
		state: 'linked',
		status,
		created_at: 1,
		updated_at: 1
	};
}

const CONNECTED: ConnectionView[] = [connection('Tpt'), connection('Tes')];

function chip(
	inventory: InventoryId,
	over: {
		mapping?: MappingHead;
		work?: WorkItem;
		connection?: ConnectionView;
		status?: InventoryStatus;
	} = {}
) {
	return chipFor({
		product: 'p1',
		inventory,
		mapping: over.mapping,
		work: over.work,
		connection: over.connection ?? connection(inventory === 'Tpt' ? 'Tpt' : 'Tes'),
		status: over.status
	});
}

describe('the transport branch', () => {
	it('puts TPT and Tes on the seller device and Etsy on the sanctioned API', () => {
		expect(onSellerDevice('Tpt')).toBe(true);
		expect(onSellerDevice('TesNz')).toBe(true);
		expect(onSellerDevice('Etsy')).toBe(false);
	});

	it('says a send runs on the seller machine only for the device branch', () => {
		const onDevice = chip('Tpt', {
			mapping: mapping({ inventory: 'Tpt', binding_state: 'unbound' }),
			work: work({ mapping: 'm-Tpt', state: 'running' })
		});
		expect(onDevice.detail).toContain('your own device');
		const served = chip('Etsy', {
			mapping: mapping({ id: 'm-Etsy', inventory: 'Etsy', binding_state: 'unbound' }),
			work: work({ mapping: 'm-Etsy', state: 'running' }),
			connection: connection('Etsy')
		});
		expect(served.detail).not.toContain('device');
	});
});

describe('a chip with no run touching it', () => {
	it('is not listed where no mapping exists at all', () => {
		const only = chip('Tpt');
		expect(only.state).toBe('not_listed');
		expect(only.action).toBeNull();
	});

	it('is not listed where the mapping was never sent', () => {
		const never = chip('Tpt', {
			mapping: mapping({ inventory: 'Tpt', binding_state: 'unbound' })
		});
		expect(never.state).toBe('not_listed');
		expect(never.action?.href).toBe('/inventory/p1');
	});

	it('a listed chip opens the listing itself when the server serves its page', () => {
		const listed = chip('Tpt', {
			mapping: mapping({
				inventory: 'Tpt',
				listing_url: 'https://www.teacherspayteachers.com/Product/listing-17511712'
			})
		});
		expect(listed.state).toBe('listed');
		expect(listed.action).toEqual({
			label: 'Open the listing',
			href: 'https://www.teacherspayteachers.com/Product/listing-17511712',
			external: true
		});
	});

	it('a listed chip with no served page falls back to the item rather than guessing', () => {
		const listed = chip('Etsy', {
			mapping: mapping({ id: 'm-Etsy', inventory: 'Etsy', listing_url: null }),
			connection: connection('Etsy')
		});
		expect(listed.state).toBe('listed');
		expect(listed.action).toEqual({ label: 'Open the item', href: '/inventory/p1' });
		expect(listed.action?.external).toBeUndefined();
	});

	it('separates a draft from a live listing', () => {
		expect(chip('Tpt', { mapping: mapping({ inventory: 'Tpt' }) }).state).toBe('listed');
		expect(
			chip('Tpt', { mapping: mapping({ inventory: 'Tpt', lifecycle_state: 'draft' }) }).state
		).toBe('draft');
	});

	it('reads an unrecognised binding as a send whose outcome is unknown', () => {
		const unknown = chip('Tpt', {
			mapping: mapping({ inventory: 'Tpt', binding_state: 'creating' })
		});
		expect(unknown.state).toBe('in_flight');
	});
});

describe('a chip the newest run speaks for', () => {
	const bound = mapping({ inventory: 'Tpt' });

	it('is sending while the item is queued, leased, running or verifying', () => {
		for (const state of ['queued', 'leased', 'running', 'verifying'] as const) {
			expect(chip('Tpt', { mapping: bound, work: work({ state }) }).state).toBe('in_flight');
		}
	});

	it('is held where a send parked, and names the gate it waits on', () => {
		const held = chip('Tpt', {
			mapping: bound,
			work: work({ state: 'parked_live', blocked_on: 'ReauthRequired' })
		});
		expect(held.state).toBe('stranded');
		expect(held.detail).toContain('you signing in again');
		expect(held.action?.href).toBe('/marketplaces');
	});

	// A park whose gate is an unanswered write is not an interruption: the
	// send completed and the answer is what is missing, so this branch reads
	// differently for that one gate and identically for every other.
	it('says the answer is missing where the write completed, not that it was interrupted', () => {
		const held = chip('Tpt', {
			mapping: bound,
			work: work({ state: 'parked_live', blocked_on: 'awaiting_marketplace_answer' })
		});
		expect(held.state).toBe('stranded');
		expect(held.detail).toBe(
			'We sent the listing, did not get an answer we could trust, and are going back to look for it.'
		);
		expect(held.detail).not.toContain('interrupted');
		expect(held.detail).not.toContain('waiting on');
		// Nothing the seller does clears it, so the chip offers the run rather
		// than a sign-in it would not fix.
		expect(held.action?.href).toBe('/sync/j1');
	});

	it('leaves every other parked gate reading as an interruption', () => {
		const held = chip('Tpt', {
			mapping: bound,
			work: work({ state: 'parked_cold', blocked_on: 'reconciliation' })
		});
		expect(held.state).toBe('stranded');
		expect(held.detail).toContain('interrupted');
		expect(held.detail).toContain('held rather than retried blind');
		expect(held.detail).toContain('waiting on reconciliation');
	});

	it('names a parked item with no gate rather than inventing one', () => {
		const held = chip('Tpt', { mapping: bound, work: work({ state: 'parked_cold' }) });
		expect(held.detail).toContain('not been told the name of');
	});

	it('separates a sign-in gate from every other blocked gate', () => {
		const signIn = chip('Tpt', {
			mapping: bound,
			work: work({ state: 'blocked', blocked_on: 'awaiting_seller_signin' })
		});
		expect(signIn.state).toBe('needs_signin');
		expect(signIn.action?.href).toBe('/marketplaces');

		const question = chip('Tpt', {
			mapping: bound,
			work: work({ state: 'blocked', blocked_on: 'reconciliation' })
		});
		expect(question.state).toBe('blocked');
		expect(question.action?.href).toBe('/reconciliation');
	});

	it('sends a cover gate to the listing and every unrouted gate to the run', () => {
		expect(
			chip('Tpt', {
				mapping: bound,
				work: work({ state: 'blocked', blocked_on: 'cover_missing' })
			}).action?.href
		).toBe('/inventory/p1');
		expect(
			chip('Tpt', {
				mapping: bound,
				work: work({ state: 'blocked', blocked_on: 'election' }, 'j9')
			}).action?.href
		).toBe('/sync/j9');
	});

	it('is failed only where the item settled failed, and carries its own reason', () => {
		const failed = chip('Tpt', {
			mapping: bound,
			work: work({ state: 'settled', outcome: 'failed', failure_detail: 'the form refused' })
		});
		expect(failed.state).toBe('failed');
		expect(failed.detail).toBe('the form refused');
	});

	it('lets the stored standing answer for a run that settled well', () => {
		const done = chip('Tpt', {
			mapping: bound,
			work: work({ state: 'settled', outcome: 'succeeded' })
		});
		expect(done.state).toBe('listed');
	});
});

describe('a dropped connection', () => {
	it('outranks the stored standing and says the listing is still up', () => {
		const dropped = chip('Tpt', {
			mapping: mapping({ inventory: 'Tpt' }),
			connection: connection('Tpt', 'disconnected')
		});
		expect(dropped.state).toBe('needs_signin');
		expect(dropped.detail).toContain('still up');
	});

	it('does not outrank a run that is already speaking', () => {
		const running = chip('Tpt', {
			mapping: mapping({ inventory: 'Tpt' }),
			work: work({ state: 'running' }),
			connection: connection('Tpt', 'disconnected')
		});
		expect(running.state).toBe('in_flight');
	});

	it('reads a marketplace with no connection at all as needing a sign-in', () => {
		const none = chipFor({
			product: 'p1',
			inventory: 'Tpt',
			mapping: mapping({ inventory: 'Tpt' }),
			work: undefined,
			connection: undefined,
			status: undefined
		});
		expect(none.state).toBe('needs_signin');
	});
});

describe('a halted marketplace', () => {
	it('overlays the standing rather than replacing it, and quotes its reason', () => {
		const halted = chip('Tpt', {
			mapping: mapping({ inventory: 'Tpt' }),
			status: {
				inventory: 'Tpt',
				marketplace: 'Tpt',
				halted: true,
				reason: 'a capture is being re-taken'
			}
		});
		expect(halted.state).toBe('listed');
		expect(halted.paused).toBe('a capture is being re-taken');
	});

	it('leaves the overlay empty where nothing is halted', () => {
		expect(chip('Tpt', { mapping: mapping({ inventory: 'Tpt' }) }).paused).toBeNull();
	});
});

describe('the newest item per mapping', () => {
	it('keeps the latest and discards what a later run replaced', () => {
		const newest = newestWork([
			work({ mapping: 'm1', created_at: 1, state: 'settled', outcome: 'failed' }),
			work({ mapping: 'm1', created_at: 9, state: 'running' }),
			work({ mapping: 'm2', created_at: 4, state: 'blocked' })
		]);
		expect(newest.get('m1')?.item.state).toBe('running');
		expect(newest.get('m2')?.item.state).toBe('blocked');
		expect(newest.size).toBe(2);
	});
});

describe('the strip', () => {
	it('shows every marketplace this console authors for, mapped or not', () => {
		expect(stripFor([])).toEqual(['Tpt', 'TesGb', 'TesUs', 'TesNz']);
	});

	it('keeps a mapping onto a marketplace the form does not offer', () => {
		expect(stripFor([mapping({ inventory: 'Etsy' })])).toContain('Etsy');
	});
});

describe('a row', () => {
	const row = rowFor({
		product: product('p1'),
		mappings: [
			mapping({ id: 'm-Tpt', inventory: 'Tpt' }),
			mapping({ id: 'm-TesNz', inventory: 'TesNz', binding_state: 'unbound' })
		],
		work: newestWork([work({ mapping: 'm-Tpt', state: 'blocked', blocked_on: 'ReauthRequired' })]),
		connections: CONNECTED,
		statuses: []
	});

	it('carries one chip per marketplace in the strip', () => {
		expect(row.chips.map((entry) => entry.inventory)).toEqual([
			'Tpt',
			'TesGb',
			'TesUs',
			'TesNz'
		]);
	});

	it('lists what needs acting on, worst first', () => {
		expect(row.attention.map((entry) => entry.state)).toEqual(['needs_signin']);
		expect(ATTENTION_STATES[0]).toBe('needs_signin');
	});

	it('indexes the mappings it could send by marketplace', () => {
		expect(row.mapped.get('TesNz')?.id).toBe('m-TesNz');
		expect(row.mapped.has('TesGb')).toBe(false);
	});
});

describe('the filter bar', () => {
	const rows = [
		rowFor({
			product: product('p1', 'Fractions pack'),
			mappings: [mapping({ id: 'm-Tpt', inventory: 'Tpt' })],
			work: new Map(),
			connections: CONNECTED,
			statuses: []
		}),
		rowFor({
			product: product('p2', 'Phonics mats'),
			mappings: [mapping({ id: 'm2', product: 'p2', inventory: 'TesNz' })],
			work: newestWork([
				work({ mapping: 'm2', state: 'settled', outcome: 'failed' })
			]),
			connections: CONNECTED,
			statuses: []
		})
	];

	it('lets everything through unfiltered', () => {
		expect(rows.filter((row) => matchesFilters(row, NO_FILTERS))).toHaveLength(2);
	});

	it('matches the title case-insensitively', () => {
		const found = rows.filter((row) => matchesFilters(row, { ...NO_FILTERS, query: 'PHONICS' }));
		expect(found.map((row) => row.product.id)).toEqual(['p2']);
	});

	it('narrows to the rows a marketplace carries', () => {
		const found = rows.filter((row) =>
			matchesFilters(row, { ...NO_FILTERS, marketplace: 'Tpt' })
		);
		expect(found.map((row) => row.product.id)).toEqual(['p1']);
	});

	it('finds what needs attention', () => {
		const found = rows.filter((row) =>
			matchesFilters(row, { ...NO_FILTERS, standing: 'attention' })
		);
		expect(found.map((row) => row.product.id)).toEqual(['p2']);
	});

	it('reads a marketplace and a standing together as one question', () => {
		expect(
			rows.filter((row) =>
				matchesFilters(row, { ...NO_FILTERS, marketplace: 'Tpt', standing: 'not_listed' })
			).map((row) => row.product.id)
		).toEqual(['p2']);
		expect(
			rows.filter((row) =>
				matchesFilters(row, { ...NO_FILTERS, marketplace: 'Tpt', standing: 'failed' })
			)
		).toHaveLength(0);
	});
});

describe('the tally and the bulk target', () => {
	const rows = [
		rowFor({
			product: product('p1'),
			mappings: [mapping({ id: 'm-Tpt', inventory: 'Tpt' })],
			work: new Map(),
			connections: CONNECTED,
			statuses: []
		}),
		rowFor({
			product: product('p2'),
			mappings: [mapping({ id: 'm2', product: 'p2', inventory: 'TesNz' })],
			work: newestWork([work({ mapping: 'm2', state: 'settled', outcome: 'failed' })]),
			connections: CONNECTED,
			statuses: []
		})
	];

	it('counts the catalogue, what the filters show, and what needs a person', () => {
		const tally = inventoryTally(rows, rows.slice(0, 1));
		expect(tally).toEqual({ total: 2, shown: 1, attention: 1, listedOn: 1 });
	});

	it('names the mappings a bulk send would address and the items it must map first', () => {
		expect(bulkTarget(rows, 'Tpt')).toEqual({
			inventory: 'Tpt',
			mappings: ['m-Tpt'],
			unmapped: ['p2']
		});
		expect(bulkTarget(rows, 'TesGb')).toEqual({
			inventory: 'TesGb',
			mappings: [],
			unmapped: ['p1', 'p2']
		});
	});
});

describe('the runs that touched one listing', () => {
	const mappings = [
		mapping({ id: 'm-Tpt', inventory: 'Tpt' }),
		mapping({ id: 'm-TesNz', inventory: 'TesNz' })
	];

	it('is newest first and one row per run', () => {
		const rows = runsFor(
			mappings,
			newestWork([
				work({ mapping: 'm-Tpt', created_at: 3, state: 'settled', outcome: 'succeeded' }, 'j1'),
				work({ mapping: 'm-TesNz', created_at: 7, state: 'running' }, 'j2')
			])
		);
		expect(rows.map((row) => row.job)).toEqual(['j2', 'j1']);
		expect(rows[0].inventory).toBe('TesNz');
		expect(rows[1].outcome).toBe('succeeded');
	});

	it('collapses two mappings that travelled in one run to a single row', () => {
		const rows = runsFor(
			mappings,
			new Map([
				['m-Tpt', work({ mapping: 'm-Tpt', created_at: 1 }, 'j1')],
				['m-TesNz', work({ mapping: 'm-TesNz', created_at: 4 }, 'j1')]
			])
		);
		expect(rows).toHaveLength(1);
		expect(rows[0].inventory).toBe('TesNz');
	});

	it('is empty where no run in the window touched this listing', () => {
		expect(runsFor(mappings, newestWork([work({ mapping: 'other' })]))).toEqual([]);
	});
});
