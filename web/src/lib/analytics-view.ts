// The analytics table's rendering: which captured metrics it shows, how a
// figure is written out, how old that figure is said to be, and which listing
// a row names. Pure, so it tests without a component.

import type { MappingHead, ProductHead } from '$lib/api';

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

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** How old a row's figures are, in the words the table shows.
 *
 * The instant is the oldest of the figures in the row, which is what the
 * server sends, so this wording understates freshness and never overstates
 * it. A reading ahead of `now` is clock skew rather than a figure from the
 * future, and reads as just captured rather than as a negative age. */
export function capturedAgo(observedAt: number, now: number): string {
	const elapsed = Math.max(0, now - observedAt);
	if (elapsed < MINUTE) {
		return 'captured just now';
	}
	if (elapsed < HOUR) {
		return `captured ${Math.floor(elapsed / MINUTE)} min ago`;
	}
	if (elapsed < DAY) {
		return `captured ${Math.floor(elapsed / HOUR)} h ago`;
	}
	const days = Math.floor(elapsed / DAY);
	return `captured ${days} ${days === 1 ? 'day' : 'days'} ago`;
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
