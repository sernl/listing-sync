import { describe, expect, it } from 'vitest';
import type { GuideTaxon } from '$lib/api';
import {
	filterKey,
	filterSearch,
	filtersFromUrl,
	narrowedBy,
	offered,
	pageSummary
} from './filters';

const taxon = (id: string, over: Partial<GuideTaxon> = {}): GuideTaxon => ({
	id,
	slug: id,
	name: id,
	retired: false,
	...over
});

describe('the filters an address asks for', () => {
	it('reads a search, a topic and a comma-joined tag list', () => {
		expect(filtersFromUrl(new URLSearchParams('q=vat&topic=t1&tags=a,b'))).toEqual({
			q: 'vat',
			topic: 't1',
			tags: ['a', 'b'],
			sort: 'title',
			page: 1
		});
	});

	it('canonicalises what a hand-edited address can say', () => {
		// One narrowing has to have one cache key, or the same list is read
		// twice and drawn from two entries.
		expect(filtersFromUrl(new URLSearchParams('q=%20vat%20&tags=b,,a&tags=a&topic=%20'))).toEqual({
			q: 'vat',
			topic: null,
			tags: ['a', 'b'],
			sort: 'title',
			page: 1
		});
	});

	it('asks for nothing where the address names nothing', () => {
		expect(filtersFromUrl(new URLSearchParams(''))).toEqual({
			q: '',
			topic: null,
			tags: [],
			sort: 'title',
			page: 1
		});
	});

	it('writes back exactly what it reads', () => {
		const filters = {
			q: 'vat & tax',
			topic: 't1',
			tags: ['b', 'a'],
			sort: 'newest' as const,
			page: 4
		};
		const written = filterSearch(filters);
		expect(filtersFromUrl(new URLSearchParams(written))).toEqual({
			q: 'vat & tax',
			topic: 't1',
			tags: ['a', 'b'],
			sort: 'newest',
			page: 4
		});
	});

	it('narrows to a bare path where nothing is narrowed', () => {
		// The default order and the first page are absence, so one question has
		// one address: `?sort=title&page=1` is the same place as `/guides`.
		const bare = { q: '', topic: null, tags: [], sort: 'title' as const, page: 1 };
		expect(filterSearch(bare)).toBe('');
		expect(filterKey(bare)).toBeNull();
	});

	it('keys two narrowings apart and one narrowing together', () => {
		const one = filterKey({ q: 'vat', topic: null, tags: ['b', 'a'], sort: 'title', page: 1 });
		const same = filterKey(filtersFromUrl(new URLSearchParams('tags=a,b&q=vat')));
		expect(one).toBe(same);
		expect(filterKey({ q: 'vat', topic: 't1', tags: [], sort: 'title', page: 1 })).not.toBe(one);
	});

	it('keys each page and each order as its own answer', () => {
		// Page two of a search is a different set of rows from page one of it,
		// and an order changes which rows a page holds: a key that dropped
		// either would serve one page's rows as another's.
		const first = filterKey({ q: 'vat', topic: null, tags: [], sort: 'title', page: 1 });
		const second = filterKey({ q: 'vat', topic: null, tags: [], sort: 'title', page: 2 });
		const newest = filterKey({ q: 'vat', topic: null, tags: [], sort: 'newest', page: 1 });
		expect(new Set([first, second, newest]).size).toBe(3);
	});

	it('reads an order it does not offer and a page that is not one as the defaults', () => {
		// The server refuses a sort word outside its two, so reading a
		// hand-edited one as the default here is what keeps this console from
		// asking a question it knows will be refused. A page is a position and
		// is defaulted on both sides.
		for (const address of ['sort=whenever', 'page=0', 'page=-3', 'page=two', 'page=1.5']) {
			const asked = filtersFromUrl(new URLSearchParams(address));
			expect([asked.sort, asked.page], address).toEqual(['title', 1]);
		}
	});

	it('holds a page ordinal no listing will read', () => {
		expect(filtersFromUrl(new URLSearchParams('page=99999999')).page).toBe(10_000);
	});
});

describe('what counts as narrowed', () => {
	it('is the filters, and not the order or the page', () => {
		// This is what "Clear filters" undoes and what tells an empty search
		// from an empty shelf: reordering or paging leaves every guide in the
		// answer.
		expect(narrowedBy({ q: '', topic: null, tags: [], sort: 'newest', page: 6 })).toBe(false);
		expect(narrowedBy({ q: 'vat', topic: null, tags: [], sort: 'title', page: 1 })).toBe(true);
		expect(narrowedBy({ q: '', topic: 't1', tags: [], sort: 'title', page: 1 })).toBe(true);
		expect(narrowedBy({ q: '', topic: null, tags: ['a'], sort: 'title', page: 1 })).toBe(true);
	});
});

describe('which rows of the answer a page is', () => {
	it('counts the rows on the screen against the whole narrowing', () => {
		expect(pageSummary(1, 25, 143)).toBe('1\u201325 of 143 guides');
		expect(pageSummary(2, 25, 143)).toBe('26\u201350 of 143 guides');
		expect(pageSummary(6, 18, 143)).toBe('126\u2013143 of 143 guides');
	});

	it('never reports the page length as the total', () => {
		// The defect this replaces: a server answering 25 rows of 143 reported
		// 25 as the count, so the footer said the corpus was one page long.
		expect(pageSummary(1, 25, 143)).toContain('of 143');
		expect(pageSummary(1, 1, 1)).toBe('1\u20131 of 1 guide');
	});

	it('says an empty set and an empty page differently', () => {
		expect(pageSummary(1, 0, 0)).toBe('No guides');
		expect(pageSummary(9, 0, 143)).toBe('No guides on this page of 143 guides');
	});
});

describe('a retired topic or tag', () => {
	it('stays in a picker the reader is already filtered by', () => {
		// Dropping the chosen value out of its own control is how a filter
		// becomes impossible to clear.
		const taxa = [taxon('a'), taxon('gone', { retired: true })];
		expect(offered(taxa, []).map((one) => one.id)).toEqual(['a']);
		expect(offered(taxa, ['gone']).map((one) => one.id)).toEqual(['a', 'gone']);
	});
});
