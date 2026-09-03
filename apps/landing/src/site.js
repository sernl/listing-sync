/**
 * Every value the founder must supply before this site goes live is here, and
 * nowhere else. `placeholder: true` renders the amount as unset rather than as
 * a number, so no draft price can be mistaken for an offer.
 *
 * The list of what still needs replacing is in
 * `docs/notes/design/landing-page.md` under "Placeholders".
 */

/** Where the console is served. Replace when the console's host is settled. */
export const consoleOrigin = 'https://app.teachouse.io';

export const signUpUrl = `${consoleOrigin}/signup`;
export const signInUrl = `${consoleOrigin}/login`;

/** Replace with the address the founder actually monitors. */
export const supportEmail = 'hello@teachouse.io';

/**
 * What the founder is willing to say about availability today. This sentence
 * is the only claim on the site about whether a seller can use it right now.
 */
export const availability =
	'Teachouse is in private testing. TeachersPayTeachers and Tes connections work today, and Etsy is next.';

/**
 * D4: the meter is connected marketplaces, with a catalogue cap on the entry
 * tier. Amounts and caps are unset; the memo names none.
 */
export const tiers = [
	{
		name: 'One marketplace',
		who: 'For a seller who wants a second storefront without a second evening of typing.',
		amount: null,
		period: 'per month',
		features: [
			'One connected marketplace',
			'Catalogue cap: to be set',
			'Mapping, publishing and revision from your own computer',
			'Sales and view figures pulled back into one place'
		]
	},
	{
		name: 'A few marketplaces',
		who: 'For a seller who already lists in more than one place and is tired of the drift.',
		amount: null,
		period: 'per month',
		features: [
			'Up to a set number of connected marketplaces',
			'Catalogue cap: to be set',
			'One update propagated to every marketplace it applies to',
			'Per-marketplace results, reported one by one'
		]
	},
	{
		name: 'Every marketplace',
		who: 'For a seller whose catalogue is the business.',
		amount: null,
		period: 'per month',
		features: [
			'Every marketplace Teachouse supports',
			'No catalogue cap',
			'Multiple accounts on the same marketplace',
			'Every marketplace added later, as it is added'
		]
	}
];

/**
 * Transport class per marketplace, matching `InventoryId::transport_class` in
 * `crates/tam-domain/src/registry`. `device` means every request to that
 * marketplace originates on the seller's own machine under the seller's own
 * login; `api` means the marketplace publishes an official API and issues us a
 * token for the purpose.
 */
export const marketplaces = [
	{
		name: 'TeachersPayTeachers',
		transport: 'device',
		status: 'Working today'
	},
	{
		name: 'Tes',
		transport: 'device',
		status: 'Working today'
	},
	{
		name: 'Etsy',
		transport: 'api',
		status: 'Next'
	},
	{
		name: 'More marketplaces',
		transport: 'either',
		status: 'Planned'
	}
];

export const transportWording = {
	device: 'Runs on your computer, under your own login',
	api: 'Runs on our servers, over the official API',
	either: 'Whichever of the two the marketplace sanctions'
};
