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

import { AI, FOUNDING, PACKS, PLANS, SERVICES } from './plans.generated.js';

export { AI, FOUNDING, PACKS };

/** `u32::MAX` is how the plan table spells "no cap". */
const UNCAPPED = 4294967295;

/** A price in dollars, showing cents only where a price is not whole. */
export const dollars = (cents) =>
	cents % 100 === 0 ? `$${cents / 100}` : `$${(cents / 100).toFixed(2)}`;

/** The free plan, which the table names "Look". */
export const free = PLANS.find((plan) => plan.id === 'free');

/**
 * The one subscription, as the server's table holds it. `studio` is in the
 * same table with `sold: false`; nothing on this site may render it.
 */
export const subscription = PLANS.find((plan) => plan.id === 'subscriber');

/**
 * The headline figure is the annual price divided across its months, because
 * that is the number a reader compares against another tool's monthly price.
 * The true monthly price sits beside it in smaller type rather than being
 * left for the checkout to introduce.
 */
export const subscriptionPerMonth = Math.round(subscription.yearly_cents / 12);

/** The service the table sells beside the plans, currently one booking. */
export const moveWithMe = SERVICES.find((service) => service.key === 'move_with_me');

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
 * "Start Sync" does not have to find Sync again on the other side.
 */
const SIGNUP = 'https://teachouse.io/signup';
export const signupUrl = (priceKey) =>
	priceKey === undefined
		? `${SIGNUP}?next=/settings/subscription`
		: `${SIGNUP}?next=/settings/subscription&price=${priceKey}`;

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

/**
 * "31 December 2026", spelled out here rather than by `toLocaleDateString`:
 * the date is rendered once at build time into a static page, and a build
 * machine with a trimmed ICU would quietly print a different string.
 */
const spellDate = (iso) => {
	const [year, month, day] = iso.split('-').map(Number);
	return `${day} ${MONTHS[month - 1]} ${year}`;
};

/** "31 December 2026", the day the founding offer stops being offered. */
export const foundingCloses = spellDate(FOUNDING.closes_at);

/**
 * Whether the founding band is drawn at all.
 *
 * This is a static site: there is no request to evaluate the date against, so
 * the question is answered when the page is built and the answer is frozen
 * into the HTML. That is why the closing date is printed beside the offer —
 * a reader who finds a stale build can see for themselves that it has passed,
 * and a rebuild drops the band.
 */
export const foundingOpen = Date.parse(`${FOUNDING.closes_at}T23:59:59Z`) > Date.now();

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
 * plan page (`web/src/lib/pages/account/plans.ts`) says the same lines.
 *
 * A cap that is not there is not a line: an unlimited catalogue says nothing
 * rather than "No limit on resources", which the review asked to remove.
 */
export const planFeatures = (plan) => {
	const caps = plan.capabilities;
	const lines = [];
	if (caps.free_moves_lifetime > 0)
		lines.push({ text: `${caps.free_moves_lifetime} moves onto a marketplace of your choice` });
	if (caps.moves_per_month > 0) lines.push({ text: `${caps.moves_per_month} moves a month` });
	if (caps.moves_accrual_cap > 0)
		lines.push({ text: `Unused moves stack to ${caps.moves_accrual_cap}` });
	if (caps.import_spreadsheet && caps.import_marketplace)
		lines.push({ text: 'Import all your resources from wherever you sell' });
	if (caps.resources_max < UNCAPPED) lines.push({ text: `Up to ${caps.resources_max} resources` });
	if (caps.sync_pull_interval_secs !== null)
		lines.push({ text: 'Edit resources in Teachouse and sync the edits across all platforms' });
	if (caps.scheduling) lines.push({ text: 'Scheduling' });
	if (caps.templates_max > 1) lines.push({ text: `${caps.templates_max} templates` });
	if (caps.collections_max > 0) lines.push({ text: `${caps.collections_max} collections` });
	if (caps.analytics) lines.push({ text: 'Statistics on every shop' });
	if (caps.ai_fills_per_month > 0 && AI.status === 'coming_soon')
		lines.push({
			text: `AI fill, coming soon (${caps.ai_fills_per_month} a month)`,
			soon: true
		});
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
 *
 * The founding question rides on the same gate as the band: once the offer
 * has closed, answering a question about how to take it is an advertisement
 * for something nobody can buy.
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
		q: 'Move Packs or Sync \u2014 which is right for me?',
		a: `Buy a Move Pack if you are moving your shop once. Choose Sync if you add new resources often: you get ${subscription.capabilities.moves_per_month} moves a month, scheduling and statistics, and your edits stay in step on every marketplace.`
	},
	{
		q: 'Do moves run out?',
		a: `Move Pack moves last 12 months from the day you buy them. With Sync, moves you do not use carry over, up to ${subscription.capabilities.moves_accrual_cap}.`
	},
	{
		q: 'Can I change a listing after I move it?',
		a: `Yes. Within ${free.capabilities.pack_edit_days} days of a move you can edit that listing once and send the change without using another move.`
	},
	{
		q: 'Can I get a refund?',
		a: 'Yes, for a Move Pack you have not used: write to us within 14 days of buying it. You can cancel Sync at any time in Account \u2192 Subscription, and it runs until the end of the period you paid for.'
	},
	...(foundingOpen
		? [
				{
					q: `What is the Founding ${FOUNDING.places}?`,
					a: `Our launch offer, paid yearly: ${dollars(FOUNDING.year_one_cents)} for your first year, then ${dollars(FOUNDING.ongoing_cents)} a year until year ${FOUNDING.ongoing_years}, plus ${FOUNDING.extra_moves} extra moves. It closes when ${FOUNDING.places} teachers have joined, or on ${foundingCloses}.`
				}
			]
		: []),
	{
		q: 'What is "Move with me"?',
		a: `A ${dollars(moveWithMe.price_cents)} session where we move your shop together with you. The moves themselves come from a Move Pack.`
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
