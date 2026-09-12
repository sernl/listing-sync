// The `soon` flag's contract, checked against the routes rather than against a
// list somebody keeps by hand.
//
// The old test asserted which items carried the flag, which catches an item
// added to or removed from the set and misses the thing that actually goes
// wrong: a slice makes its page real and nobody drops the flag, so a built page
// keeps advertising itself as unbuilt and the test still passes, because it
// asserts what the set is rather than whether the set is true. This asserts the
// property instead, so a slice that finishes its page breaks the lane until the
// flag goes.

import { readFileSync, existsSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { ALL_DESTINATIONS } from './nav';

const ROUTES = new URL('../routes', import.meta.url).pathname;

/** Destinations flagged `soon` although their route is built.
 *
 * The flag says "you cannot use this yet", which is usually the same as "there
 * is no page yet" and is sometimes not: a page can be finished and still unable
 * to do its job, because the thing it drives has not shipped. Each entry states
 * why in one sentence, and the assertions below refuse an entry that has stopped
 * being true, so the list cannot quietly outlive its reason.
 *
 * Empty since Scheduling landed: its page now saves a schedule a tick actually
 * runs, which is the one exception this list ever held. */
const BUILT_BUT_NOT_USABLE: Record<string, string> = {};

type Verdict = 'built' | 'placeholder' | 'redirect' | 'missing';

/** What the route at this path actually is.
 *
 * "Built" is decided by whether the page reaches for data — a query, the API
 * client, or a page component under `$lib/pages` — because that is what makes
 * a screen show the seller something real. A placeholder reaches for nothing. */
function verdictFor(href: string): Verdict {
	const page = `${ROUTES}${href}/+page.svelte`;
	const loader = `${ROUTES}${href}/+page.ts`;
	if (!existsSync(page)) {
		if (existsSync(loader) && /redirect\(/.test(readFileSync(loader, 'utf8'))) {
			return 'redirect';
		}
		return 'missing';
	}
	const source = readFileSync(page, 'utf8');
	return /createQuery|api\.|\$lib\/pages\//.test(source) ? 'built' : 'placeholder';
}

describe('the soon flag against the routes it describes', () => {
	it('points every destination at a route that exists', () => {
		const nowhere = ALL_DESTINATIONS.filter((item) => verdictFor(item.href) === 'missing');
		expect(nowhere.map((item) => item.href)).toEqual([]);
	});

	it('never flags a page that is built, unless the exception says why', () => {
		const lying = ALL_DESTINATIONS.filter(
			(item) =>
				item.soon === true &&
				verdictFor(item.href) === 'built' &&
				BUILT_BUT_NOT_USABLE[item.href] === undefined
		);
		expect(lying.map((item) => item.href)).toEqual([]);
	});

	// The exception list is held to the same standard as the flags it excuses:
	// an entry that has stopped being true fails here rather than sitting on
	// forever, which is the failure the flags themselves used to have.
	it('keeps no exception that has stopped being true', () => {
		const stale = Object.keys(BUILT_BUT_NOT_USABLE).filter((href) => {
			const item = ALL_DESTINATIONS.find((entry) => entry.href === href);
			return item === undefined || item.soon !== true || verdictFor(href) !== 'built';
		});
		expect(stale).toEqual([]);
	});

	it('gives every exception a reason somebody wrote', () => {
		for (const [href, reason] of Object.entries(BUILT_BUT_NOT_USABLE)) {
			expect(reason.length, href).toBeGreaterThan(30);
			expect(reason.trim().endsWith('.'), href).toBe(true);
		}
	});

	it('never leaves a placeholder unflagged', () => {
		const silent = ALL_DESTINATIONS.filter(
			(item) => item.soon !== true && verdictFor(item.href) === 'placeholder'
		);
		expect(silent.map((item) => item.href)).toEqual([]);
	});

	// Guides was the last placeholder destination, so a `toContain('placeholder')`
	// canary would now fail for the right reason and have to be deleted anyway.
	// `/app` is the redirect the reader has always classified, and a fabricated
	// path is the one verdict no route can accidentally satisfy — together they
	// show the sweep is reading the filesystem rather than answering `built`
	// to everything.
	it('reads real routes, so an empty sweep cannot pass silently', () => {
		const verdicts = ALL_DESTINATIONS.map((item) => verdictFor(item.href));
		expect(verdicts.filter((v) => v === 'built').length).toBeGreaterThan(8);
		expect(verdicts).toContain('redirect');
		expect(verdictFor('/no-such-route')).toBe('missing');
	});
});
