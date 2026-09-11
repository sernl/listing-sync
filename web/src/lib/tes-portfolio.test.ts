import { describe, expect, it } from 'vitest';
import {
	PORTFOLIO_ROWS,
	readPrice,
	standingOf,
	tesPortfolio,
	type TesPortfolio
} from './tes-portfolio';
import type { MappingHead, ProductHead } from '$lib/api';
import type { InventoryId } from '$lib/generated/vocab';

const PAID = { Paid: { minor_units: 450, currency: 'Gbp' } };

function product(id: string, price: unknown = PAID): ProductHead {
	return { id, title: `Product ${id}`, price, created_at: 0, updated_at: 0 };
}

function mapping(
	id: string,
	productId: string,
	inventory: InventoryId,
	binding: string,
	lifecycle: string
): MappingHead {
	return {
		id,
		product: productId,
		inventory,
		binding_state: binding,
		lifecycle_state: lifecycle,
		updated_at: 0,
		listing_url: null
	};
}

function live(id: string, productId: string, inventory: InventoryId = 'Tes'): MappingHead {
	return mapping(id, productId, inventory, 'bound', 'live');
}

const EMPTY: TesPortfolio = {
	listings: 0,
	live: 0,
	livePriced: 0,
	liveFree: 0,
	drafts: 0,
	unsent: 0,
	other: 0,
	productsWithoutListing: 0
};

describe('a catalogue price', () => {
	it('reads the unit variant the server spells as a bare string', () => {
		expect(readPrice('Free')).toBe('free');
	});

	it('reads the paid variant by the key it is tagged with', () => {
		expect(readPrice(PAID)).toBe('paid');
	});

	it('calls a shape it does not recognise unreadable rather than free', () => {
		expect(readPrice(null)).toBe('unreadable');
		expect(readPrice(undefined)).toBe('unreadable');
		expect(readPrice(450)).toBe('unreadable');
		expect(readPrice('free')).toBe('unreadable');
		expect(readPrice({})).toBe('unreadable');
	});
});

describe('where a mapping stands', () => {
	it('reads a bound mapping by its lifecycle', () => {
		expect(standingOf(mapping('m', 'p', 'Tes', 'bound', 'live'))).toBe('live');
		expect(standingOf(mapping('m', 'p', 'Tes', 'bound', 'draft'))).toBe('draft');
	});

	it('calls a mapping that was never bound unsent', () => {
		expect(standingOf(mapping('m', 'p', 'Tes', 'unbound', 'absent'))).toBe('unsent');
	});

	it('does not read a severed mapping as live, whatever its lifecycle says', () => {
		expect(standingOf(mapping('m', 'p', 'Tes', 'severed', 'live'))).toBe('other');
	});

	it('holds a create in flight apart from a listing that exists', () => {
		expect(standingOf(mapping('m', 'p', 'Tes', 'creating', 'absent'))).toBe('other');
		expect(standingOf(mapping('m', 'p', 'Tes', 'ambiguous_create', 'absent'))).toBe('other');
	});

	it('counts a lifecycle it has no row for rather than dropping it', () => {
		expect(standingOf(mapping('m', 'p', 'Tes', 'bound', 'in_review'))).toBe('other');
		expect(standingOf(mapping('m', 'p', 'Tes', 'bound', 'withdrawn'))).toBe('other');
		expect(standingOf(mapping('m', 'p', 'Tes', 'bound', 'invented'))).toBe('other');
	});
});

describe('the portfolio of an empty catalogue', () => {
	it('is every figure at zero rather than nothing at all', () => {
		expect(tesPortfolio([], [])).toEqual(EMPTY);
	});

	it('stays at zero when products exist but nothing is mapped anywhere', () => {
		expect(tesPortfolio([product('p1'), product('p2')], [])).toEqual({
			...EMPTY,
			productsWithoutListing: 2
		});
	});
});

describe('a catalogue with no Tes mappings', () => {
	it('counts no listings, and says both products are not on Tes', () => {
		const portfolio = tesPortfolio(
			[product('p1'), product('p2')],
			[live('m1', 'p1', 'Tpt'), live('m2', 'p2', 'Etsy')]
		);
		expect(portfolio).toEqual({ ...EMPTY, productsWithoutListing: 2 });
	});
});

describe('the listing figures', () => {
	const products = [product('p1'), product('p2', 'Free'), product('p3'), product('p4')];
	const mappings = [
		live('m1', 'p1'),
		live('m2', 'p2', 'Tes'),
		mapping('m3', 'p3', 'Tes', 'bound', 'draft'),
		mapping('m4', 'p4', 'Tes', 'unbound', 'absent'),
		mapping('m5', 'p1', 'Tes', 'bound', 'in_review'),
		live('m6', 'p3', 'Tpt')
	];

	it('counts only the mappings onto Tes', () => {
		expect(tesPortfolio(products, mappings).listings).toBe(5);
	});

	it('partitions those listings, so the four standings sum to the total', () => {
		const portfolio = tesPortfolio(products, mappings);
		expect(portfolio.live + portfolio.drafts + portfolio.unsent + portfolio.other).toBe(
			portfolio.listings
		);
		expect(portfolio.live).toBe(2);
		expect(portfolio.drafts).toBe(1);
		expect(portfolio.unsent).toBe(1);
		expect(portfolio.other).toBe(1);
	});

	it('splits the live listings by what the catalogue prices their product at', () => {
		const portfolio = tesPortfolio(products, mappings);
		expect(portfolio.livePriced).toBe(1);
		expect(portfolio.liveFree).toBe(1);
	});

	it('counts a product listed on two Tes sites once as a listed product', () => {
		const portfolio = tesPortfolio(products, mappings);
		expect(portfolio.productsWithoutListing).toBe(0);
	});
});

describe('a price the panel cannot attribute', () => {
	it('leaves a live listing out of both splits when the price is unreadable', () => {
		const portfolio = tesPortfolio([product('p1', null)], [live('m1', 'p1')]);
		expect(portfolio.live).toBe(1);
		expect([portfolio.livePriced, portfolio.liveFree]).toEqual([0, 0]);
	});

	it('leaves it out when the product is not in the catalogue read at all', () => {
		const portfolio = tesPortfolio([], [live('m1', 'p9')]);
		expect(portfolio.live).toBe(1);
		expect([portfolio.livePriced, portfolio.liveFree]).toEqual([0, 0]);
		expect(portfolio.productsWithoutListing).toBe(0);
	});
});

describe('products with no Tes listing', () => {
	it('counts a product whose only mapping is on another marketplace', () => {
		const portfolio = tesPortfolio(
			[product('p1'), product('p2')],
			[live('m1', 'p1'), live('m2', 'p2', 'Tpt')]
		);
		expect(portfolio.productsWithoutListing).toBe(1);
	});

	it('counts a product mapped onto Tes but never created there as listed', () => {
		const portfolio = tesPortfolio(
			[product('p1')],
			[mapping('m1', 'p1', 'Tes', 'unbound', 'absent')]
		);
		expect(portfolio.productsWithoutListing).toBe(0);
	});
});

describe('the panel rows', () => {
	it('names each figure once, and leaves the total to the heading', () => {
		const keys = PORTFOLIO_ROWS.map((row) => row.key);
		expect(keys).toEqual([
			'live',
			'livePriced',
			'liveFree',
			'drafts',
			'unsent',
			'other',
			'productsWithoutListing'
		]);
		expect(new Set(keys).size).toBe(keys.length);
		expect(keys).not.toContain('listings');
	});

	it('marks only the two price figures as breakdowns of the row above them', () => {
		expect(PORTFOLIO_ROWS.filter((row) => row.breakdown).map((row) => row.key)).toEqual([
			'livePriced',
			'liveFree'
		]);
	});
});
