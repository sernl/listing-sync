/**
 * What the landing page adds to the server's plan table.
 *
 * Every price, cap and capability arrives from `plans.generated.js`, which
 * `cargo run -p tam-typegen` emits from `tam-limits` and `just web-check`
 * diffs, so the pricing page cannot drift from what the server actually
 * enforces. Changing a number is a change in the Rust table, never here.
 *
 * What stays here is what the landing alone owns: the phrasing of a card, the
 * dollar formatting, the chips each plan derives from `capabilities`, the
 * signup links that carry a price key across to the console, and the
 * questions. Prices are quoted in USD only: TPT is a US marketplace and Tes is
 * UK-centred, so NZD is neither buyer's currency and quoting it puts an FX
 * conversion in front of a small ticket.
 */

import { AI, PACKS, PLANS } from './plans.generated.js';

export { AI, PACKS };

/** `u32::MAX` is how the plan table spells "no cap". */
const UNCAPPED = 4294967295;

/** A price in dollars, showing cents only where a price is not whole. */
export const dollars = (cents) =>
	cents % 100 === 0 ? `$${cents / 100}` : `$${(cents / 100).toFixed(2)}`;

/** The free plan, which the table names "Look". */
export const free = PLANS.find((plan) => plan.id === 'free');

/** The three paid plans, cheapest first, in the table's own order. */
export const paidPlans = PLANS.filter((plan) => plan.yearly_cents !== null);

/**
 * The plan we expect most sellers to want, drawn with the ring. Sync, the
 * middle rung: the one a teacher adding resources every week lands on.
 */
export const leadPlan = 'subscriber';

/** Who each paid plan is for, in one sentence under its name. */
export const planPitch = {
	starter: 'For teachers who add a resource now and then.',
	subscriber: 'For teachers who add resources every week.',
	studio: 'For big catalogues and whole-shop moves.'
};

/**
 * The headline figure is the annual price divided across its months, because
 * that is the number a reader compares against another tool's monthly price.
 * The true monthly price sits beside it in smaller type rather than being
 * left for the checkout to introduce.
 */
export const perMonthYearly = (plan) => Math.round(plan.yearly_cents / 12);

/**
 * The pack a reader lands on from the packs card.
 *
 * The middle rung is the one a seller moving a shop usually wants, and a card
 * whose button leads nowhere in particular is a card with no button.
 */
export const featuredPack = PACKS.find((pack) => pack.key === 'pack_100');

/**
 * Where a call to action goes, and what it carries.
 *
 * The console's signup reads `next` and `price` from its query: `next` is
 * where to land after the account exists, and `price` is the checkout to open
 * on arrival. Sending the price key from here means a reader who clicked
 * "Choose Sync" does not have to find Sync again on the other side.
 */
const SIGNUP = 'https://teachouse.io/signup';
export const signupUrl = (priceKey) =>
	priceKey === undefined || priceKey === null
		? `${SIGNUP}?next=/settings/billing`
		: `${SIGNUP}?next=/settings/billing&price=${priceKey}`;

/**
 * A plan card's lines, read off `capabilities` rather than typed out beside
 * them.
 *
 * A hand-written chip is a second price list: it goes stale the day a cap
 * moves in `tam-limits` and nothing fails. These are the capabilities a
 * teacher is choosing between, in the order they matter, worded for a
 * teacher rather than for whoever built them (the founder's 2026-09-26
 * review: no "pulls every 6 hours"), and `soon` marks the one line that is
 * sold before it is built so a tick cannot claim otherwise. The console's
 * Billing page (`web/src/lib/pages/account/plans.ts`) says the same lines.
 *
 * The order is the founder's PDF review of 2026-09-27: what every plan does
 * first, the counts in the middle, and the free plan's trial moves last. A
 * cap that is not there is not a line: an unlimited catalogue says nothing
 * rather than "No limit on resources", which the 2026-09-26 review asked to
 * remove.
 */
export const planFeatures = (plan) => {
	const caps = plan.capabilities;
	const count = (n, noun) => (n === UNCAPPED ? `Unlimited ${noun}` : `${n} ${noun}`);
	const lines = [];
	if (caps.import_spreadsheet && caps.import_marketplace)
		lines.push({ text: 'Import from wherever you sell' });
	if (caps.sync_pull_interval_secs !== null) lines.push({ text: 'Edit once, sync everywhere' });
	if (caps.moves_per_month > 0) lines.push({ text: `${caps.moves_per_month} moves a month` });
	if (caps.moves_accrual_cap > 0)
		lines.push({ text: `Unused moves stack to ${caps.moves_accrual_cap}` });
	if (caps.resources_max < UNCAPPED) lines.push({ text: `Up to ${caps.resources_max} resources` });
	lines.push({ text: 'Add a watermarked preview of your file' });
	if (caps.scheduling) lines.push({ text: 'Scheduling' });
	if (caps.templates_max > 1) lines.push({ text: count(caps.templates_max, 'templates') });
	if (caps.collections_max > 0) lines.push({ text: count(caps.collections_max, 'collections') });
	if (caps.analytics) lines.push({ text: 'Statistics on every shop' });
	if (caps.auto_publish_rules) lines.push({ text: 'Automatic publishing rules' });
	if (caps.support === 'email_1_day') lines.push({ text: 'Priority support' });
	if (caps.ai_fills_per_month > 0 && AI.status === 'coming_soon')
		lines.push({ text: 'AI description fill, coming soon', soon: true });
	if (caps.free_moves_lifetime > 0)
		lines.push({ text: `${caps.free_moves_lifetime} moves onto a marketplace of your choice` });
	return lines;
};

/**
 * What a pack buys, per pack. The per-move figure is the reason the table has
 * three columns: the rungs are priced to reward a bigger commitment, and a
 * reader cannot see that from the price alone.
 */
export const packRows = PACKS.map((pack) => ({
	key: pack.key,
	moves: pack.moves,
	price: dollars(pack.price_cents),
	perMove: dollars(pack.per_move_cents)
}));

/** The edit window, said on the packs card rather than at the first re-edit. */
export const packEditNote = `Edit a moved listing once within ${free.capabilities.pack_edit_days} days without spending another move.`;

/** What a move is, said once under the heading of the cards that sell them. */
export const moveDefinition =
	'A move is publishing one imported resource onto one marketplace. Publishing a resource to Tes and TPT is 2 moves; to Tes alone is 1 move.';

/**
 * The questions, and the only place the site answers one.
 *
 * Every figure in an answer is interpolated from the plan table above, so the
 * price list and the questions about it cannot disagree. The answers follow
 * `docs/guides/faq.md`, cut to two short sentences and written the way a
 * teacher would say them, not the way the system counts them: a reader on
 * this page is deciding, not learning the product.
 */
export const faqs = [
	{
		q: 'What is a move?',
		a: 'Publishing one imported resource onto one marketplace. Publishing it to two marketplaces is 2 moves; to one is 1 move. A draft counts the same as a live listing.'
	},
	{
		q: 'What is free?',
		a: 'Importing your resources, editing them in Teachouse, previewing and exporting never use a move. You only use a move when you publish a resource onto a marketplace.'
	},
	{
		q: 'What do the free moves get me?',
		a: `When you connect your first shop, you get ${free.capabilities.free_moves_lifetime} free moves to publish onto a marketplace of your choice. Every account gets them once.`
	},
	{
		q: 'Which plan is right for me?',
		a: `Buy a Move Pack if you are moving your shop once. Otherwise pick by how often you publish: ${paidPlans.map((plan) => `${plan.name} gives ${plan.capabilities.moves_per_month} moves a month`).join(', ')}.`
	},
	{
		q: 'Do moves run out?',
		a: 'Move Pack moves last 12 months from the day you buy them. On a paid plan, moves you do not use carry over, up to three months\u2019 worth.'
	},
	{
		q: 'Can I change a listing after I move it?',
		a: `Yes. Within ${free.capabilities.pack_edit_days} days of a move you can edit that listing once and send the change without using another move.`
	},
	{
		q: 'Can I get a refund?',
		a: 'Yes, for a Move Pack you have not used: write to us within 14 days of buying it. You can cancel a plan at any time in Account \u2192 Billing, and it runs until the end of the period you paid for.'
	},
	{
		q: 'Can I change plans?',
		a: 'Yes, up or down at any time in Account \u2192 Billing. Your resources and the moves you already hold stay where they are.'
	},
	{
		q: 'Which marketplaces can I use?',
		a: 'Tes and Teachers Pay Teachers. You can move resources either way between them.'
	},
	{
		q: 'Why do I need the desktop app?',
		a: 'The app signs in to your marketplaces for you, on your own computer. That way your marketplace passwords never leave it.'
	},
	{
		q: 'Do my files get uploaded to Teachouse?',
		a: 'No. Your files stay on your computer and go straight to a marketplace when you publish. We keep only the cover picture, so you can see your resources in Teachouse.'
	}
];
