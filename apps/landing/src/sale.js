/**
 * The landing's half of a sale: the same sums the console's
 * `web/src/lib/sale.ts` does, which `web/src/lib/sale.test.ts` holds this
 * file to.
 *
 * The page is static and a sale is data an operator saves, so the prices
 * are drawn at list and this is applied in the browser from `GET /v1/plans`
 * (same origin, so the landing's `connect-src 'self'` admits it). Without
 * a sale, or without the request, the page stays exactly as built.
 */

/** Stripe's rounding: the amount off is rounded to the cent, then taken off. */
export const afterPercentOff = (cents, percent) =>
	Math.max(0, cents - Math.round((cents * percent) / 100));

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

/** "25% off until 31 October". */
export const saleLine = (sale) => {
	const [, month, day] = sale.until.split('-').map(Number);
	return `${sale.percent_off}% off until ${day} ${MONTHS[month - 1]}`;
};
