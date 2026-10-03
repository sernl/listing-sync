import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
	isLostChunk,
	mayReloadForLostChunk,
	START_TIMEOUT_MS,
	START_WATCHDOG
} from './lost-chunk';

function store(): Pick<Storage, 'getItem' | 'setItem'> {
	const held = new Map<string, string>();
	return {
		getItem: (key) => held.get(key) ?? null,
		setItem: (key, value) => void held.set(key, value)
	};
}

describe('a lost chunk', () => {
	it('is recognised in the words each engine uses', () => {
		expect(
			isLostChunk(
				new TypeError(
					'Failed to fetch dynamically imported module: https://dash.teachouse.io/_app/immutable/nodes/50.js'
				)
			)
		).toBe(true);
		expect(isLostChunk(new TypeError('Importing a module script failed.'))).toBe(true);
		expect(isLostChunk(new Error('Unable to preload CSS for /_app/immutable/assets/x.css'))).toBe(
			true
		);
	});

	it('is not any other failure, so a real bug still shows its cause', () => {
		expect(isLostChunk(new TypeError('Object.hasOwn is not a function'))).toBe(false);
		expect(isLostChunk(new TypeError('Failed to fetch'))).toBe(false);
		expect(isLostChunk(null)).toBe(false);
	});

	it('is reloaded for once per window and then shown', () => {
		const held = store();
		expect(mayReloadForLostChunk(held, 1_000)).toBe(true);
		expect(mayReloadForLostChunk(held, 20_000)).toBe(false);
		expect(mayReloadForLostChunk(held, 31_001)).toBe(true);
	});

	it('treats a corrupt record as no record', () => {
		const held = store();
		held.setItem('teachouse.reloaded-for-lost-chunk', 'yesterday');
		expect(mayReloadForLostChunk(held, 5_000)).toBe(true);
	});
});

describe('the start watchdog', () => {
	beforeEach(() => {
		vi.useFakeTimers({ now: 1_000_000 });
	});
	afterEach(() => {
		vi.useRealTimers();
	});

	/** Runs the shell's inline script the way the page does, against a page
	 *  that has drawn something once `drawn` is set, and counts its reloads. */
	function boot(held: Pick<Storage, 'getItem' | 'setItem'>) {
		const page = { drawn: false, reloads: 0 };
		const source = START_WATCHDOG.replace(/^<script>|<\/script>$/g, '');
		new Function('document', 'sessionStorage', 'location', source)(
			{
				querySelector: (selector: string) =>
					selector === 'body > div > :not(script)' && page.drawn ? {} : null
			},
			held,
			{ reload: () => void (page.reloads += 1) }
		);
		return page;
	}

	it('reloads once when nothing has been drawn in time', () => {
		const page = boot(store());
		vi.advanceTimersByTime(START_TIMEOUT_MS - 1);
		expect(page.reloads).toBe(0);
		vi.advanceTimersByTime(1);
		expect(page.reloads).toBe(1);
		vi.advanceTimersByTime(60_000);
		expect(page.reloads).toBe(1);
	});

	it('leaves a page alone once its first screen is drawn', () => {
		const page = boot(store());
		vi.advanceTimersByTime(START_TIMEOUT_MS - 100);
		page.drawn = true;
		vi.advanceTimersByTime(100);
		expect(page.reloads).toBe(0);
	});

	it('does not reload a page that is itself the reload, until the window passes', () => {
		const held = store();
		boot(held);
		vi.advanceTimersByTime(START_TIMEOUT_MS);
		// The reloaded shell stalls again on the same edge.
		const again = boot(held);
		vi.advanceTimersByTime(START_TIMEOUT_MS);
		expect(again.reloads).toBe(0);
		vi.advanceTimersByTime(30_000);
		const later = boot(held);
		vi.advanceTimersByTime(START_TIMEOUT_MS);
		expect(later.reloads).toBe(1);
	});

	it('shares one window with the lost-chunk reload, in both directions', () => {
		const failedFirst = store();
		expect(mayReloadForLostChunk(failedFirst, Date.now())).toBe(true);
		const stalled = boot(failedFirst);
		vi.advanceTimersByTime(START_TIMEOUT_MS);
		expect(stalled.reloads).toBe(0);

		const stalledFirst = store();
		const page = boot(stalledFirst);
		vi.advanceTimersByTime(START_TIMEOUT_MS);
		expect(page.reloads).toBe(1);
		expect(mayReloadForLostChunk(stalledFirst, Date.now())).toBe(false);
	});

	it('reads a corrupt record as no record', () => {
		const held = store();
		held.setItem('teachouse.reloaded-for-lost-chunk', 'yesterday');
		const page = boot(held);
		vi.advanceTimersByTime(START_TIMEOUT_MS);
		expect(page.reloads).toBe(1);
	});

	it('never reloads when storage refuses, since nothing could stop the next one', () => {
		const refused = () => {
			throw new DOMException('The operation is insecure.', 'SecurityError');
		};
		const page = boot({ getItem: refused, setItem: refused });
		vi.advanceTimersByTime(START_TIMEOUT_MS);
		expect(page.reloads).toBe(0);
	});
});
