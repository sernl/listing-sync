import { describe, expect, it } from 'vitest';
import {
	INVENTORY_ORDER,
	MARKETPLACE_OF,
	badgesFor,
	formatPrice,
	matchesQuery,
	normaliseQuery,
	metricsByProduct,
	rowStatus
} from './listings-view';
import type { ListingMetricsView, MappingHead } from './api';
import type { InventoryId } from './generated/vocab';

function mapping(
	inventory: InventoryId,
	binding_state: string,
	lifecycle_state: string
): MappingHead {
	return {
		id: `${inventory}-${binding_state}-${lifecycle_state}`,
		product: 'p1',
		inventory,
		binding_state,
		lifecycle_state,
		updated_at: 0,
		listing_url: null
	};
}

describe('the marketplace of an inventory', () => {
	it('names one for every inventory the vocabulary carries', () => {
		expect(Object.keys(MARKETPLACE_OF).sort()).toEqual([...INVENTORY_ORDER].sort());
	});
});

describe("a row's platform badges", () => {
	it('are the inventories the product is mapped onto, in a fixed order', () => {
		const badges = badgesFor([
			mapping('Tes', 'bound', 'live'),
			mapping('Tpt', 'unbound', 'absent')
		]);
		expect(badges.map((badge) => badge.inventory)).toEqual(['Tpt', 'Tes']);
	});

	it('mark live only what the marketplace was last recorded as showing', () => {
		const badges = badgesFor([
			mapping('Tpt', 'bound', 'live'),
			mapping('Tes', 'bound', 'draft')
		]);
		expect(badges.map((badge) => badge.live)).toEqual([true, false]);
	});

	it('carry the two stored states verbatim, so the summary hides nothing', () => {
		const [badge] = badgesFor([mapping('Tpt', 'bound', 'in_review')]);
		expect(badge.state).toBe('bound · in_review');
	});

	it('are none for a product with no mapping', () => {
		expect(badgesFor([])).toEqual([]);
	});
});

describe("a row's status", () => {
	it('says nothing is mapped rather than inventing a state', () => {
		expect(rowStatus([])).toEqual({ label: 'No mapping', tone: 'mut' });
	});

	it('is live only when every mapping is', () => {
		expect(rowStatus([mapping('Tpt', 'bound', 'live'), mapping('Tes', 'bound', 'live')])).toEqual(
			{ label: 'Live', tone: 'ok' }
		);
	});

	it('says how many marketplaces carry it when not all do', () => {
		expect(
			rowStatus([mapping('Tpt', 'bound', 'live'), mapping('Tes', 'unbound', 'absent')])
		).toEqual({ label: 'Live on 1 of 2', tone: 'ok' });
	});

	it('separates never sent from drafted and from anything else', () => {
		expect(rowStatus([mapping('Tpt', 'unbound', 'absent')]).label).toBe('Not sent');
		expect(rowStatus([mapping('Tpt', 'bound', 'draft')]).label).toBe('Draft');
		expect(rowStatus([mapping('Tpt', 'bound', 'in_review')])).toEqual({
			label: 'In progress',
			tone: 'run'
		});
	});
});

describe('a catalogue price', () => {
	it('writes free as free', () => {
		expect(formatPrice('Free')).toBe('Free');
	});

	it('writes a paid price under the denomination it carries', () => {
		expect(formatPrice({ Paid: { minor_units: 825, currency: 'Usd' } })).toBe('US$8.25');
		expect(formatPrice({ Paid: { minor_units: 400, currency: 'Gbp' } })).toBe('£4.00');
	});

	it('is an em dash for a shape or a denomination this client cannot read', () => {
		expect(formatPrice(undefined)).toBe('—');
		expect(formatPrice('Cheap')).toBe('—');
		expect(formatPrice({ Paid: 825 })).toBe('—');
		expect(formatPrice({ Paid: { minor_units: '825', currency: 'Usd' } })).toBe('—');
		expect(formatPrice({ Paid: { minor_units: 825, currency: 'Eur' } })).toBe('—');
	});
});

describe('the search filter', () => {
	it('reads a missing or blank query as no filter at all', () => {
		expect(normaliseQuery(null)).toBe('');
		expect(normaliseQuery('  ')).toBe('');
		expect(normaliseQuery(' poetry ')).toBe('poetry');
	});

	it('names every listing when nothing was typed', () => {
		expect(matchesQuery('Fractions Escape Room', '')).toBe(true);
		expect(matchesQuery('Fractions Escape Room', '   ')).toBe(true);
	});

	it('matches part of a title, whatever the casing', () => {
		expect(matchesQuery('Fractions Escape Room', 'escape')).toBe(true);
		expect(matchesQuery('Fractions Escape Room', 'ESCAPE ROOM')).toBe(true);
		expect(matchesQuery('Fractions Escape Room', 'phonics')).toBe(false);
	});
});

describe("a product's captured figures", () => {
	function captured(mapping: string, observed_at: number, metrics: Record<string, number>) {
		return { mapping, inventory: 'Tpt', observed_at, metrics } as ListingMetricsView;
	}

	const mappings: MappingHead[] = [
		{ ...mapping('Tpt', 'bound', 'live'), id: 'm1', product: 'p1' },
		{ ...mapping('Tes', 'bound', 'live'), id: 'm2', product: 'p1' },
		{ ...mapping('Etsy', 'bound', 'live'), id: 'm3', product: 'p2' }
	];

	it('sum across the mappings that carry one, aged by the oldest', () => {
		const rows = metricsByProduct(
			[
				captured('m1', 500, { resource_views: 100, sales_count: 4 }),
				captured('m2', 200, { resource_views: 30, sales_count: 1 })
			],
			mappings
		);
		expect(rows.get('p1')).toEqual({ views: 130, sales: 5, observedAt: 200 });
	});

	it('leave a figure absent rather than reading nothing captured as a zero', () => {
		const rows = metricsByProduct([captured('m3', 700, { resource_views: 9 })], mappings);
		expect(rows.get('p3')).toBeUndefined();
		expect(rows.get('p2')).toEqual({ views: 9, sales: null, observedAt: 700 });
	});

	it('ignore a capture whose mapping is not in the catalogue read', () => {
		expect(metricsByProduct([captured('unknown', 1, { sales_count: 3 })], mappings).size).toBe(0);
	});
});
