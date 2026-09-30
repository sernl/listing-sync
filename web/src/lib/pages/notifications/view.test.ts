import { describe, expect, it } from 'vitest';
import type { NotificationCounts, NotificationView, RunNotification } from '$lib/api';
import { readState } from '$lib/pages/automations/read-state';
import {
	NOTHING_TO_CHANGE,
	badgeLabel,
	heldAfter,
	href,
	inboxAfter,
	outcomes,
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

type RunView = NotificationView & RunNotification;

function view(over: Partial<RunView> = {}): RunView {
	return {
		id: 'n-1',
		tone: 'success',
		source: 'run',
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
			'done',
			'done with warnings',
			'failed',
			'unclear',
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
		expect(outcomes({ ...NONE, succeeded: 3 }).map((one) => one.label)).toEqual(['done']);
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

function notice(over: Partial<NotificationView> = {}): NotificationView {
	return {
		id: 'k-1',
		tone: 'warning',
		source: 'notice',
		title: 'That file is too big to send.',
		body: '',
		created_at: NOW - 5 * MINUTE,
		read_at: null,
		...over
	} as NotificationView;
}

describe('a kept notice', () => {
	it('says its own title and text, in its own tone, and opens nothing', () => {
		const [row] = rows([notice({ body: 'TES takes files up to 1 GB.' })], NOW);
		expect(row).toMatchObject({
			tone: 'warning',
			title: 'That file is too big to send.',
			line: 'TES takes files up to 1 GB.',
			href: null,
			outcomes: [],
			unread: true
		});
	});
});

describe('the one line under a run', () => {
	it('puts the counts in words', () => {
		const [row] = rows([view({ counts: { ...NONE, succeeded: 19, failed: 9 } })], NOW);
		expect(row?.line).toBe(
			row?.outcomes.map((segment) => `${segment.count} ${segment.label}`).join(' · ')
		);
		expect(row?.line).toContain('19');
	});

	it('says a run that changed nothing in words', () => {
		expect(rows([view({ counts: NONE })], NOW)[0]?.line).toBe(NOTHING_TO_CHANGE);
	});
});

describe('the eyebrow over the list', () => {
	it('counts every unread row the seller has', () => {
		expect(unreadLine(1)).toBe('1 new');
		expect(unreadLine(2)).toBe('2 new');
	});

	it('says so plainly when nothing is new', () => {
		expect(unreadLine(0)).toBe('Nothing new');
	});
});

describe('the bell’s badge', () => {
	it('draws nothing when nothing is unread', () => {
		expect(badgeLabel(0)).toBeNull();
	});

	it('prints the figure up to 99 and caps it past that', () => {
		expect(badgeLabel(1)).toBe('1');
		expect(badgeLabel(99)).toBe('99');
		expect(badgeLabel(100)).toBe('99+');
	});
});

describe('what a Mark read or a Dismiss does before the server answers', () => {
	const held = {
		notifications: [notice({ id: 'k-1' }), view({ id: 'n-1', read_at: NOW - MINUTE })],
		unread: 5
	};

	it('marks one unread row read and takes one off the count', () => {
		const after = inboxAfter(held, { kind: 'read', id: 'k-1', at: NOW });
		expect(after.notifications[0]?.read_at).toBe(NOW);
		expect(after.unread).toBe(4);
	});

	it('leaves the count alone for a row that was read already, or is not held', () => {
		expect(inboxAfter(held, { kind: 'read', id: 'n-1', at: NOW }).unread).toBe(5);
		expect(inboxAfter(held, { kind: 'read', id: 'gone', at: NOW }).unread).toBe(5);
	});

	it('reads everything and empties the count, even rows beyond the ones held', () => {
		const after = inboxAfter(held, { kind: 'readAll', at: NOW });
		expect(after.notifications.every((one) => one.read_at !== null)).toBe(true);
		expect(after.unread).toBe(0);
	});

	it('takes a dismissed unread row off the list and the count', () => {
		const after = inboxAfter(held, { kind: 'dismiss', id: 'k-1' });
		expect(after.notifications.map((one) => one.id)).toEqual(['n-1']);
		expect(after.unread).toBe(4);
	});

	it('takes a dismissed read row off the list and leaves the count', () => {
		expect(inboxAfter(held, { kind: 'dismiss', id: 'n-1' }).unread).toBe(5);
	});

	it('takes only the read rows away on Delete read', () => {
		const after = inboxAfter(held, { kind: 'dismissRead' });
		expect(after.notifications.map((one) => one.id)).toEqual(['k-1']);
		expect(after.unread).toBe(5);
	});

	it('does not mutate what it was given', () => {
		inboxAfter(held, { kind: 'readAll', at: NOW });
		expect(held.notifications[0]?.read_at).toBeNull();
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
