import { describe, expect, it } from 'vitest';
import {
	migrationCounts,
	migrationLog,
	migrationRows,
	migrations,
	openFromSource,
	pillTone,
	seededSource,
	sourceFromQuery
} from './migration';
import { MIGRATE_SOURCES } from '$lib/sync-request';
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
	it('names both shops in full, because three Tes sites differ only by region', () => {
		const [row] = migrationRows([request({ source: 'TesNz' })], 0);
		expect(row.title).toContain('New Zealand');
		expect(row.title).toContain('Teachers Pay Teachers');
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
	it('counts by marketplace rather than by site, because a seller has one Tes login', () => {
		const counts = migrationCounts([
			request({ request: 'a', source: 'TesGb' }),
			request({ request: 'b', source: 'TesNz' })
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
		const open = openFromSource([request({ state: 'pending', resources_total: 0 })], 'TesGb');
		expect(open?.request).toBe('r-1');
		expect(open?.href).toBe('/sync/requests/r-1');
	});

	it('is found while it is reading', () => {
		expect(openFromSource([request({ state: 'draining' })], 'TesGb')).not.toBeNull();
	});

	it('is not found once it has imported, so a second migration is allowed', () => {
		expect(openFromSource([request({ state: 'enqueued' })], 'TesGb')).toBeNull();
	});

	it('is not found once it has failed, so the seller can try again', () => {
		expect(openFromSource([request({ state: 'failed' })], 'TesGb')).toBeNull();
	});

	it('is not found when the shop was read and held nothing', () => {
		const empty = request({ state: 'enqueued', resources_total: 0 });
		expect(openFromSource([empty], 'TesGb')).toBeNull();
	});

	it('treats a state it does not recognise as still going, which is the cautious side', () => {
		const strange = request({ state: 'something-new' as never });
		expect(openFromSource([strange], 'TesGb')).not.toBeNull();
	});

	it('does not block a different Tes site, because three sites are three shops', () => {
		const running = request({ source: 'TesGb', state: 'draining' });
		expect(openFromSource([running], 'TesNz')).toBeNull();
		expect(openFromSource([running], 'TesGb')).not.toBeNull();
	});

	it('ignores a sync request, which is not a migration', () => {
		const synced = request({ disposition: 'sync', state: 'draining' });
		expect(openFromSource([synced], 'TesGb')).toBeNull();
	});

	it('answers nothing before a source has been chosen', () => {
		expect(openFromSource([request({ state: 'draining' })], null)).toBeNull();
	});

	it('answers the newest where more than one is somehow open', () => {
		const rows = [
			request({ request: 'older', state: 'draining', created_at: 1 }),
			request({ request: 'newer', state: 'draining', created_at: 2 })
		];
		expect(openFromSource(rows, 'TesGb')?.request).toBe('newer');
		expect(openFromSource([...rows].reverse(), 'TesGb')?.request).toBe('newer');
	});

	it('names the shop and says what it is doing, so the banner is not a bare refusal', () => {
		const open = openFromSource([request({ state: 'draining' })], 'TesGb');
		expect(open?.line).toContain('United Kingdom');
		expect(open?.line).toContain('Importing');
	});
});

describe('the source a hand-over names', () => {
	it('preselects a site this page offers', () => {
		expect(sourceFromQuery('TesNz', MIGRATE_SOURCES)).toBe('TesNz');
	});

	it('preselects nothing for a value that is not one of them', () => {
		expect(sourceFromQuery('Tpt', MIGRATE_SOURCES)).toBeNull();
		expect(sourceFromQuery('nonsense', MIGRATE_SOURCES)).toBeNull();
		expect(sourceFromQuery(null, MIGRATE_SOURCES)).toBeNull();
	});

	it('preselects nothing for a site the seller has no connection for', () => {
		expect(sourceFromQuery('TesNz', ['TesGb'])).toBeNull();
	});
});


describe('which shop the From field names', () => {
	const OFFERED = MIGRATE_SOURCES;

	it('takes the seller’s own pick over everything else', () => {
		expect(seededSource('TesGb', 'TesNz', OFFERED)).toBe('TesGb');
	});

	it('takes a hand-over where the seller has picked nothing', () => {
		expect(seededSource(null, 'TesNz', OFFERED)).toBe('TesNz');
	});

	it('falls back to the default this page had before seeding existed', () => {
		expect(seededSource(null, null, OFFERED)).toBe('TesGb');
		expect(seededSource(null, null, OFFERED)).toBe(seededSource(null, 'nonsense', OFFERED));
	});

	it('never selects nothing because a link was stale', () => {
		for (const stale of ['nonsense', 'Tpt', 'Etsy', '', 'tesgb']) {
			expect(seededSource(null, stale, OFFERED)).toBe('TesGb');
		}
	});

	it('falls back rather than naming a site this seller cannot migrate from', () => {
		expect(seededSource(null, 'TesNz', ['TesGb'])).toBe('TesGb');
	});

	it('answers nothing only when the seller is offered nothing', () => {
		expect(seededSource(null, 'TesGb', [])).toBeNull();
	});
});
