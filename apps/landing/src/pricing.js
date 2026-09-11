/**
 * The published price list, from the founder's landing mockup of 2026-09-11 and
 * the pricing decision of 2026-09-12, in USD. It replaces the four-tier ladder
 * approved on 2026-09-05.
 *
 * Prices are quoted in USD only: TPT is a US marketplace and Tes is UK-centred,
 * so NZD is neither buyer's currency and quoting it puts an FX conversion in
 * front of a small ticket. Changing a number here changes it on both `/` and
 * `/pricing`, which are the only two places it appears.
 */

/**
 * The one-off import, priced by how much of a catalogue comes across.
 *
 * A rung is a cap and a price rather than two display strings, so the FAQ and
 * the cards phrase the same rung their own way and no page can quote a band
 * this file does not hold. The last rung has no cap and no price: the measured
 * dual-lister holds about 764 listings, which is a conversation rather than a
 * band (decision of 2026-09-12).
 */
export const importLadder = [
	{ upTo: 20, price: 47 },
	{ upTo: 50, price: 77 },
	{ upTo: 100, price: 127 },
	{ upTo: 250, price: 247 },
	{ upTo: 500, price: 397 },
	{ upTo: null, label: 'Talk to us' }
];

/** "Up to 250", or the open rung's own label. */
export const rungCap = (rung) => rung.label ?? `Up to ${rung.upTo}`;

/** "$247", or the invitation the open rung carries in place of a figure. */
export const rungPrice = (rung) => (rung.price === undefined ? 'Ask us' : `$${rung.price}`);

/** Every rung that names a figure, which is the ladder minus the open one. */
export const pricedRungs = importLadder.filter((rung) => rung.price !== undefined);

/**
 * What the cap counts. The ladder is priced by resources, and an import of two
 * marketplaces holding the same 300 resources commits 300 of them, not 600, so
 * the sentence is on the card rather than left to be discovered at the invoice.
 */
export const importLadderNote =
	'Counted as resources added to your catalogue after duplicates are merged.';

/**
 * AI auto-fill, which is sold today and built later. `status` is what the
 * cards read to decide whether they may promise it; "coming soon" promises no
 * accuracy figure, no marketplace write and no date, per the decision of
 * 2026-09-12.
 */
const ai = {
	status: 'coming-soon',
	includedFills: 200,
	addOn: { fills: 100, price: 5 }
};

/**
 * The one subscription. `monthly` is the old Studio price, unchanged: the
 * mockup states no figure for the subscription, and the founder's four tiers
 * collapsed into this single plan, so the middle tier's price carries over
 * rather than a new number being invented.
 *
 * `includesImport` is the 2026-09-12 decision: every comparable crosslister
 * bundles import, and charging at the moment of activation taxes the one step
 * that makes the product useful.
 */
export const subscription = {
	name: 'Teachouse Subscription',
	cadence: 'monthly or annual',
	monthly: 24,
	yearly: 240,
	includesImport: true,
	ai,
	/* A feature is a line and, where it is sold before it is built, the state
	   that stops a tick claiming otherwise. */
	features: [
		{ text: 'Central catalogue' },
		{ text: 'Bulk tools' },
		{ text: 'Import included' },
		{ text: `AI fill \u2014 coming soon (${ai.includedFills} a month)`, soon: true },
		{ text: 'Marketplace management' }
	]
};

/** The founding offer, and the only scarcity claim on the site. */
export const founding = {
	discountYearOne: 25,
	discountOngoing: 20,
	ongoingYears: 3,
	freeImports: 20,
	places: 100
};
