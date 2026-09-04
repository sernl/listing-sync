/**
 * The published price list, approved by the founder on 2026-09-05, in USD.
 *
 * Prices are quoted in USD only: TPT is a US marketplace and TES is UK-centred,
 * so NZD is neither buyer's currency and quoting it puts an FX conversion in
 * front of a small ticket. Changing a number here changes it on both `/` and
 * `/pricing`, which are the only two places it appears.
 */

/** Stated once above the cards rather than repeated inside each one. */
export const trialLine = '14-day Studio trial.';

export const tiers = [
	{
		name: 'Free',
		persona: 'For trying it on one marketplace before you commit.',
		monthly: '$0',
		cadence: 'Free, with no card',
		bridge: null,
		features: ['One marketplace', '20 resources kept in sync', 'Manual sync'],
		cta: 'Start free',
		featured: false
	},
	{
		name: 'Solo',
		persona: 'For a seller with one shop and a second one to fill.',
		monthly: '$12',
		cadence: 'or $120 a year — two months free',
		bridge: 'Everything in Free, plus:',
		features: [
			'100 resources kept in sync',
			'2 marketplaces',
			'Daily sync',
			'1 device',
			'50 resources migrated a year'
		],
		cta: 'Start free',
		featured: false
	},
	{
		name: 'Studio',
		persona: 'For a seller keeping a few hundred resources current on both marketplaces.',
		monthly: '$24',
		cadence: 'or $240 a year — two months free',
		bridge: 'Everything in Solo, plus:',
		features: [
			'400 resources kept in sync',
			'All marketplaces',
			'Sync every 6 hours',
			'2 devices',
			'200 resources migrated a year'
		],
		cta: 'Start free',
		featured: true
	},
	{
		name: 'Publisher',
		persona: 'For a full catalogue that has to stay current everywhere.',
		monthly: '$48',
		cadence: 'or $480 a year — two months free',
		bridge: 'Everything in Studio, plus:',
		features: [
			'Unlimited resources kept in sync',
			'All marketplaces',
			'Hourly sync',
			'3 devices',
			'500 resources migrated a year'
		],
		cta: 'Start free',
		featured: false
	}
];

/** The tier that also gets the full-width panel under the cards. */
export const topTier = {
	name: 'Publisher, in full',
	lead: 'Unlimited resources, hourly, on three devices.',
	body: 'A catalogue with no cap on it, reconciled every hour, on up to three computers, with 500 resources migrated a year included. The sync timer for TPT and TES runs on your own machine rather than on ours, which is why an hourly schedule is priced at all rather than rationed.',
	price: '$48 a month, or $480 a year'
};

export const migrationBands = [
	{ size: '50 resources or fewer', price: '$49' },
	{ size: '51 to 100', price: '$79' },
	{ size: '101 to 200', price: '$129' },
	{ size: '201 to 500', price: '$199' },
	{
		size: '501 and up',
		price: '$299 for the first 500 resources, then $0.25 for each resource beyond 500'
	}
];

export const migrationRules = [
	'Every migration includes 30 days of Studio, so you can look over the mappings on the listings we just created.',
	'If you already subscribe, your plan’s yearly allowance is used first, and anything past it is half the band price.',
	'Buy an annual plan within 30 days of a migration and the migration price comes off it in full.'
];
