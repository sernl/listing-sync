import { describe, expect, it } from 'vitest';
import {
	activityRows,
	attention,
	liveTally,
	recentlyUpdated,
	salesCapture,
	syncTally
} from './dashboard';
import type {
	ConnectionView,
	Counts,
	InventoryStatus,
	JobView,
	ListingMetricsView,
	MappingHead,
	ProductHead
} from './api';
import type { InventoryId } from './generated/vocab';

function counts(partial: Partial<Counts>): Counts {
	return {
		total: 0,
		queued: 0,
		in_flight: 0,
		blocked: 0,
		parked: 0,
		settled: 0,
		succeeded: 0,
		degraded: 0,
		failed: 0,
		ambiguous: 0,
		skipped: 0,
		outcome_blocked: 0,
		...partial
	};
}

function job(id: string, phase: JobView['phase'], partial: Partial<Counts>): JobView {
	return {
		job: id,
		inventory: 'TesNz',
		created_at: 1_000,
		phase,
		counts: counts(partial)
	};
}

function mapping(
	id: string,
	inventory: InventoryId,
	binding_state: string,
	lifecycle_state: string
): MappingHead {
	return { id, product: `p-${id}`, inventory, binding_state, lifecycle_state, updated_at: 0 };
}

function connection(id: string, status: ConnectionView['status']): ConnectionView {
	return {
		id,
		marketplace: 'Tpt',
		state: 'linked',
		status,
		created_at: 0,
		updated_at: 0
	};
}

const NO_TALLY = { active: 0, pending: 0, failed: 0, failingRuns: 0, runs: 0 };

describe('the live-listings figure', () => {
	it('counts only mappings bound to a listing the marketplace shows', () => {
		const tally = liveTally([
			mapping('a', 'Tpt', 'bound', 'live'),
			mapping('b', 'TesGb', 'bound', 'draft'),
			mapping('c', 'TesNz', 'unbound', 'absent')
		]);
		expect(tally).toEqual({ live: 1, total: 3, marketplaces: 1 });
	});

	it('counts marketplaces rather than inventories, so two Tes sites are one', () => {
		const tally = liveTally([
			mapping('a', 'TesGb', 'bound', 'live'),
			mapping('b', 'TesNz', 'bound', 'live'),
			mapping('c', 'Tpt', 'bound', 'live')
		]);
		expect(tally).toEqual({ live: 3, total: 3, marketplaces: 2 });
	});

	it('reads an empty catalogue as zero rather than as unknown', () => {
		expect(liveTally([])).toEqual({ live: 0, total: 0, marketplaces: 0 });
	});
});

describe('the sync activity rows', () => {
	it('call an unsettled run running, whatever it has recorded so far', () => {
		const [row] = activityRows([job('j1', 'active', { total: 4, succeeded: 1, failed: 1 })]);
		expect(row.label).toBe('Running');
		expect(row.tone).toBe('run');
	});

	it('name a settled run by the worst outcome it recorded', () => {
		expect(activityRows([job('j', 'settled', { total: 2, succeeded: 1, failed: 1 })])[0].label).toBe(
			'Failed'
		);
		expect(
			activityRows([job('j', 'settled', { total: 2, succeeded: 1, outcome_blocked: 1 })])[0].label
		).toBe('Blocked');
		expect(activityRows([job('j', 'settled', { total: 2, succeeded: 1, degraded: 1 })])[0].label).toBe(
			'Degraded'
		);
		expect(activityRows([job('j', 'settled', { total: 2, succeeded: 2 })])[0].label).toBe('Done');
	});

	it('write the counts out rather than summarising them into a verdict', () => {
		const [row] = activityRows([job('j', 'settled', { total: 3, succeeded: 2, failed: 1 })]);
		expect(row.detail).toBe('3 items · 2 succeeded · 1 failed');
	});

	it('say a run recorded no items rather than showing an empty line', () => {
		expect(activityRows([job('j', 'settled', {})])[0].detail).toBe('no items');
	});
});

describe('the sync tally', () => {
	it('counts what the runs read actually carry', () => {
		expect(
			syncTally([
				job('a', 'active', { total: 5, queued: 2, in_flight: 1, succeeded: 2 }),
				job('b', 'settled', { total: 3, succeeded: 2, failed: 1 }),
				job('c', 'settled', { total: 1, failed: 1 })
			])
		).toEqual({ active: 1, pending: 3, failed: 2, failingRuns: 2, runs: 3 });
	});

	it('is all zeroes when nothing has run', () => {
		expect(syncTally([])).toEqual(NO_TALLY);
	});
});

describe('the captured sales total', () => {
	const now = 1_756_000_000_000;

	function captured(mapping: string, observed_at: number, sales?: number): ListingMetricsView {
		return {
			mapping,
			inventory: 'Tpt',
			observed_at,
			metrics: sales === undefined ? {} : { sales_count: sales }
		};
	}

	it('sums what was captured and ages it by the oldest row, not the newest', () => {
		const capture = salesCapture(
			[captured('a', now - 3_600_000, 11), captured('b', now - 10_800_000, 6)],
			now
		);
		expect(capture.sales).toBe(17);
		expect(capture.listings).toBe(2);
		expect(capture.age).toBe('captured 3 h ago');
	});

	it('is absent rather than zero when no row carries a sales figure', () => {
		const capture = salesCapture([captured('a', now - 60_000)], now);
		expect(capture.sales).toBeNull();
		expect(capture.age).toBe('captured 1 min ago');
	});

	it('has no total and no age when nothing has been captured at all', () => {
		expect(salesCapture([], now)).toEqual({ sales: null, listings: 0, age: null });
	});
});

describe('what needs a person', () => {
	const halted: InventoryStatus = {
		inventory: 'TesNz',
		marketplace: 'Tes',
		halted: true,
		reason: 'the adapter was withdrawn'
	};

	it('is empty when every signal is clear', () => {
		expect(
			attention({
				connections: [connection('c1', 'connected'), connection('c2', 'checking')],
				statuses: [{ inventory: 'Tpt', marketplace: 'Tpt', halted: false }],
				openQuestions: 0,
				tally: NO_TALLY
			})
		).toEqual([]);
	});

	it('puts what is broken before what is only degraded', () => {
		const items = attention({
			connections: [connection('c1', 'unstable'), connection('c2', 'disconnected')],
			statuses: [halted],
			openQuestions: 2,
			tally: { ...NO_TALLY, failed: 1, failingRuns: 1, runs: 4 }
		});
		expect(items.map((item) => item.tone)).toEqual(['bad', 'bad', 'bad', 'warn', 'warn']);
		expect(items.map((item) => item.key)).toEqual([
			'connection-c2',
			'halted-TesNz',
			'failed-writes',
			'unstable-c1',
			'reconciliation'
		]);
	});

	it('states the halt reason the server gave rather than one of its own', () => {
		const [item] = attention({
			connections: [],
			statuses: [halted],
			openQuestions: 0,
			tally: NO_TALLY
		});
		expect(item.body).toBe('the adapter was withdrawn');
	});

	it('still says something when a halt carries no reason', () => {
		const [item] = attention({
			connections: [],
			statuses: [{ ...halted, reason: undefined }],
			openQuestions: 0,
			tally: NO_TALLY
		});
		expect(item.body).toMatch(/queued items wait/);
	});

	it('counts in the singular where one is one', () => {
		const items = attention({
			connections: [],
			statuses: [],
			openQuestions: 1,
			tally: { ...NO_TALLY, failed: 1, failingRuns: 1, runs: 1 }
		});
		expect(items[0].title).toBe('1 write failed');
		expect(items[0].body).toMatch(/Across 1 of the 1 most recent run\./);
		expect(items[1].title).toBe('1 question waiting');
	});
});

describe('the recently updated strip', () => {
	function product(id: string, updated_at: number): ProductHead {
		return { id, title: id, price: 'Free', created_at: 0, updated_at };
	}

	it('is the newest first, cut to the limit', () => {
		const rows = recentlyUpdated([product('a', 3), product('b', 9), product('c', 5)], 2);
		expect(rows.map((row) => row.id)).toEqual(['b', 'c']);
	});

	it('leaves the catalogue it was given in the order it was given', () => {
		const catalogue = [product('a', 3), product('b', 9)];
		recentlyUpdated(catalogue, 2);
		expect(catalogue.map((row) => row.id)).toEqual(['a', 'b']);
	});
});
