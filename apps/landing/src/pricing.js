/**
 * What the landing page adds to the server's plan table.
 *
 * Every price, cap, tagline and comparison row arrives from
 * `plans.generated.js`, which `cargo run -p tam-typegen` emits from
 * `tam-limits` and `just web-check` diffs, so the pricing page cannot drift
 * from what the server actually enforces. Changing a number, a tagline or
 * which plan is recommended is a change in the Rust table, never here.
 *
 * What stays here is what the landing alone owns: dollar formatting, how a
 * comparison cell reads, which rows a card lifts out as its highlights, the
 * signup links that carry a price key across to the console, and the
 * questions. Prices are quoted in USD only: TPT is a US marketplace and Tes is
 * UK-centred, so NZD is neither buyer's currency and quoting it puts an FX
 * conversion in front of a small ticket.
 */

import { PACKS, PLANS, PLAN_FEATURES, PLAN_FEATURE_GROUPS } from './plans.generated.js';
import { dollars } from './sale.js';
import { consoleUrl } from './site.js';

export { dollars };

/** Every plan, weakest first, in the table's own order. */
export const plans = PLANS;

/** The free plan, which the table names "Look". */
export const free = PLANS.find((plan) => plan.id === 'free');

/**
 * The headline figure in yearly mode is the annual price divided across its
 * months, because that is the number a reader compares against another
 * tool's monthly price. The year's total sits under it rather than being left
 * for the checkout to introduce.
 */
export const perMonthYearly = (plan) => Math.round(plan.yearly_cents / 12);

/**
 * What paying yearly saves, in dollars: a concrete sum reads better than a
 * percentage (2026-09-29 pricing review, section 4).
 */
export const yearlySaving = (plan) => plan.monthly_cents * 12 - plan.yearly_cents;

/** The largest yearly saving on the page, as a whole percentage, for the switch. */
export const bestYearlyPercent = Math.max(
	...PLANS.filter((plan) => plan.yearly_cents !== null).map((plan) =>
		Math.floor((yearlySaving(plan) * 100) / (plan.monthly_cents * 12))
	)
);

/**
 * Where a call to action goes, and what it carries.
 *
 * The console's signup reads `next` and `price` from its query: `next` is
 * where to land after the account exists, and `price` is the checkout to open
 * on arrival. Sending the price key from here means a reader who clicked
 * "Choose Pro" does not have to find Pro again on the other side.
 */
const SIGNUP = `${consoleUrl}/signup`;
export const signupUrl = (priceKey) =>
	priceKey === undefined || priceKey === null
		? `${SIGNUP}?next=/settings/billing`
		: `${SIGNUP}?next=/settings/billing&price=${priceKey}`;

/** How often edits go out, said the way a teacher would. */
const cadence = (hours) =>
	hours === 24 ? 'Daily' : hours === 1 ? 'Hourly' : `Every ${hours} hours`;

/** A size in mebibytes, as MB below a gigabyte and GB above. */
const size = (megabytes) =>
	megabytes >= 1024 ? `${Math.round(megabytes / 1024)} GB` : `${megabytes} MB`;

/**
 * How many of a row's allowance a plan holds for the account's life rather
 * than each month: Look's trial moves and its watermarked previews. Zero on
 * every other row and plan.
 */
const lifetime = (feature, plan) =>
	feature.key === 'moves'
		? plan.capabilities.free_moves_lifetime
		: feature.key === 'watermarked_previews'
			? plan.capabilities.previews_lifetime
			: 0;

/**
 * One comparison cell, as the table draws it: `kind` picks the mark (a tick,
 * a dash, a hollow "soon" mark or a figure), `text` is what a figure says and
 * `label` is what a screen reader hears in place of a mark.
 *
 * The cells the table cannot read off their row are Look's moves and
 * watermarked previews: Look has no monthly allowance of either but does
 * have a few to try for the account's life, which the table says rather than
 * a dash that would read as "none".
 */
export const cell = (feature, plan) => {
	const value = feature.included[plan.id];
	const forLife = lifetime(feature, plan);
	if (value === false && forLife > 0) return { kind: 'text', text: `${forLife} to try` };
	if (value === false) return { kind: 'no', label: 'Not included' };
	if (value === true) {
		if (feature.unit !== null) return { kind: 'text', text: 'Unlimited' };
		return feature.soon
			? { kind: 'soon', label: 'Coming soon' }
			: { kind: 'yes', label: 'Included' };
	}
	switch (feature.unit) {
		case 'per_month':
			return { kind: 'text', text: `${value} a month` };
		case 'megabytes':
			return { kind: 'text', text: size(value) };
		case 'every_hours':
			return { kind: 'text', text: cadence(value) };
		default:
			return { kind: 'text', text: `${value}` };
	}
};

/** The comparison table: its sections in order, each with its rows. */
export const comparison = PLAN_FEATURE_GROUPS.map((group) => ({
	...group,
	rows: PLAN_FEATURES.filter((feature) => feature.group === group.id)
}));

/**
 * The rows a card lifts out, in the order they matter to a teacher choosing.
 * A card shows at most five, so the full list lives in the table under the
 * cards and each card stays a glance.
 */
const HIGHLIGHT_ORDER = [
	'moves',
	'edit_sync',
	'scheduling',
	'auto_publish_rules',
	'analytics',
	'resources',
	'import',
	'watermarked_previews',
	'templates',
	'collections',
	'priority_support',
	'export'
];
const HIGHLIGHTS_MAX = 5;
const byKey = new Map(PLAN_FEATURES.map((feature) => [feature.key, feature]));

/** One highlight line, worded from a row and one plan's cell. */
const highlight = (feature, plan) => {
	const value = feature.included[plan.id];
	const noun = feature.label.toLowerCase();
	if (value === false && lifetime(feature, plan) > 0)
		return `${lifetime(feature, plan)} ${noun} to try`;
	if (feature.key === 'edit_sync') return `Edits synced ${cadence(value).toLowerCase()}`;
	if (feature.unit === null) return feature.label;
	if (value === true) return `Unlimited ${noun}`;
	if (feature.unit === 'per_month') return `${value} ${noun} a month`;
	const counted = value === 1 ? noun.replace(/s$/, '') : noun;
	return feature.key === 'resources' ? `Up to ${value} ${counted}` : `${value} ${counted}`;
};

/** Whether a row's cell on one plan is worth saying on that plan's card. */
const reaches = (feature, plan) =>
	feature.included[plan.id] !== false || lifetime(feature, plan) > 0;

/**
 * A card's highlights. The free card says what the trial gives; each paid
 * card says only what it adds to the plan before it, under "Everything in
 * Look, plus", which is the reading the good-better-best layout asks for.
 */
export const cardHighlights = (plan) => {
	const index = PLANS.indexOf(plan);
	const below = index > 0 ? PLANS[index - 1] : null;
	const lines = HIGHLIGHT_ORDER.map((key) => byKey.get(key))
		.filter((feature) => feature !== undefined && !feature.soon && reaches(feature, plan))
		.filter(
			(feature) =>
				below === null || feature.included[plan.id] !== feature.included[below.id]
		)
		.slice(0, HIGHLIGHTS_MAX)
		.map((feature) => highlight(feature, plan));
	return { base: below === null ? null : below.name, lines };
};

/** The pack a reader lands on from the packs strip: the middle rung. */
export const featuredPack = PACKS.find((pack) => pack.key === 'pack_100');

/** What a pack buys, per pack, with the per-move figure the rungs fall by. */
export const packRows = PACKS.map((pack) => ({
	key: pack.key,
	moves: pack.moves,
	price: dollars(pack.price_cents),
	perMove: dollars(pack.per_move_cents)
}));

/** What a move is, said once in a box above the cards that sell them: one
 *  sentence, and the rest behind Explain. */
export const moveDefinition = 'Publishing one of your resources onto one marketplace is one move.';

/** The long form behind the box's Explain. */
export const moveExplained = [
	'Publishing a resource to Tes and TPT is 2 moves; to Tes alone is 1 move. A draft counts the same as a live listing.',
	'Importing, editing, previewing and exporting never use a move.',
	'Plans add moves every month and save up unused ones. Pack moves last 12 months and work on any plan.'
];

/**
 * The questions, and the only place the site answers one.
 *
 * Eight, trimmed from eleven in the 2026-09-29 review: the pricing questions
 * a card or the comparison table does not already answer, and the two
 * questions about trust (the desktop app, where files go) that no table
 * can. Every figure in an answer is interpolated from the plan table.
 *
 * An answer may carry a `link`, drawn after its text: the console page that
 * acts on it, or the section of the terms that sets it out in full.
 * `/marketplaces` is where the app's downloads are, and a reader who is not
 * signed in is asked to sign in first.
 */
export const faqs = [
	{
		q: 'What is a move?',
		a: 'Publishing one imported resource onto one marketplace. Publishing it to two marketplaces is 2 moves; to one is 1 move. A draft counts the same as a live listing.'
	},
	{
		q: 'What is free?',
		a: `Importing your resources, editing them in Teachouse, previewing and exporting never use a move. On ${free.name} you also get ${free.capabilities.free_moves_lifetime} free moves when you connect your first shop.`
	},
	{
		q: 'Which plan is right for me?',
		a: `Pick by how often you publish: ${PLANS.filter((plan) => plan.capabilities.moves_per_month > 0)
			.map((plan) => `${plan.name} gives ${plan.capabilities.moves_per_month} moves a month`)
			.join(', ')}. Moving your shop once? A Move Pack is cheaper than subscribing.`
	},
	{
		q: 'What happens if I run out of moves?',
		a: 'Nothing is charged by surprise. Your next move waits for next month\u2019s moves, or you buy a Move Pack. Unused moves carry over, up to three months\u2019 worth, and pack moves last 12 months.'
	},
	{
		q: 'Can I change plans or cancel?',
		a: 'Yes, up or down at any time in Account \u2192 Billing. A cancelled plan runs to the end of the period you paid for, and your resources and moves stay where they are.'
	},
	{
		q: 'Can I get a refund?',
		a: 'Yes. A Move Pack you have not used is refunded in full within 14 days of buying it. A yearly plan is refunded for its unused whole months, less one month\u2019s fee. A monthly plan is not refunded once its month has started; cancelling stops the next renewal.',
		link: { href: '/terms/#refunds', label: 'How refunds work' }
	},
	{
		q: 'Why do I need the Teachouse app?',
		a: 'Some marketplaces, including the two that connect today, have no official way for other services to connect to them. For those, the free Teachouse app signs in on your own computer or phone, so Teachouse never holds your password. Connect those marketplaces in the app; importing, editing, publishing and everything else works in your browser too.',
		link: { href: `${consoleUrl}/marketplaces`, label: 'Get the app from Marketplaces' }
	},
	{
		q: 'Do my files get uploaded to Teachouse?',
		a: 'Not the files you import. They stay on your own device, and when you open one in your browser, Teachouse passes it across from that device without keeping it. Files you upload to Teachouse yourself are kept so you can use them anywhere, until you delete them.',
		link: { href: `${consoleUrl}/guides/why-the-app`, label: 'Why the app, and where your files stay' }
	}
];
