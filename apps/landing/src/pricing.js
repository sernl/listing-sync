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

/** "every 6 hours", from the interval the plan table carries in seconds. */
const cadenceOf = (secs) => {
	if (secs % 86400 === 0) return secs === 86400 ? 'daily' : `every ${secs / 86400} days`;
	if (secs % 3600 === 0) return secs === 3600 ? 'hourly' : `every ${secs / 3600} hours`;
	return `every ${Math.round(secs / 60)} minutes`;
};

/**
 * A plan card's lines, read off `capabilities` rather than typed out beside
 * them.
 *
 * A hand-written chip is a second price list: it goes stale the day a cap
 * moves in `tam-limits` and nothing fails. These are the capabilities a
 * teacher is choosing between, in the order they matter, and `soon` marks the
 * one line that is sold before it is built so a tick cannot claim otherwise.
 */
export const planFeatures = (plan) => {
	const caps = plan.capabilities;
	const lines = [];
	if (caps.moves_per_month > 0) lines.push({ text: `${caps.moves_per_month} moves a month` });
	if (caps.moves_accrual_cap > 0)
		lines.push({ text: `Unused moves stack to ${caps.moves_accrual_cap}` });
	if (caps.free_moves_lifetime > 0)
		lines.push({ text: `${caps.free_moves_lifetime} free moves for each shop` });
	lines.push({
		text:
			caps.resources_max >= UNCAPPED ? 'No limit on resources' : `Up to ${caps.resources_max} resources`
	});
	if (caps.import_spreadsheet && caps.import_marketplace) lines.push({ text: 'Import included' });
	if (caps.scheduling) lines.push({ text: 'Scheduling' });
	if (caps.sync_pull_interval_secs !== null)
		lines.push({ text: `Pulls ${cadenceOf(caps.sync_pull_interval_secs)}` });
	if (caps.templates_max > 1) lines.push({ text: `${caps.templates_max} templates` });
	if (caps.collections_max > 0) lines.push({ text: `${caps.collections_max} collections` });
	if (caps.analytics) lines.push({ text: 'Figures on every shop' });
	if (caps.ai_fills_per_month > 0 && AI.status === 'coming_soon')
		lines.push({
			text: `AI fill \u2014 coming soon (${caps.ai_fills_per_month} a month)`,
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
export const packEditNote = `Edit a moved listing for ${free.capabilities.pack_edit_days} days without spending another move.`;

/**
 * The questions, and the only place the site answers one.
 *
 * Every figure in an answer is interpolated from the plan table above, so the
 * price list and the questions about it cannot disagree. The answers are the
 * ones in `docs/guides/faq.md`, cut to two sentences: a reader on this page
 * is deciding, not learning the product.
 *
 * The founding question rides on the same gate as the band: once the offer
 * has closed, answering a question about how to take it is an advertisement
 * for something nobody can buy.
 */
export const faqs = [
	{
		q: 'What is a move?',
		a: 'One resource committed to the other marketplace. It is counted when you commit it, after duplicates are merged, and a draft costs the same as a live listing.'
	},
	{
		q: 'What does not cost a move?',
		a: 'Importing, reading, editing in Teachouse, exporting and previewing all cost nothing. Only committing a resource to a marketplace does.'
	},
	{
		q: 'What do the free moves get me?',
		a: `${free.capabilities.free_moves_lifetime} moves for each shop, given once when the shop first connects. A shop never gets a second set.`
	},
	{
		q: 'Packs or Sync \u2014 which do I want?',
		a: `Buy a pack if you are moving a shop once. Take Sync if you list every week and want schedules, pulls and figures as well as ${subscription.capabilities.moves_per_month} moves a month.`
	},
	{
		q: 'Do moves expire?',
		a: `Pack moves are good for 12 months from the day you buy them. Sync's monthly moves stack up to ${subscription.capabilities.moves_accrual_cap}; above that, a new month adds none.`
	},
	{
		q: 'Can I edit a listing after I move it?',
		a: `Yes. For ${free.capabilities.pack_edit_days} days after a move you can edit that listing and send the change without spending another move.`
	},
	{
		q: 'Can I get a refund?',
		a: 'Write to us within 14 days of buying a pack and we will refund it if the moves are unspent. Cancel a subscription at any time from Account \u2192 Subscription; the current period is not refunded.'
	},
	...(foundingOpen
		? [
				{
					q: `What is the Founding ${FOUNDING.places}?`,
					a: `Annual only: ${dollars(FOUNDING.year_one_cents)} for the first year, ${dollars(FOUNDING.ongoing_cents)} a year through year ${FOUNDING.ongoing_years}, and ${FOUNDING.extra_moves} moves on top. It closes at ${FOUNDING.places} members or on ${foundingCloses}.`
				}
			]
		: []),
	{
		q: 'What is "Move with me"?',
		a: `A ${dollars(moveWithMe.price_cents)} session where we do your move with you. It is a booking, so buy a pack for the moves themselves.`
	},
	{
		q: 'Which marketplaces can I use?',
		a: 'Tes and Teachers Pay Teachers, in both directions.'
	},
	{
		q: 'Why do I need the desktop app?',
		a: 'Tes and TPT are reached through your own signed-in browser session on your own machine. The app provides that session and keeps your marketplace password on your machine.'
	},
	{
		q: 'Do my files get uploaded to Teachouse?',
		a: 'No. Files stay on your machines and are sent to a marketplace only when you publish. The thumbnail is the one exception, stored so the console can show it.'
	}
];
