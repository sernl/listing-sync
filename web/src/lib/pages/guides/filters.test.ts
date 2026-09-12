import { describe, expect, it } from 'vitest';
import type { GuideTaxon } from '$lib/api';
import { filterKey, filterSearch, filtersFromUrl, offered } from './filters';

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
			tags: ['a', 'b']
		});
	});

	it('canonicalises what a hand-edited address can say', () => {
		// One narrowing has to have one cache key, or the same list is read
		// twice and drawn from two entries.
		expect(filtersFromUrl(new URLSearchParams('q=%20vat%20&tags=b,,a&tags=a&topic=%20'))).toEqual({
			q: 'vat',
			topic: null,
			tags: ['a', 'b']
		});
	});

	it('asks for nothing where the address names nothing', () => {
		expect(filtersFromUrl(new URLSearchParams(''))).toEqual({ q: '', topic: null, tags: [] });
	});

	it('writes back exactly what it reads', () => {
		const filters = { q: 'vat & tax', topic: 't1', tags: ['b', 'a'] };
		const written = filterSearch(filters);
		expect(filtersFromUrl(new URLSearchParams(written))).toEqual({
			q: 'vat & tax',
			topic: 't1',
			tags: ['a', 'b']
		});
	});

	it('narrows to a bare path where nothing is narrowed', () => {
		expect(filterSearch({ q: '', topic: null, tags: [] })).toBe('');
		expect(filterKey({ q: '', topic: null, tags: [] })).toBeNull();
	});

	it('keys two narrowings apart and one narrowing together', () => {
		const one = filterKey({ q: 'vat', topic: null, tags: ['b', 'a'] });
		const same = filterKey(filtersFromUrl(new URLSearchParams('tags=a,b&q=vat')));
		expect(one).toBe(same);
		expect(filterKey({ q: 'vat', topic: 't1', tags: [] })).not.toBe(one);
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
