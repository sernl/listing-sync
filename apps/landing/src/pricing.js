/**
 * The published price list, from the founder's landing mockup of 2026-09-11, in
 * USD. It replaces the four-tier ladder approved on 2026-09-05.
 *
 * Prices are quoted in USD only: TPT is a US marketplace and Tes is UK-centred,
 * so NZD is neither buyer's currency and quoting it puts an FX conversion in
 * front of a small ticket. Changing a number here changes it on both `/` and
 * `/pricing`, which are the only two places it appears.
 */

/** The one-off import, priced by how much of a catalogue comes across. */
export const importLadder = [
	{ cap: 'Up to 20', price: '$47' },
	{ cap: 'Up to 50', price: '$77' },
	{ cap: 'Up to 100', price: '$127' },
	{ cap: 'Up to 250', price: '$247' }
];

/**
 * The one subscription. `monthly` is the old Studio price, unchanged: the
 * mockup states no figure for the subscription, and the founder's four tiers
 * collapsed into this single plan, so the middle tier's price carries over
 * rather than a new number being invented.
 */
export const subscription = {
	name: 'Teachouse Subscription',
	cadence: 'monthly or annual',
	monthly: 24,
	yearly: 240,
	features: [
		'Central catalogue',
		'Bulk tools',
		'AI-powered listings',
		'Marketplace management'
	]
};

/** The founding offer, and the only scarcity claim on the site. */
export const founding = {
	discountYearOne: 25,
	discountOngoing: 20,
	freeImports: 20,
	places: 100
};
