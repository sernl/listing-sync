/**
 * What a sale does to a price on screen.
 *
 * `GET /v1/plans` answers `sale` while one is open — the percentage every
 * plan's checkout takes off by itself, the last day, and the banner — and
 * `null` otherwise. These are the only sums the pages do with it, so the
 * console's Billing page and the landing's pricing section (whose
 * `apps/landing/src/sale.js` is held to the same answers by `sale.test.ts`)
 * strike the same list price and show the same sale price Stripe charges.
 */

import type { SaleView } from '$lib/generated/plans';

/** A list price and the price a sale makes of it. */
export interface SalePrice {
	readonly listCents: number;
	readonly saleCents: number;
}

/**
 * The price after `percent` off, in cents.
 *
 * Stripe computes a percentage coupon's amount off and rounds it to the
 * nearest cent, then subtracts; doing the same here is what keeps the
 * figure on the card and the figure on the receipt the same figure.
 */
export function afterPercentOff(cents: number, percent: number): number {
	const off = Math.round((cents * percent) / 100);
	return Math.max(0, cents - off);
}

/**
 * The struck and sale prices for one plan price, or `null` where nothing
 * is struck: no sale, a free plan, or a price the sale does not reach.
 */
export function salePrice(
	sale: SaleView | null | undefined,
	cents: number | null | undefined
): SalePrice | null {
	if (!sale || cents === null || cents === undefined || cents <= 0) {
		return null;
	}
	const saleCents = afterPercentOff(cents, sale.percent_off);
	return saleCents === cents ? null : { listCents: cents, saleCents };
}

const MONTHS = [
	'January',
	'February',
	'March',
	'April',
	'May',
	'June',
	'July',
	'August',
	'September',
	'October',
	'November',
	'December'
];

/** "31 October", the last day of a sale, spelled out rather than localised
 *  so the console and the landing say the same words. */
export function saleUntilLabel(until: string): string {
	const [, month, day] = until.split('-').map(Number);
	return `${day} ${MONTHS[month - 1]}`;
}

/** The short line under a struck price: "25% off until 31 October". */
export function saleLine(sale: SaleView): string {
	return `${sale.percent_off}% off until ${saleUntilLabel(sale.until)}`;
}
