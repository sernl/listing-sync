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

/** A price in dollars, showing cents only where a price is not whole. Here
 *  rather than in `pricing.js` because the browser imports this file, and
 *  `pricing.js` would bring the generated plan table with it. */
export const dollars = (cents) =>
	cents % 100 === 0 ? `$${cents / 100}` : `$${(cents / 100).toFixed(2)}`;

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

/**
 * Draws an open sale into a built page.
 *
 * A price that a sale reaches carries `data-list-cents` (the price as
 * charged) and optionally `data-per` (what it is shown divided by: 12 for a
 * yearly price quoted per month). Its text becomes the struck list figure
 * and the sale figure; `[data-sale-banner]` gets the banner and is shown,
 * and `[data-sale-hide]` (a figure worked out at list price, such as the
 * yearly saving) is hidden rather than left quoting a sum the sale changed.
 * Only text and the `hidden` attribute change: the landing's policy admits
 * no inline style.
 */
export const applySale = (root, sale, dollars) => {
	for (const price of root.querySelectorAll('[data-list-cents]')) {
		const cents = Number(price.dataset.listCents);
		const per = Number(price.dataset.per ?? '1');
		if (!(cents > 0) || !(per > 0)) continue;
		const was = document.createElement('s');
		was.className = 'was';
		was.textContent = dollars(Math.round(cents / per));
		const now = document.createElement('span');
		now.className = 'now';
		now.textContent = dollars(Math.round(afterPercentOff(cents, sale.percent_off) / per));
		price.replaceChildren(was, ' ', now);
	}
	for (const stale of root.querySelectorAll('[data-sale-hide]')) stale.hidden = true;
	for (const banner of root.querySelectorAll('[data-sale-banner]')) {
		const text = banner.querySelector('[data-sale-text]') ?? banner;
		text.textContent = sale.banner;
		const line = banner.querySelector('[data-sale-line]');
		if (line) line.textContent = saleLine(sale);
		const link = banner.querySelector('a[data-sale-link]');
		if (link) {
			if (sale.banner_href) link.href = sale.banner_href;
			else link.remove();
		}
		banner.hidden = false;
	}
};

/** Asks the server for the price list and draws its sale, if one is open.
 *  Any failure leaves the page at list prices, which is what it was built
 *  with. */
export const loadSale = async (root, dollars) => {
	try {
		const response = await fetch('/v1/plans', { headers: { accept: 'application/json' } });
		if (!response.ok) return;
		const { sale } = await response.json();
		if (sale && sale.percent_off > 0) applySale(root, sale, dollars);
	} catch {
		// List prices stand.
	}
};
