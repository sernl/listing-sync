import { describe, expect, it } from 'vitest';
import {
	migrationCounts,
	migrationLog,
	migrationRows,
	migrations,
	openFromSource,
	pillTone
} from './migration';
import { request } from './fixtures.test-support';

const HOUR = 3_600_000;

describe('which requests this page owns', () => {
	it('keeps the migrate half and drops the sync half', () => {
		const rows = migrations([
			request({ request: 'a' }),
			request({ request: 'b', disposition: 'sync' })
		]);
		expect(rows.map((row) => row.request)).toEqual(['a']);
	});
});

describe('a migration row', () => {
	it('carries both shops, so the row draws each by its own mark', () => {
		const [row] = migrationRows([request({ source: 'Tes' })], 0);
		expect(row.source).toBe('Tes');
		expect(row.target).toBe('Tpt');
	});

	it('opens the request’s own page', () => {
		const [row] = migrationRows([request({ request: 'r-9' })], 0);
		expect(row.href).toBe('/sync/requests/r-9');
	});

	it('carries the stage the request list already presents, in the pill’s own tones', () => {
		const [waiting] = migrationRows([request({ state: 'pending', resources_total: 0 })], 0);
		expect(waiting.label).toBe('Waiting');
		expect(waiting.tone).toBe('soon');

		const [running] = migrationRows([request({ state: 'draining' })], 0);
		expect(running.tone).toBe('run');

		const [failed] = migrationRows([request({ state: 'failed' })], 0);
		expect(failed.tone).toBe('bad');
	});

	it('says when it started', () => {
		const [row] = migrationRows([request({ created_at: 0 })], 2 * HOUR);
		expect(row.meta).toContain('started 2 h ago');
	});
});

describe('the activity log', () => {
	it('carries one line per migration, newest order left as the server sent it', () => {
		const log = migrationLog([request({ request: 'a' }), request({ request: 'b' })], 0);
		expect(log.map((entry) => entry.id)).toEqual(['a', 'b']);
	});

	it('states the outcome in the line rather than only in a colour', () => {
		const [entry] = migrationLog([request({ state: 'failed' })], 0);
		expect(entry.what).toContain('Failed');
	});
});

describe('the per-source count', () => {
	it('sums every migration from one marketplace under it', () => {
		const counts = migrationCounts([
			request({ request: 'a', source: 'Tes' }),
			request({ request: 'b', source: 'Tes' })
		]);
		expect(counts).toEqual({ Tes: 2 });
	});

	it('counts no sync request', () => {
		expect(migrationCounts([request({ disposition: 'sync' })])).toEqual({});
	});
});

describe('the tone translation', () => {
	it('renders the request vocabulary’s grey as the pill’s grey', () => {
		expect(pillTone('mut')).toBe('soon');
		expect(pillTone('ok')).toBe('ok');
		expect(pillTone('run')).toBe('run');
		expect(pillTone('bad')).toBe('bad');
	});
});


describe('a migration already under way from the same shop', () => {
	it('is found while the device has not started it', () => {
		const open = openFromSource([request({ state: 'pending', resources_total: 0 })], 'Tes');
		expect(open?.request).toBe('r-1');
		expect(open?.href).toBe('/sync/requests/r-1');
	});

	it('is found while it is reading', () => {
		expect(openFromSource([request({ state: 'draining' })], 'Tes')).not.toBeNull();
	});

	it('is not found once it has imported, so a second migration is allowed', () => {
		expect(openFromSource([request({ state: 'enqueued' })], 'Tes')).toBeNull();
	});

	it('is not found once it has failed, so the seller can try again', () => {
		expect(openFromSource([request({ state: 'failed' })], 'Tes')).toBeNull();
	});

	it('is not found when the shop was read and held nothing', () => {
		const empty = request({ state: 'enqueued', resources_total: 0 });
		expect(openFromSource([empty], 'Tes')).toBeNull();
	});

	it('treats a state it does not recognise as still going, which is the cautious side', () => {
		const strange = request({ state: 'something-new' as never });
		expect(openFromSource([strange], 'Tes')).not.toBeNull();
	});

	it('does not block a different marketplace, which is a different shop', () => {
		const running = request({ source: 'Tes', state: 'draining' });
		expect(openFromSource([running], 'Tpt')).toBeNull();
		expect(openFromSource([running], 'Tes')).not.toBeNull();
	});

	it('ignores a sync request, which is not a migration', () => {
		const synced = request({ disposition: 'sync', state: 'draining' });
		expect(openFromSource([synced], 'Tes')).toBeNull();
	});

	it('answers nothing before a source has been chosen', () => {
		expect(openFromSource([request({ state: 'draining' })], null)).toBeNull();
	});

	it('answers the newest where more than one is somehow open', () => {
		const rows = [
			request({ request: 'older', state: 'draining', created_at: 1 }),
			request({ request: 'newer', state: 'draining', created_at: 2 })
		];
		expect(openFromSource(rows, 'Tes')?.request).toBe('newer');
		expect(openFromSource([...rows].reverse(), 'Tes')?.request).toBe('newer');
	});

	it('names the shop and says what it is doing, so the banner is not a bare refusal', () => {
		const open = openFromSource([request({ state: 'draining' })], 'Tes');
		expect(open?.line).toContain('Tes.com');
		expect(open?.line).toContain('Importing');
	});
});
