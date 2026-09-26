import { describe, expect, it } from 'vitest';
import type { AdminUserView, ImpersonationView, OrgSummaryView } from '$lib/api';
import {
	drainVerdict,
	guideImages,
	impersonationKey,
	openImpersonations,
	orgInFilter,
	orgMatches,
	orgRows
} from './admin-view';

const org = (id: string, over: Partial<OrgSummaryView> = {}): OrgSummaryView => ({
	org: id,
	name: `Org ${id}`,
	slug: null,
	created_at: 0,
	products: 0,
	mappings: 0,
	connections: 1,
	users: 1,
	...over
});

const user = (orgId: string, over: Partial<AdminUserView> = {}): AdminUserView => ({
	user: `u-${orgId}`,
	email: `${orgId}@example.test`,
	organisation: { org: orgId, name: `Org ${orgId}` },
	plan: 'free',
	created_at: 0,
	...over
});

describe('orgRows', () => {
	it('takes the latest sign-in of any member and leaves an org nobody lists unknown', () => {
		const rows = orgRows(
			[org('a'), org('b')],
			[
				user('a', { plan: 'studio', last_sign_in_at: 10 }),
				user('a', { plan: 'studio', last_sign_in_at: 30 }),
				user('a', { plan: 'studio' })
			]
		);
		expect(rows[0]).toMatchObject({ plan: 'studio', lastSeen: 30 });
		expect(rows[1]).toMatchObject({ plan: null, lastSeen: null });
	});
});

describe('org filters', () => {
	const [paid, free, unknown] = orgRows(
		[org('p'), org('f', { connections: 0, slug: 'miso' }), org('u')],
		[user('p', { plan: 'subscriber' }), user('f')]
	);

	it('puts an org of unknown plan under neither Paid nor Free', () => {
		expect([paid, free, unknown].map((row) => orgInFilter(row, 'paid'))).toEqual([true, false, false]);
		expect([paid, free, unknown].map((row) => orgInFilter(row, 'free'))).toEqual([false, true, false]);
		expect(orgInFilter(free, 'no-shop')).toBe(true);
		expect(orgInFilter(paid, 'no-shop')).toBe(false);
	});

	it('searches name, slug and id without regard to case', () => {
		expect(orgMatches(free, '  MISO ')).toBe(true);
		expect(orgMatches(paid, 'org P')).toBe(true);
		expect(orgMatches(paid, 'miso')).toBe(false);
		expect(orgMatches(paid, '')).toBe(true);
	});
});

describe('openImpersonations', () => {
	const event = (kind: string, at: number, actor = 'op', target = 't'): ImpersonationView => ({
		event: kind,
		actor,
		target,
		at
	});

	it('keeps a start no later stop answers, newest-first input or not', () => {
		const closed = event('user_impersonated', 1);
		const stop = event('impersonation_stopped', 2);
		const live = event('user_impersonated', 3);
		const other = event('user_impersonated', 4, 'op', 'other');
		const open = openImpersonations([other, live, stop, closed]);
		expect([...open].sort()).toEqual([impersonationKey(live), impersonationKey(other)].sort());
	});

	it('closes nothing with a stop that came before the start', () => {
		const start = event('user_impersonated', 5);
		expect(openImpersonations([start, event('impersonation_stopped', 4)]).has(impersonationKey(start))).toBe(
			true
		);
	});
});

describe('guideImages', () => {
	it('lists each picture once, uploaded or remote, with its alt text', () => {
		const body = [
			'![Setup](/v1/guides/images/abc)',
			'text ![](https://example.test/a.png "title") more',
			'![again](/v1/guides/images/abc)',
			'[a link](/v1/guides/images/not-an-image)'
		].join('\n');
		expect(guideImages(body)).toEqual([
			{ src: '/v1/guides/images/abc', alt: 'Setup', own: true },
			{ src: 'https://example.test/a.png', alt: '', own: false }
		]);
	});
});

describe('drainVerdict', () => {
	it('reads a fall as draining and anything else as not', () => {
		const base = { first: null, tenth: null };
		expect(drainVerdict({ ...base, fall: 0.2, gap: null })).toBe('draining');
		expect(drainVerdict({ ...base, fall: 0, gap: null })).toBe('flat');
		expect(drainVerdict({ ...base, fall: null, gap: 'short' })).toBe('short');
		expect(drainVerdict({ ...base, fall: null, gap: 'unmeasurable' })).toBe('unmeasurable');
	});
});
