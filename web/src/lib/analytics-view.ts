// The analytics table's rendering: which captured metrics it shows, how a
// figure is written out, how old that figure is said to be, and which listing
// a row names. Pure, so it tests without a component.

import type { MappingHead, ProductHead } from '$lib/api';
import { agoLabel } from '$lib/elapsed';

export interface MetricColumn {
	/** The name the capture stored the figure under, which is the map key. */
	key: string;
	heading: string;
}

/** The metrics a capture pass stores, in the order the table shows them.
 *
 * The server's metric names are an open set rather than a closed vocabulary,
 * so this list names the ones the table has a column for and a row's other
 * keys are simply not shown. Widening the captured shortlist therefore cannot
 * break a row; it only leaves a figure unrendered until a column is added. */
export const METRIC_COLUMNS: readonly MetricColumn[] = [
	{ key: 'sales_count', heading: 'Sales' },
	{ key: 'earnings', heading: 'Earnings' },
	{ key: 'resource_views', heading: 'Resource views' }
];

// A fixed locale rather than the visitor's: this grouping is asserted in a
// test, and it must stay a plain number. Earnings arrives as the untyped
// number the marketplace served, so rendering it under a currency would state
// a denomination the figure does not carry.
const NUMBER = new Intl.NumberFormat('en-GB', { maximumFractionDigits: 2 });

/** A captured figure, or an em dash where this listing carries none. */
export function formatMetric(value: number | undefined): string {
	return typeof value === 'number' && Number.isFinite(value) ? NUMBER.format(value) : '—';
}

/** How old a row's figures are, in the words the table shows.
 *
 * The instant is the oldest of the figures in the row, which is what the
 * server sends, so this wording understates freshness and never overstates
 * it. */
export function capturedAgo(observedAt: number, now: number): string {
	return `captured ${agoLabel(observedAt, now)}`;
}

/** Mapping id to the title of the product that mapping lists.
 *
 * The analytics summary names a mapping and nothing else, so the human title
 * is joined here from the two reads the catalogue page already makes. A
 * mapping whose product is not among them is left out rather than given an
 * invented name; the table renders such a row by its identifier. */
export function titlesByMapping(
	products: readonly ProductHead[],
	mappings: readonly MappingHead[]
): Map<string, string> {
	const byProduct = new Map(products.map((product) => [product.id, product.title]));
	const titles = new Map<string, string>();
	for (const mapping of mappings) {
		const title = byProduct.get(mapping.product);
		if (title !== undefined) {
			titles.set(mapping.id, title);
		}
	}
	return titles;
}
