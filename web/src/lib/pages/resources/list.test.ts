import { describe, expect, it } from 'vitest';
import {
	DEFAULT_TAB,
	MARKETPLACE_TILES,
	PILL_TONE,
	NO_RESOURCE_FILTERS,
	SORTS,
	STANDING_OPTIONS,
	VERB_PHRASE,
	countsFor,
	filterSearch,
	filtersActive,
	fullStop,
	inTab,
	labelsFromUrl,
	listedOn,
	matchesResource,
	metaLine,
	needsYou,
	sortRows,
	standingOfRow,
	tabCounts,
	unionById
} from './list';
import { STATE_LABEL, STATE_TONE, type InventoryRow, type MarketplaceChip } from '$lib/inventory';
import type { MappingHead, ProductHead } from '$lib/api';
import type { InventoryId } from '$lib/generated/vocab';

function chip(inventory: InventoryId, state: MarketplaceChip['state']): MarketplaceChip {
	return {
		inventory,
		state,
		label: STATE_LABEL[state],
		tone: STATE_TONE[state],
		detail: 'A sentence the row does not read.',
		action: null,
		onDevice: inventory !== 'Etsy',
		paused: null
	};
}

function product(partial: Partial<ProductHead> = {}): ProductHead {
	return {
		id: 'p1',
		title: 'Fractions pack',
		price: 'Free',
		created_at: 1,
		updated_at: 2,
		...partial
	};
}

/** A row as `rowFor` builds one: the mappings are the marketplaces that hold
 *  the resource, which is what a marketplace filter reads. */
function row(chips: MarketplaceChip[], head: Partial<ProductHead> = {}): InventoryRow {
	const stored = product(head);
	const mapped = new Map<InventoryId, MappingHead>(
		chips
			.filter((entry) => entry.state !== 'not_listed')
			.map((entry) => [
				entry.inventory,
				{
					id: `m-${entry.inventory}`,
					product: stored.id,
					inventory: entry.inventory,
					binding_state: 'bound',
					lifecycle_state: 'live',
					updated_at: 1,
					listing_url: null
				}
			])
	);
	return { product: stored, chips, attention: [], mapped };
}

describe('the tab partition', () => {
	it('reads listed when any marketplace shows the resource', () => {
		const one = row([chip('Tpt', 'draft'), chip('Tes', 'listed')]);
		expect(standingOfRow(one)).toBe('listed');
	});

	it('reads draft only when nothing is showing it', () => {
		expect(standingOfRow(row([chip('Tpt', 'draft'), chip('Tes', 'not_listed')]))).toBe('draft');
	});

	it('reads not listed when no marketplace holds it', () => {
		expect(standingOfRow(row([chip('Tpt', 'not_listed')]))).toBe('not_listed');
	});

	it('counts a resource needing a person in its own tab and in its standing tab', () => {
		const counts = tabCounts([
			row([chip('Tpt', 'listed'), chip('Tes', 'failed')]),
			row([chip('Tpt', 'draft')]),
			row([chip('Tpt', 'not_listed')])
		]);
		expect(counts).toEqual({ all: 3, listed: 1, draft: 1, not_listed: 1, attention: 1 });
	});

	it('holds every resource in the segment the board opens on', () => {
		const rows = [row([chip('Tpt', 'listed')]), row([chip('Tpt', 'draft')])];
		expect(tabCounts(rows).all).toBe(2);
		expect(rows.every((entry) => inTab(entry, 'all'))).toBe(true);
		expect(DEFAULT_TAB).toBe('all');
	});

	it('states no figure at all until the catalogue has been read', () => {
		const rows = [row([chip('Tpt', 'listed')])];
		expect(countsFor(rows, false)).toEqual({
			all: null,
			not_listed: null,
			draft: null,
			listed: null,
			attention: null
		});
		expect(countsFor(rows, true)).toEqual(tabCounts(rows));
	});

	it('does not stand zero in for an unread catalogue', () => {
		const unread = countsFor([], false);
		const empty = countsFor([], true);
		expect(unread.listed).toBeNull();
		expect(empty.listed).toBe(0);
	});

	it('treats every attention state as needing a person', () => {
		expect(needsYou(row([chip('Tpt', 'needs_signin')]))).toBe(true);
		expect(needsYou(row([chip('Tpt', 'stranded')]))).toBe(true);
		expect(needsYou(row([chip('Tpt', 'blocked')]))).toBe(true);
		expect(needsYou(row([chip('Tpt', 'in_flight')]))).toBe(false);
	});

	it('puts a listed-and-failing resource in both the listed and the needs-you tab', () => {
		const both = row([chip('Tpt', 'listed'), chip('Tes', 'failed')]);
		expect(inTab(both, 'listed')).toBe(true);
		expect(inTab(both, 'attention')).toBe(true);
		expect(inTab(both, 'draft')).toBe(false);
	});
});

describe('the filter card', () => {
	const listedOnTpt = row([chip('Tpt', 'listed'), chip('Tes', 'failed')], {
		title: 'Fractions pack'
	});

	it('lets everything through with nothing set', () => {
		expect(matchesResource(listedOnTpt, NO_RESOURCE_FILTERS)).toBe(true);
	});

	it('reads several marketplaces as any of them', () => {
		const failingSomewhere = {
			...NO_RESOURCE_FILTERS,
			marketplaces: ['Tes', 'Tpt'] as InventoryId[],
			standing: 'failed' as const
		};
		expect(matchesResource(listedOnTpt, failingSomewhere)).toBe(true);
	});

	it('does not match a state held on a marketplace outside the selection', () => {
		const failingOnTpt = {
			...NO_RESOURCE_FILTERS,
			marketplaces: ['Tpt'] as InventoryId[],
			standing: 'failed' as const
		};
		expect(matchesResource(listedOnTpt, failingOnTpt)).toBe(false);
	});

	it('narrows on the query and the marketplace at once', () => {
		const onTes = {
			...NO_RESOURCE_FILTERS,
			marketplaces: ['Tes'] as InventoryId[],
			query: 'fractions'
		};
		expect(matchesResource(listedOnTpt, onTes)).toBe(true);
		expect(matchesResource(listedOnTpt, { ...onTes, query: 'decimals' })).toBe(false);
	});

	it('applies the tab and the standing together', () => {
		const draftAndNeedy = {
			...NO_RESOURCE_FILTERS,
			tab: 'draft' as const,
			standing: 'attention' as const
		};
		expect(matchesResource(listedOnTpt, draftAndNeedy)).toBe(false);
		expect(
			matchesResource(row([chip('Tpt', 'draft'), chip('Tes', 'failed')]), draftAndNeedy)
		).toBe(true);
	});

	it('searches the title case-insensitively', () => {
		expect(matchesResource(listedOnTpt, { ...NO_RESOURCE_FILTERS, query: 'FRACTIONS' })).toBe(
			true
		);
		expect(matchesResource(listedOnTpt, { ...NO_RESOURCE_FILTERS, query: 'decimals' })).toBe(
			false
		);
	});

	it('knows whether the clear control has anything to clear', () => {
		expect(filtersActive(NO_RESOURCE_FILTERS)).toBe(false);
		expect(filtersActive(NO_RESOURCE_FILTERS, ['year 5'])).toBe(true);
		expect(filtersActive({ ...NO_RESOURCE_FILTERS, query: '  ' })).toBe(false);
		expect(filtersActive({ ...NO_RESOURCE_FILTERS, tab: 'listed' })).toBe(true);
	});

	it('counts a chosen marketplace and a chosen standing as set', () => {
		expect(
			filtersActive({ ...NO_RESOURCE_FILTERS, marketplaces: ['Tpt'] as InventoryId[] })
		).toBe(true);
		expect(filtersActive({ ...NO_RESOURCE_FILTERS, standing: 'failed' })).toBe(true);
	});

	it('offers the marketplaces in the order the specification names', () => {
		expect(MARKETPLACE_TILES.map((tile) => tile.label)).toEqual(['TES', 'TPT', 'Etsy']);
	});

	it('offers Etsy and refuses it, with the reason on the tile', () => {
		const etsy = MARKETPLACE_TILES.find((tile) => tile.inventory === 'Etsy');
		expect(etsy?.disabled).toBe(true);
		expect(etsy?.reason).toContain('coming soon');
		expect(MARKETPLACE_TILES.filter((tile) => tile.disabled)).toHaveLength(1);
	});

	it('names every state the chips can hold in the standing select', () => {
		const named = STANDING_OPTIONS.map((option) => option.label);
		for (const label of Object.values(STATE_LABEL)) {
			expect(named).toContain(label);
		}
	});
});

describe('the row', () => {
	it('reads its meta line as age, price and who is showing it', () => {
		const now = 4 * 24 * 60 * 60 * 1000;
		const showing = row([chip('Tes', 'listed'), chip('Tpt', 'listed'), chip('Etsy', 'draft')], {
			updated_at: now - 3 * 24 * 60 * 60 * 1000,
			price: { Paid: { minor_units: 450, currency: 'Gbp' } }
		});
		expect(metaLine(showing.product, showing, now)).toBe(
			'Updated 3 days ago · £4.50 · TES, TPT'
		);
	});

	it('says nothing about marketplaces when none is showing it', () => {
		const quiet = row([chip('Tpt', 'draft')], { updated_at: 0, price: 'Free' });
		expect(metaLine(quiet.product, quiet, 0)).toBe('Updated just now · Free');
		expect(listedOn(quiet)).toEqual([]);
	});
});

describe('the ordering', () => {
	const rows = [
		row([], { id: 'a', title: 'Beta', created_at: 3, updated_at: 1 }),
		row([], { id: 'b', title: 'alpha', created_at: 1, updated_at: 3 }),
		row([], { id: 'c', title: 'Gamma', created_at: 2, updated_at: 2 })
	];

	it('defaults to the newest change first', () => {
		expect(sortRows(rows, 'updated_desc').map((entry) => entry.product.id)).toEqual([
			'b',
			'c',
			'a'
		]);
	});

	it('orders by title without regard to case', () => {
		expect(sortRows(rows, 'title_asc').map((entry) => entry.product.title)).toEqual([
			'alpha',
			'Beta',
			'Gamma'
		]);
	});

	it('leaves the rows it was given alone', () => {
		sortRows(rows, 'created_desc');
		expect(rows.map((entry) => entry.product.id)).toEqual(['a', 'b', 'c']);
	});

	it('orders oldest change first, and newest creation first', () => {
		expect(sortRows(rows, 'updated_asc').map((entry) => entry.product.id)).toEqual([
			'a',
			'c',
			'b'
		]);
		expect(sortRows(rows, 'created_desc').map((entry) => entry.product.id)).toEqual([
			'a',
			'c',
			'b'
		]);
	});

	it('names an order for every option it offers', () => {
		for (const option of SORTS) {
			expect(sortRows(rows, option.id)).toHaveLength(rows.length);
		}
	});
});

describe('several label-filtered reads', () => {
	it('keeps a resource carrying two of the chosen labels once', () => {
		const first = [product({ id: 'a' }), product({ id: 'b' })];
		const second = [product({ id: 'b' }), product({ id: 'c' })];
		expect(unionById([first, second]).map((entry) => entry.id)).toEqual(['a', 'b', 'c']);
	});

	it('answers nothing for no reads at all', () => {
		expect(unionById([])).toEqual([]);
	});
});

describe('the filters the URL carries', () => {
	it('reads every label parameter, trimmed', () => {
		const params = new URLSearchParams('q=fractions&label=year%205&label=%20maths%20');
		expect(labelsFromUrl(params)).toEqual(['year 5', 'maths']);
	});

	it('drops blanks and repeats rather than asking twice', () => {
		expect(labelsFromUrl(new URLSearchParams('label=maths&label=&label=maths'))).toEqual([
			'maths'
		]);
	});

	it('answers nothing for a URL with no labels', () => {
		expect(labelsFromUrl(new URLSearchParams('q=fractions'))).toEqual([]);
	});

	it('writes a search and its labels back', () => {
		expect(filterSearch('fractions', ['year 5', 'maths'])).toBe(
			'q=fractions&label=year+5&label=maths'
		);
	});

	it('writes nothing at all when nothing is set', () => {
		expect(filterSearch('   ', [])).toBe('');
	});

	it('round-trips the labels it writes', () => {
		const labels = ['year 5', 'maths, applied', 'reading & writing'];
		expect(labelsFromUrl(new URLSearchParams(filterSearch('', labels)))).toEqual(labels);
	});
});

describe('closing a phrase', () => {
	it('adds nothing to a phrase that closed itself', () => {
		expect(fullStop('checking what this platform requires…')).toBe('');
		expect(fullStop('It is ready.')).toBe('');
		expect(fullStop('Ready?  ')).toBe('');
	});

	it('closes a phrase that did not', () => {
		expect(fullStop('a send is under way')).toBe('.');
		expect(fullStop('')).toBe('.');
	});
});

describe('the status tone', () => {
	it('spells the board\u2019s quiet tone the badge\u2019s way', () => {
		expect(PILL_TONE.mut).toBe('soon');
		expect(PILL_TONE.ok).toBe('ok');
		expect(PILL_TONE.run).toBe('run');
		expect(PILL_TONE.bad).toBe('bad');
	});
});

describe('the bulk wording', () => {
	it('reads as a verb inside the select-all sentence', () => {
		expect(`Select the resources to ${VERB_PHRASE.cross_list}`).toBe(
			'Select the resources to cross-list'
		);
		expect(VERB_PHRASE.labels).toBe('label');
	});
});
