// The Tes portfolio panel's figures. Tes publishes no statistics of its own,
// so a listing on Tes is counted from what this org's own catalogue records
// about it rather than captured from the marketplace. Pure, so it tests
// without a component.

import type { MappingHead, ProductHead } from '$lib/api';
import type { InventoryId } from '$lib/generated/vocab';

/** Which of the closed inventories is Tes.
 *
 * A total map rather than a membership list: the union is generated from the
 * Rust enum, so a marketplace added there stops this file type-checking
 * instead of silently going uncounted. */
export const IS_TES: Record<InventoryId, boolean> = {
	Tes: true,
	Etsy: false,
	Tpt: false
};

/** Where one mapping stands, as a partition: every Tes mapping falls in
 * exactly one of these, so the four counts sum to the listings total. */
export type ListingStanding = 'live' | 'draft' | 'unsent' | 'other';

/** The standing a mapping's two stored states describe.
 *
 * A listing exists only while the mapping is bound to one, so the lifecycle
 * is read only there; on any other binding it describes a listing that is not
 * there. The two states arrive as the spellings the server stores rather than
 * as a generated union, so an unrecognised one lands in `other` and is still
 * counted rather than dropped. */
export function standingOf(mapping: MappingHead): ListingStanding {
	if (mapping.binding_state === 'unbound') {
		return 'unsent';
	}
	if (mapping.binding_state !== 'bound') {
		return 'other';
	}
	switch (mapping.lifecycle_state) {
		case 'live':
			return 'live';
		case 'draft':
			return 'draft';
		default:
			return 'other';
	}
}

/** What a catalogue price says.
 *
 * The products endpoint serves the price intent untyped, as the string
 * `Free` or as an object carrying `Paid`. `unreadable` stays its own answer
 * so a shape this client does not recognise is counted as neither. */
export type PriceReading = 'paid' | 'free' | 'unreadable';

export function readPrice(price: unknown): PriceReading {
	if (price === 'Free') {
		return 'free';
	}
	if (typeof price === 'object' && price !== null && 'Paid' in price) {
		return 'paid';
	}
	return 'unreadable';
}

export interface TesPortfolio {
	/** Every mapping onto Tes, whatever standing it is in. */
	listings: number;
	live: number;
	/** Of the live listings, those whose catalogue product carries a price,
	 *  and those it records as free. A product missing from the catalogue
	 *  read, or priced in a shape this client cannot read, is in neither. */
	livePriced: number;
	liveFree: number;
	drafts: number;
	unsent: number;
	other: number;
	/** Catalogue products carrying no Tes mapping at all. */
	productsWithoutListing: number;
}

export function tesPortfolio(
	products: readonly ProductHead[],
	mappings: readonly MappingHead[]
): TesPortfolio {
	const priceOf = new Map(products.map((product) => [product.id, readPrice(product.price)]));
	const listed = new Set<string>();
	const portfolio: TesPortfolio = {
		listings: 0,
		live: 0,
		livePriced: 0,
		liveFree: 0,
		drafts: 0,
		unsent: 0,
		other: 0,
		productsWithoutListing: 0
	};
	for (const mapping of mappings) {
		if (!IS_TES[mapping.inventory]) {
			continue;
		}
		portfolio.listings += 1;
		listed.add(mapping.product);
		switch (standingOf(mapping)) {
			case 'live': {
				portfolio.live += 1;
				const price = priceOf.get(mapping.product);
				if (price === 'paid') {
					portfolio.livePriced += 1;
				} else if (price === 'free') {
					portfolio.liveFree += 1;
				}
				break;
			}
			case 'draft':
				portfolio.drafts += 1;
				break;
			case 'unsent':
				portfolio.unsent += 1;
				break;
			case 'other':
				portfolio.other += 1;
				break;
		}
	}
	portfolio.productsWithoutListing = products.filter(
		(product) => !listed.has(product.id)
	).length;
	return portfolio;
}

export interface PortfolioRow {
	key: keyof TesPortfolio;
	label: string;
	/** What the figure counts, in the words the panel shows on hover. */
	explanation: string;
	/** A breakdown of the row above it rather than a figure of its own. */
	breakdown?: true;
}

/** The rows the panel shows, in order. `listings` is deliberately absent: it
 *  is the total, and the panel states it beside the heading. */
export const PORTFOLIO_ROWS: readonly PortfolioRow[] = [
	{
		key: 'live',
		label: 'Live on TES',
		explanation: 'Live on TES when we last checked.'
	},
	{
		key: 'livePriced',
		label: 'with a price in your Resources',
		explanation: 'Live listings whose resource has a price here.',
		breakdown: true
	},
	{
		key: 'liveFree',
		label: 'free in your Resources',
		explanation: 'Live listings whose resource is free here.',
		breakdown: true
	},
	{
		key: 'drafts',
		label: 'Drafts waiting to go live',
		explanation: 'Created on TES and not published yet.'
	},
	{
		key: 'unsent',
		label: 'Not sent to TES yet',
		explanation: 'Set up for TES, with nothing created there yet.'
	},
	{
		key: 'other',
		label: 'In another state',
		explanation:
			'Sent for review, in review, rejected, withdrawn, being created, or removed from TES.'
	},
	{
		key: 'productsWithoutListing',
		label: 'Resources with no TES listing',
		explanation: 'In your Resources, and not set up for TES.'
	}
];
