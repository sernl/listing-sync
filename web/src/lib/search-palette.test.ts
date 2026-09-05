import { describe, expect, it } from 'vitest';
import type { MappingHead, ProductHead } from '$lib/api';
import {
	RESULTS_SHOWN_MAX,
	clampHighlight,
	coverToDraw,
	isStale,
	keyAction,
	opensPalette,
	paletteView,
	rankMatches,
	resultCount,
	resultHref,
	showingByProduct
} from './search-palette';

function product(over: Partial<ProductHead> & { id: string; title: string }): ProductHead {
	return {
		price: 'Free',
		created_at: 1,
		updated_at: 1,
		...over
	};
}

function mapping(over: Partial<MappingHead> & { product: string }): MappingHead {
	return {
		id: `m-${over.product}-${over.inventory ?? 'Tpt'}`,
		inventory: 'Tpt',
		binding_state: 'bound',
		lifecycle_state: 'live',
		updated_at: 1,
		listing_url: null,
		...over
	};
}

const CATALOGUE: ProductHead[] = [
	product({ id: 'a', title: 'Fractions warm-up', updated_at: 10 }),
	product({ id: 'b', title: 'Poetry unit', updated_at: 30 }),
	product({ id: 'c', title: 'Advanced fractions', updated_at: 20 })
];

describe('what a key does to an open palette', () => {
	it('closes on Escape even when nothing matched', () => {
		expect(keyAction({ key: 'Escape', count: 0, highlighted: 0, isComposing: false })).toEqual({ kind: 'close' });
	});

	it('closes on Escape with results showing', () => {
		expect(keyAction({ key: 'Escape', count: 3, highlighted: 1, isComposing: false })).toEqual({ kind: 'close' });
	});

	it('walks down the list', () => {
		expect(keyAction({ key: 'ArrowDown', count: 3, highlighted: 0, isComposing: false })).toEqual({
			kind: 'move',
			to: 1
		});
	});

	it('wraps from the last result to the first', () => {
		expect(keyAction({ key: 'ArrowDown', count: 3, highlighted: 2, isComposing: false })).toEqual({
			kind: 'move',
			to: 0
		});
	});

	it('wraps from the first result to the last', () => {
		expect(keyAction({ key: 'ArrowUp', count: 3, highlighted: 0, isComposing: false })).toEqual({
			kind: 'move',
			to: 2
		});
	});

	it('opens the highlighted result on Enter', () => {
		expect(keyAction({ key: 'Enter', count: 3, highlighted: 2, isComposing: false })).toEqual({
			kind: 'open',
			index: 2
		});
	});

	// The one that would navigate to nothing: a seller who types a query that
	// matches none and presses Enter must stay where they are.
	it('opens nothing on Enter when nothing matched', () => {
		expect(keyAction({ key: 'Enter', count: 0, highlighted: 0, isComposing: false })).toEqual({ kind: 'ignore' });
	});

	// A stale highlight from the previous answer must not open the row it
	// happens to land on.
	it('opens the last result rather than a stale index past the end', () => {
		expect(keyAction({ key: 'Enter', count: 2, highlighted: 7, isComposing: false })).toEqual({
			kind: 'open',
			index: 1
		});
	});

	it('ignores the arrows when nothing matched', () => {
		expect(keyAction({ key: 'ArrowDown', count: 0, highlighted: 0, isComposing: false })).toEqual({ kind: 'ignore' });
		expect(keyAction({ key: 'ArrowUp', count: 0, highlighted: 0, isComposing: false })).toEqual({ kind: 'ignore' });
	});

	// Home and End are in this list rather than moving the highlight: the
	// caret owns them in an editable combobox, and a seller pressing Home to
	// reach a typo at the start of what they typed must get the caret, not a
	// silent jump to the first result.
	it('leaves every other key to the input, Home and End included', () => {
		for (const key of ['a', ' ', 'Tab', 'Backspace', 'ArrowLeft', 'PageDown', 'Home', 'End']) {
			expect(keyAction({ key, count: 3, highlighted: 0, isComposing: false })).toEqual({
				kind: 'ignore'
			});
		}
	});

	// Composing on a Japanese or Chinese keyboard, the arrows walk the
	// candidate list and Enter commits the candidate. Acting on either would
	// move the highlight under the seller and open a resource they never chose,
	// and Escape mid-composition would throw away what they had typed.
	it('leaves a composing Enter, arrow or Escape to the input method', () => {
		for (const key of ['Enter', 'ArrowDown', 'ArrowUp', 'Escape']) {
			expect(keyAction({ key, count: 3, highlighted: 1, isComposing: true })).toEqual({
				kind: 'ignore'
			});
		}
	});

	it('acts on those same keys once the composition is committed', () => {
		expect(keyAction({ key: 'Enter', count: 3, highlighted: 1, isComposing: false })).toEqual({
			kind: 'open',
			index: 1
		});
		expect(keyAction({ key: 'Escape', count: 3, highlighted: 1, isComposing: false })).toEqual({
			kind: 'close'
		});
	});
});

describe('the highlight held across a changing list', () => {
	it('is the first row for an empty list', () => {
		expect(clampHighlight(4, 0)).toBe(0);
	});

	it('is pulled back inside a list that shrank', () => {
		expect(clampHighlight(9, 3)).toBe(2);
	});

	it('is left alone inside the list', () => {
		expect(clampHighlight(1, 3)).toBe(1);
	});

	it('refuses a negative or unreal index', () => {
		expect(clampHighlight(-2, 3)).toBe(0);
		expect(clampHighlight(Number.NaN, 3)).toBe(0);
	});
});

describe('the key that opens the palette', () => {
	it('opens on ctrl-K and on cmd-K, in either case', () => {
		expect(opensPalette({ key: 'k', ctrlKey: true, metaKey: false })).toBe(true);
		expect(opensPalette({ key: 'K', ctrlKey: false, metaKey: true })).toBe(true);
	});

	it('leaves a bare k to whatever is typing', () => {
		expect(opensPalette({ key: 'k', ctrlKey: false, metaKey: false })).toBe(false);
		expect(opensPalette({ key: 'j', ctrlKey: true, metaKey: false })).toBe(false);
	});
});

describe('what the popup says', () => {
	it('says nothing at all before anything is typed', () => {
		expect(
			paletteView({ query: '  ', catalogue: CATALOGUE, mappings: [], failed: false })
		).toEqual({ kind: 'blank' });
	});

	// The distinction the sweep note asks for: an unread catalogue is not an
	// empty one, and a seller who sees "no matches" over a failed read is being
	// told something this client cannot support.
	it('separates a catalogue not yet read from one holding no match', () => {
		expect(
			paletteView({ query: 'fractions', catalogue: null, mappings: [], failed: false })
		).toEqual({ kind: 'reading' });
		expect(
			paletteView({ query: 'nothing here', catalogue: CATALOGUE, mappings: [], failed: false })
		).toEqual({ kind: 'none', query: 'nothing here', stale: false });
	});

	it('reports a failed read only where there is nothing to search', () => {
		expect(paletteView({ query: 'fractions', catalogue: null, mappings: [], failed: true })).toEqual(
			{ kind: 'failed' }
		);
	});

	// A failed refetch keeps the rows it already holds: `isError` and cached
	// data co-occur in query-core, and refusing to search a catalogue sitting
	// in memory is a worse answer than the "no matches" this popup will not
	// fabricate.
	it('still searches a catalogue it holds when the newest read failed', () => {
		const view = paletteView({
			query: 'fractions',
			catalogue: CATALOGUE,
			mappings: [],
			failed: true
		});
		expect(view.kind).toBe('results');
		if (view.kind !== 'results') {
			return;
		}
		expect(view.rows.map((row) => row.id)).toEqual(['a', 'c']);
		expect(view.stale).toBe(true);
	});

	it('marks an empty answer stale too, so "no match" is not read as settled', () => {
		expect(
			paletteView({ query: 'nothing here', catalogue: CATALOGUE, mappings: [], failed: true })
		).toEqual({ kind: 'none', query: 'nothing here', stale: true });
	});

	it('calls an answer stale only where a read actually failed', () => {
		expect(isStale(paletteView({ query: 'fractions', catalogue: CATALOGUE, mappings: [], failed: true }))).toBe(true);
		expect(isStale(paletteView({ query: 'fractions', catalogue: CATALOGUE, mappings: [], failed: false }))).toBe(false);
		expect(isStale({ kind: 'failed' })).toBe(false);
		expect(isStale({ kind: 'reading' })).toBe(false);
		expect(isStale({ kind: 'blank' })).toBe(false);
	});

	it('finds a match anywhere in the title, whatever the case', () => {
		const view = paletteView({
			query: 'FRACT',
			catalogue: CATALOGUE,
			mappings: [],
			failed: false
		});
		expect(view.kind).toBe('results');
		if (view.kind !== 'results') {
			return;
		}
		expect(view.rows.map((row) => row.id)).toEqual(['a', 'c']);
	});

	// The cover is the server's URL verbatim. Composing it here would be a
	// second place the route is written down, and it would get the version
	// wrong the first time one changed.
	it('carries the cover URL the list view named, unchanged', () => {
		const view = paletteView({
			query: 'poetry',
			catalogue: [
				product({
					id: 'b',
					title: 'Poetry unit',
					cover: '/v1/products/b/cover'
				})
			],
			mappings: [],
			failed: false
		});
		expect(view.kind).toBe('results');
		if (view.kind !== 'results') {
			return;
		}
		expect(view.rows[0].cover).toBe('/v1/products/b/cover');
	});

	// Absent and null are one answer: neither has bytes behind it, so the row
	// draws its placeholder rather than requesting a cover that is not there.
	it('answers null for a resource with no cover, however the field is absent', () => {
		const view = paletteView({
			query: 'poetry',
			catalogue: [
				product({ id: 'missing', title: 'Poetry unit' }),
				product({ id: 'nulled', title: 'Poetry seminar', cover: null })
			],
			mappings: [],
			failed: false
		});
		expect(view.kind).toBe('results');
		if (view.kind !== 'results') {
			return;
		}
		expect(view.rows.map((row) => row.cover)).toEqual([null, null]);
	});

	it('opens a result at the resource, not at the board filtered to it', () => {
		expect(resultHref('abc-123')).toBe('/inventory/abc-123');
	});

	it('names the marketplaces showing a resource, and stays silent about the rest', () => {
		const view = paletteView({
			query: 'poetry',
			catalogue: CATALOGUE,
			mappings: [
				mapping({ product: 'b', inventory: 'TesGb' }),
				mapping({ product: 'b', inventory: 'Tpt' }),
				// Bound but not live: the marketplace is not showing it, so it is
				// not named as though it were.
				mapping({ product: 'b', inventory: 'Etsy', lifecycle_state: 'draft' })
			],
			failed: false
		});
		expect(view.kind).toBe('results');
		if (view.kind !== 'results') {
			return;
		}
		expect(view.rows[0].meta).toBe('Free · TPT, TES GB');
	});

	it('says nothing about marketplaces where the mappings were not read', () => {
		const view = paletteView({
			query: 'poetry',
			catalogue: CATALOGUE,
			mappings: null,
			failed: false
		});
		expect(view.kind).toBe('results');
		if (view.kind !== 'results') {
			return;
		}
		expect(view.rows[0].meta).toBe('Free');
	});

	it('counts every match, not only the ones it shows', () => {
		const many = Array.from({ length: RESULTS_SHOWN_MAX + 5 }, (_, index) =>
			product({ id: `p${index}`, title: `Fractions ${index}`, updated_at: index })
		);
		const view = paletteView({ query: 'fractions', catalogue: many, mappings: [], failed: false });
		expect(view.kind).toBe('results');
		if (view.kind !== 'results') {
			return;
		}
		expect(view.rows).toHaveLength(RESULTS_SHOWN_MAX);
		expect(view.total).toBe(RESULTS_SHOWN_MAX + 5);
	});
});

describe('the cover a row actually draws', () => {
	it('draws the cover while nothing has failed', () => {
		expect(coverToDraw('/v1/products/a/cover', new Set())).toBe('/v1/products/a/cover');
	});

	// The fallback: a deleted or unreachable cover draws the same placeholder
	// the absent case draws, rather than the browser's broken-image glyph.
	it('draws nothing for a cover that failed to load', () => {
		expect(coverToDraw('/v1/products/a/cover', new Set(['/v1/products/a/cover']))).toBeNull();
	});

	// Keyed by URL rather than by row, so a row that gains a new cover is tried
	// afresh instead of being punished for the URL that failed.
	it('tries a different URL on a row whose earlier cover failed', () => {
		const failed = new Set(['/v1/products/a/cover']);
		expect(coverToDraw('/v1/products/a/cover-2', failed)).toBe('/v1/products/a/cover-2');
	});

	it('leaves a resource with no cover alone', () => {
		expect(coverToDraw(null, new Set())).toBeNull();
		expect(coverToDraw(null, new Set(['/v1/products/a/cover']))).toBeNull();
	});
});

describe('the order matches come back in', () => {
	it('leads with titles that begin with the query', () => {
		expect(rankMatches(CATALOGUE, 'fractions').map((row) => row.id)).toEqual(['a', 'c']);
	});

	it('breaks a tie by the most recently updated, as the board does', () => {
		const rows = [
			product({ id: 'old', title: 'Unit plan', updated_at: 5 }),
			product({ id: 'new', title: 'Unit plan', updated_at: 50 })
		];
		expect(rankMatches(rows, 'unit').map((row) => row.id)).toEqual(['new', 'old']);
	});

	it('answers the whole catalogue for a blank query, in the same order', () => {
		expect(rankMatches(CATALOGUE, '').map((row) => row.id)).toEqual(['b', 'c', 'a']);
	});
});

describe('how many results a key can move through', () => {
	it('is zero for every state that shows none', () => {
		expect(resultCount({ kind: 'blank' })).toBe(0);
		expect(resultCount({ kind: 'reading' })).toBe(0);
		expect(resultCount({ kind: 'failed' })).toBe(0);
		expect(resultCount({ kind: 'none', query: 'x', stale: false })).toBe(0);
	});

	it('is the rows shown for a list of results', () => {
		expect(
			resultCount({
				kind: 'results',
				rows: [{ id: 'a', title: 'A', meta: '', href: '/inventory/a', cover: null }],
				total: 1,
				stale: false
			})
		).toBe(1);
	});
});

describe('which marketplaces show a resource', () => {
	it('is empty for a resource nothing carries live', () => {
		const showing = showingByProduct([mapping({ product: 'a', binding_state: 'unbound' })]);
		expect(showing.get('a')).toBeUndefined();
	});

	it('names each marketplace once, in the chip strip order', () => {
		const showing = showingByProduct([
			mapping({ product: 'a', inventory: 'TesUs' }),
			mapping({ product: 'a', inventory: 'Tpt' })
		]);
		expect(showing.get('a')).toEqual(['TPT', 'TES US']);
	});
});
