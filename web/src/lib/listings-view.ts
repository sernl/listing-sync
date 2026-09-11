// One listing row as the console renders it: which marketplaces carry it,
// where it stands, what it costs, and whether a search names it. Shared by
// the listings table and the dashboard's recently-updated strip, so the two
// cannot describe the same product differently. Pure, so it tests without a
// component.

import type { ListingMetricsView, MappingHead } from '$lib/api';
import { standingOf, type ListingStanding } from '$lib/tes-portfolio';
import type { InventoryId, Marketplace } from '$lib/generated/vocab';

/** Which marketplace each inventory belongs to.
 *
 * A total map rather than a lookup with a fallback: the union is generated
 * from the Rust enum, so an inventory added there stops this file
 * type-checking instead of silently rendering as an unnamed platform. */
export const MARKETPLACE_OF: Record<InventoryId, Marketplace> = {
	Tes: 'Tes',
	Etsy: 'Etsy',
	Tpt: 'Tpt'
};

/** The inventories in the order a row's badges appear, so two rows never
 *  order the same pair of platforms differently. */
export const INVENTORY_ORDER: readonly InventoryId[] = ['Tpt', 'Tes', 'Etsy'];

export interface PlatformBadge {
	inventory: InventoryId;
	/** The inventory's own name, which is what the seller chose it by. */
	label: InventoryId;
	/** Whether the marketplace was last recorded as showing this listing. */
	live: boolean;
	/** The mapping's two stored states, verbatim, so the exact vocabulary the
	 *  server keeps stays reachable behind the summarised badge. */
	state: string;
}

export function badgesFor(mappings: readonly MappingHead[]): PlatformBadge[] {
	const byInventory = new Map(mappings.map((mapping) => [mapping.inventory, mapping]));
	return INVENTORY_ORDER.filter((inventory) => byInventory.has(inventory)).map((inventory) => {
		const mapping = byInventory.get(inventory);
		return {
			inventory,
			label: inventory,
			live: mapping !== undefined && standingOf(mapping) === 'live',
			state: mapping === undefined ? '' : `${mapping.binding_state} · ${mapping.lifecycle_state}`
		};
	});
}

export type RowTone = 'ok' | 'run' | 'bad' | 'mut';

export interface RowStatus {
	label: string;
	tone: RowTone;
}

/** Where a product stands, read from its mappings' two stored states and
 *  nothing else.
 *
 * A product live on some of its marketplaces and not others says so, because
 * calling that simply live would overstate what any one marketplace shows. */
export function rowStatus(mappings: readonly MappingHead[]): RowStatus {
	if (mappings.length === 0) {
		return { label: 'No mapping', tone: 'mut' };
	}
	const standings = mappings.map(standingOf);
	const count = (standing: ListingStanding) =>
		standings.filter((entry) => entry === standing).length;
	const live = count('live');
	if (live === mappings.length) {
		return { label: 'Live', tone: 'ok' };
	}
	if (live > 0) {
		return { label: `Live on ${live} of ${mappings.length}`, tone: 'ok' };
	}
	if (count('unsent') === mappings.length) {
		return { label: 'Not sent', tone: 'mut' };
	}
	if (count('draft') > 0) {
		return { label: 'Draft', tone: 'mut' };
	}
	return { label: 'In progress', tone: 'run' };
}

/** The two currencies `tam-types` denominates a price in, with the exponent
 *  its minor units are counted in. A denomination absent from here is one
 *  this client cannot write out, and reads as unreadable rather than as a
 *  number under the wrong symbol. */
export const CURRENCIES: Record<string, { code: string; exponent: number }> = {
	Gbp: { code: 'GBP', exponent: 2 },
	Usd: { code: 'USD', exponent: 2 }
};

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null;
}

/**
 * A catalogue price as the table writes it.
 *
 * The products endpoint serves `PriceIntent`: the string `Free`, or an object
 * carrying `Paid` with the minor units and the denomination. Anything else is
 * an em dash rather than a guess — a price is the one figure on this page a
 * seller would act on.
 */
export function formatPrice(price: unknown): string {
	if (price === 'Free') {
		return 'Free';
	}
	if (!isRecord(price) || !isRecord(price.Paid)) {
		return '—';
	}
	const { minor_units: minorUnits, currency } = price.Paid;
	if (typeof minorUnits !== 'number' || !Number.isFinite(minorUnits)) {
		return '—';
	}
	const denomination = typeof currency === 'string' ? CURRENCIES[currency] : undefined;
	if (denomination === undefined) {
		return '—';
	}
	const major = minorUnits / 10 ** denomination.exponent;
	return new Intl.NumberFormat('en-GB', {
		style: 'currency',
		currency: denomination.code
	}).format(major);
}

/** The search query a URL carries, or the empty string for no filter. */
export function normaliseQuery(raw: string | null | undefined): string {
	return typeof raw === 'string' ? raw.trim() : '';
}

/** Whether a search names this listing. Case-insensitive substring over the
 *  title, which is the only text of a product this client holds; a blank
 *  query names everything rather than nothing. */
export function matchesQuery(title: string, query: string): boolean {
	const needle = query.trim().toLowerCase();
	return needle.length === 0 || title.toLowerCase().includes(needle);
}

export interface RowMetrics {
	views: number | null;
	sales: number | null;
	/** The oldest instant among the figures summed into this row. */
	observedAt: number | null;
}

/**
 * The captured figures for each product, joined through its mappings.
 *
 * The analytics summary names a mapping and nothing else, so a product's
 * figures are the sum over the mappings that carry one; a product whose
 * mappings carry none is absent from the map rather than present with a zero,
 * because nothing captured is not the same fact as nothing sold. The age is
 * the oldest of the instants summed, so a row never reads fresher than its
 * stalest part.
 */
export function metricsByProduct(
	listings: readonly ListingMetricsView[],
	mappings: readonly MappingHead[]
): Map<string, RowMetrics> {
	const productOf = new Map(mappings.map((mapping) => [mapping.id, mapping.product]));
	const rows = new Map<string, RowMetrics>();
	for (const listing of listings) {
		const product = productOf.get(listing.mapping);
		if (product === undefined) {
			continue;
		}
		const row = rows.get(product) ?? { views: null, sales: null, observedAt: null };
		const views = listing.metrics.resource_views;
		if (typeof views === 'number' && Number.isFinite(views)) {
			row.views = (row.views ?? 0) + views;
		}
		const sales = listing.metrics.sales_count;
		if (typeof sales === 'number' && Number.isFinite(sales)) {
			row.sales = (row.sales ?? 0) + sales;
		}
		row.observedAt =
			row.observedAt === null ? listing.observed_at : Math.min(row.observedAt, listing.observed_at);
		rows.set(product, row);
	}
	return rows;
}
