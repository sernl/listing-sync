import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	INTENT_KEY,
	parseIntent,
	peekIntent,
	readIntent,
	safeNext,
	writeIntent
} from './intent';

const NOW = Date.UTC(2026, 8, 22, 12);
const HOUR = 60 * 60 * 1000;

/** The browser store, as far as this module uses it. The test environment is
 *  node, which has no `localStorage`, so one is stood up per test. */
function store(seed: Record<string, string> = {}) {
	const held = new Map(Object.entries(seed));
	const fake = {
		getItem: (key: string) => held.get(key) ?? null,
		setItem: (key: string, value: string) => void held.set(key, value),
		removeItem: (key: string) => void held.delete(key)
	};
	vi.stubGlobal('localStorage', fake);
	return held;
}

afterEach(() => {
	vi.unstubAllGlobals();
	vi.useRealTimers();
});

describe('reading a written intent', () => {
	it('answers the price the landing page asked to buy', () => {
		expect(
			parseIntent(
				JSON.stringify({ next: '/settings/subscription', price: 'sync_yearly', at: NOW - HOUR }),
				NOW
			)
		).toEqual({ next: '/settings/subscription', price: 'sync_yearly', at: NOW - HOUR });
	});

	// A key left behind by a sign-up nobody finished must not open a checkout
	// weeks later, when the seller has no idea what asked for it.
	it('drops a record older than a day', () => {
		const raw = JSON.stringify({ next: null, price: 'sync_yearly', at: NOW - 25 * HOUR });
		expect(parseIntent(raw, NOW)).toBeNull();
	});

	it('keeps one written just inside the day', () => {
		const raw = JSON.stringify({ next: null, price: 'sync_yearly', at: NOW - 23 * HOUR });
		expect(parseIntent(raw, NOW)?.price).toBe('sync_yearly');
	});

	// The key is a string in another tab's storage. A price this deployment
	// does not sell would open a checkout the API refuses.
	it('drops a price that is not one of ours, and the record with it', () => {
		const raw = JSON.stringify({ next: null, price: 'sync_lifetime', at: NOW });
		expect(parseIntent(raw, NOW)).toBeNull();
	});

	it('keeps a landing page even where the price was dropped', () => {
		const raw = JSON.stringify({ next: '/resources', price: 'made_up', at: NOW });
		expect(parseIntent(raw, NOW)).toEqual({ next: '/resources', price: null, at: NOW });
	});

	it('reads nothing from an absent, malformed or undated record', () => {
		expect(parseIntent(null, NOW)).toBeNull();
		expect(parseIntent('{', NOW)).toBeNull();
		expect(parseIntent('"sync_yearly"', NOW)).toBeNull();
		expect(parseIntent(JSON.stringify({ price: 'sync_yearly' }), NOW)).toBeNull();
		expect(parseIntent(JSON.stringify({ price: 'sync_yearly', at: 'now' }), NOW)).toBeNull();
	});
});

describe('where the intent says to land', () => {
	it('takes a path on this origin', () => {
		expect(safeNext('/settings/subscription')).toBe('/settings/subscription');
	});

	// `next` arrives as a query parameter, so a value that leaves this origin
	// would turn our own sign-in into somebody else's redirector.
	it('refuses anything that leaves this origin', () => {
		expect(safeNext('https://evil.test/take')).toBeNull();
		expect(safeNext('//evil.test/take')).toBeNull();
		expect(safeNext('settings/subscription')).toBeNull();
		expect(safeNext(null)).toBeNull();
	});
});

describe('the stored intent', () => {
	it('is written under the key the plan page reads', () => {
		const held = store();
		vi.useFakeTimers();
		vi.setSystemTime(NOW);
		writeIntent({ next: '/settings/subscription', price: 'sync_monthly' });
		expect(JSON.parse(held.get(INTENT_KEY) ?? 'null')).toEqual({
			next: '/settings/subscription',
			price: 'sync_monthly',
			at: NOW
		});
	});

	it('writes nothing where the link asked for nothing', () => {
		const held = store();
		writeIntent({ next: null, price: null });
		expect(held.has(INTENT_KEY)).toBe(false);
	});

	// Reading spends it: a refused checkout, a reload or a second visit must
	// not send the seller back into Stripe.
	it('is spent by the read that answers it', () => {
		const held = store({
			[INTENT_KEY]: JSON.stringify({ next: null, price: 'pack_20', at: Date.now() })
		});
		expect(readIntent()?.price).toBe('pack_20');
		expect(held.has(INTENT_KEY)).toBe(false);
		expect(readIntent()).toBeNull();
	});

	// The router asks which page the sign-in was for. The checkout named in
	// the same record is the plan page's to spend, not its.
	it('is left standing by a look that only asks where to land', () => {
		const held = store({
			[INTENT_KEY]: JSON.stringify({
				next: '/settings/subscription',
				price: 'sync_yearly',
				at: Date.now()
			})
		});
		expect(peekIntent()?.next).toBe('/settings/subscription');
		expect(held.has(INTENT_KEY)).toBe(true);
		expect(readIntent()?.price).toBe('sync_yearly');
	});

	it('is cleared even where it no longer parses', () => {
		const held = store({ [INTENT_KEY]: 'not json' });
		expect(readIntent()).toBeNull();
		expect(held.has(INTENT_KEY)).toBe(false);
	});

	// Private browsing refuses storage. A sign-up that cannot be recorded is
	// still a sign-up, and a plan page that cannot read is still a plan page.
	it('survives a browser with no store at all', () => {
		vi.stubGlobal('localStorage', {
			getItem: () => {
				throw new Error('refused');
			},
			setItem: () => {
				throw new Error('refused');
			},
			removeItem: () => {
				throw new Error('refused');
			}
		});
		expect(() => writeIntent({ next: null, price: 'sync_yearly' })).not.toThrow();
		expect(readIntent()).toBeNull();
	});
});
