import { describe, expect, it } from 'vitest';
import type { NotificationCounts, NotificationView } from '$lib/api';
import { readState } from '$lib/pages/automations/read-state';
import {
	NOTHING_TO_CHANGE,
	heldAfter,
	href,
	outcomes,
	readThrough,
	rows,
	title,
	unreadLine
} from './view';

const NOW = 1_788_000_000_000;
const MINUTE = 60_000;

const NONE: NotificationCounts = {
	succeeded: 0,
	degraded: 0,
	failed: 0,
	ambiguous: 0,
	skipped: 0,
	blocked: 0
};

function view(over: Partial<NotificationView> = {}): NotificationView {
	return {
		id: 'n-1',
		kind: 'sync',
		subject_id: 'r-1',
		inventory: 'Tes',
		marketplace: 'Tes',
		counts: { ...NONE, succeeded: 12 },
		created_at: NOW - 90 * MINUTE,
		read_at: null,
		...over
	};
}

describe('the four read states this page draws', () => {
	it('is pending before the list has answered', () => {
		expect(readState(false, false, rows([], NOW))).toEqual({ kind: 'pending' });
	});

	it('is failed rather than empty, which is what keeps the page honest', () => {
		expect(readState(true, true, rows([], NOW))).toEqual({ kind: 'failed' });
	});

	it('tells an inbox nobody has filled from one that could not be read', () => {
		expect(readState(true, false, rows([], NOW))).toEqual({ kind: 'empty' });
	});

	it('carries the rows once there are rows', () => {
		const state = readState(true, false, rows([view()], NOW));
		expect(state.kind).toBe('rows');
		expect(state.kind === 'rows' && state.rows[0]?.title).toBe('TES update finished');
	});
});

describe('a run that changed nothing', () => {
	it('draws no outcome at all, so the page has something to say instead', () => {
		expect(outcomes(NONE)).toEqual([]);
	});

	it('is the empty-outcome row rather than a row missing from the list', () => {
		const drawn = rows([view({ counts: NONE })], NOW);
		expect(drawn).toHaveLength(1);
		expect(drawn[0]?.outcomes).toEqual([]);
	});

	it('has a sentence to render in place of the counts', () => {
		expect(NOTHING_TO_CHANGE).toBe('Nothing to change');
	});
});

describe('the counts, in the outcome bar’s own words', () => {
	it('names each one the way outcome.ts names it', () => {
		const drawn = outcomes({
			succeeded: 12,
			degraded: 1,
			failed: 2,
			ambiguous: 3,
			skipped: 4,
			blocked: 5
		});
		expect(drawn.map((one) => one.label)).toEqual([
			'succeeded',
			'degraded',
			'failed',
			'ambiguous',
			'skipped',
			'blocked'
		]);
		expect(drawn.map((one) => one.count)).toEqual([12, 1, 2, 3, 4, 5]);
	});

	it('draws each in the colour the bar draws it', () => {
		const drawn = outcomes({ ...NONE, succeeded: 1, failed: 1 });
		expect(drawn.map((one) => one.tone)).toEqual(['seg-ok', 'seg-bad']);
	});

	it('leaves out every zero, so a clean run reads as one line', () => {
		expect(outcomes({ ...NONE, succeeded: 3 }).map((one) => one.label)).toEqual(['succeeded']);
	});

	it("counts a notification's blocked through the segment that renders it", () => {
		expect(outcomes({ ...NONE, blocked: 7 })).toEqual([
			{ label: 'blocked', count: 7, share: 1, tone: 'seg-block' }
		]);
	});
});

describe('what a row says and where it opens', () => {
	it('names the marketplace the run wrote to', () => {
		expect(title(view({ inventory: 'Tes' }))).toBe('TES update finished');
		expect(title(view({ inventory: 'Tpt' }))).toBe('TPT update finished');
	});

	it('names no platform for an import, which writes to none', () => {
		expect(title(view({ kind: 'import', inventory: null, marketplace: null }))).toBe(
			'Import finished'
		);
	});

	it('opens a sync and a migration on the request they name', () => {
		expect(href(view({ kind: 'sync', subject_id: 'r-9' }))).toBe('/sync/requests/r-9');
		expect(href(view({ kind: 'migration', subject_id: 'r-9' }))).toBe('/sync/requests/r-9');
	});

	it('opens an import on its batch', () => {
		expect(href(view({ kind: 'import', subject_id: 'b-4' }))).toBe('/imports/b-4');
	});

	it('escapes the identifier rather than pasting it into a path', () => {
		expect(href(view({ subject_id: 'a/b' }))).toBe('/sync/requests/a%2Fb');
	});

	it('says how long ago in the words the rest of the console uses', () => {
		expect(rows([view({ created_at: NOW - 90 * MINUTE })], NOW)[0]?.when).toBe('1 h ago');
	});
});

describe('a kind this bundle has no word for', () => {
	// The console is a static bundle served from the control plane, so a deploy
	// that widens the enum does not rebuild the page a seller already has open.
	// The type check is the first half of the answer and this is the second.
	const unknown = view({
		// @ts-expect-error the point of the test is a kind the union forbids
		kind: 'digest'
	});

	it('says a run finished rather than rendering a blank noun', () => {
		expect(title(unknown)).toBe('TES task finished');
	});

	it('offers no link rather than inventing a path', () => {
		expect(href(unknown)).toBeNull();
	});
});

describe('the read mark', () => {
	it('reaches the newest row, which is the furthest it can reach', () => {
		expect(
			readThrough([view({ id: 'n-2', read_at: null }), view({ id: 'n-1', read_at: null })])
		).toBe('n-2');
	});

	it('posts nothing for a page whose every row is already read', () => {
		expect(readThrough([view({ id: 'n-2', read_at: NOW }), view({ id: 'n-1', read_at: NOW })])).toBeNull();
	});

	it('posts nothing for an empty inbox', () => {
		expect(readThrough([])).toBeNull();
	});

	it('still reaches the newest row when only an older one is unread', () => {
		expect(readThrough([view({ id: 'n-2', read_at: NOW }), view({ id: 'n-1', read_at: null })])).toBe(
			'n-2'
		);
	});
});

describe('the eyebrow over the list', () => {
	it('counts what is unread on the pages loaded', () => {
		const drawn = rows([view({ id: 'a' }), view({ id: 'b', read_at: NOW })], NOW);
		expect(unreadLine(drawn)).toBe('1 new');
	});

	it('says so plainly when nothing is new', () => {
		expect(unreadLine(rows([view({ read_at: NOW })], NOW))).toBe('Nothing new');
	});

	it('pluralises rather than printing a bare figure', () => {
		expect(unreadLine(rows([view({ id: 'a' }), view({ id: 'b' })], NOW))).toBe('2 new');
	});
});

describe('the rows held after a page arrives', () => {
	it('appends a continuation, which is what Load more is', () => {
		const first = [view({ id: 'a' }), view({ id: 'b' })];
		const second = [view({ id: 'c' })];
		expect(heldAfter(first, second, 'cursor-1').map((row) => row.id)).toEqual(['a', 'b', 'c']);
	});

	it('replaces on a first page, so a retry after a failed Load more cannot duplicate an id', () => {
		const first = [view({ id: 'a' }), view({ id: 'b' })];
		const again = heldAfter(first, first, undefined);
		expect(again.map((row) => row.id)).toEqual(['a', 'b']);
		expect(new Set(again.map((row) => row.id)).size).toBe(again.length);
	});

	it('treats a null cursor as a first page too', () => {
		expect(heldAfter([view({ id: 'a' })], [view({ id: 'a' })], null).map((row) => row.id)).toEqual([
			'a'
		]);
	});

	it('does not mutate the rows it was given', () => {
		const first = [view({ id: 'a' })];
		heldAfter(first, [view({ id: 'b' })], 'cursor-1');
		expect(first.map((row) => row.id)).toEqual(['a']);
	});
});
