/**
 * What the landing page adds to the server's plan table.
 *
 * Every price, cap and capability now arrives from `plans.generated.js`, which
 * `cargo run -p tam-typegen` emits from `tam-limits` and `just web-check`
 * diffs, so the pricing page cannot drift from what the server actually
 * enforces. Changing a number is a change in the Rust table, never here.
 *
 * What stays here is what the landing alone owns: the phrasing of a rung, the
 * dollar formatting, the sentence under the ladder, the chips the subscription
 * card derives from `capabilities`, and the questions. Prices are quoted in
 * USD only: TPT is a US marketplace and Tes is UK-centred, so NZD is neither
 * buyer's currency and quoting it puts an FX conversion in front of a small
 * ticket.
 */

import { AI, FOUNDING, IMPORT_LADDER, LADDER_ABOVE, PLANS } from './plans.generated.js';

export { AI, FOUNDING };

/** `u32::MAX` is how the plan table spells "no cap". */
const UNCAPPED = 4294967295;

/** "$24", or "$24.50" where a price is not whole dollars. */
export const dollars = (cents) =>
	cents % 100 === 0 ? `$${cents / 100}` : `$${(cents / 100).toFixed(2)}`;

/**
 * The one subscription, as the server's table holds it. `studio` is in the
 * same table with `sold: false`; nothing on this site may render it.
 */
export const subscription = PLANS.find((plan) => plan.id === 'subscriber');

/** The cadence is the landing's own phrasing of `monthly_cents`/`yearly_cents`. */
export const subscriptionCadence = 'monthly or annual';

/**
 * The one-off import, priced by how much of a catalogue comes across, with
 * the open rung appended.
 *
 * The generated ladder is the five priced rungs; above them the founder's
 * answer is a conversation rather than a band, because the measured
 * dual-lister holds about 764 listings (decision of 2026-09-12). The open rung
 * is a display concern and so is made here rather than in the plan table.
 */
export const importLadder = [...IMPORT_LADDER, { up_to: null, label: LADDER_ABOVE }];

/** "Up to 250", or the open rung's own label. */
export const rungCap = (rung) => rung.label ?? `Up to ${rung.up_to}`;

/** "$247", or the invitation the open rung carries in place of a figure. */
export const rungPrice = (rung) =>
	rung.price_cents === undefined ? 'Ask us' : dollars(rung.price_cents);

/** Every rung that names a figure, which is the ladder minus the open one. */
export const pricedRungs = IMPORT_LADDER;

/**
 * What the cap counts. The ladder is priced by resources, and an import of two
 * marketplaces holding the same 300 resources commits 300 of them, not 600, so
 * the sentence is on the card rather than left to be discovered at the invoice.
 */
export const importLadderNote =
	'Counted as resources added to your catalogue after duplicates are merged.';

/** "every 6 hours", from the interval the plan table carries in seconds. */
const cadenceOf = (secs) => {
	if (secs % 86400 === 0) return secs === 86400 ? 'daily' : `every ${secs / 86400} days`;
	if (secs % 3600 === 0) return secs === 3600 ? 'hourly' : `every ${secs / 3600} hours`;
	return `every ${Math.round(secs / 60)} minutes`;
};

/**
 * The subscription card's lines, read off `capabilities` rather than typed out
 * beside them.
 *
 * A hand-written chip is a second price list: it goes stale the day a cap
 * moves in `tam-limits` and nothing fails. These are the capabilities a
 * teacher is choosing between, in the order they matter, and `soon` marks the
 * one line that is sold before it is built so a tick cannot claim otherwise.
 */
export const planFeatures = (plan) => {
	const caps = plan.capabilities;
	const lines = [
		{ text: `Up to ${caps.resources_max} resources` },
		{
			text: caps.marketplaces_max >= UNCAPPED ? 'All your marketplaces' : `${caps.marketplaces_max} marketplace`
		}
	];
	if (caps.import_spreadsheet && caps.import_marketplace) lines.push({ text: 'Import included' });
	if (caps.migrations_per_month > 0)
		lines.push({ text: `${caps.migrations_per_month} resources moved a month` });
	if (caps.scheduling) lines.push({ text: 'Scheduling' });
	if (caps.sync_pull_interval_secs !== null)
		lines.push({ text: `Sync ${cadenceOf(caps.sync_pull_interval_secs)}` });
	if (caps.templates_max > 0) lines.push({ text: `${caps.templates_max} templates` });
	if (caps.collections_max > 0) lines.push({ text: `${caps.collections_max} collections` });
	if (caps.analytics) lines.push({ text: 'Analytics' });
	lines.push({ text: caps.devices_max === 1 ? '1 computer' : `${caps.devices_max} computers` });
	if (caps.ai_fills_per_month > 0 && AI.status === 'coming_soon')
		lines.push({
			text: `AI fill \u2014 coming soon (${caps.ai_fills_per_month} a month)`,
			soon: true
		});
	return lines;
};

/**
 * The questions, and the only place the site answers one.
 *
 * Every figure in an answer is interpolated from the plan table above, so the
 * price list and the questions about it cannot disagree.
 */
export const faqs = [
	{
		q: 'Do you get my marketplace password?',
		a: 'No. You sign in to each marketplace yourself, in a window on your own computer, and what is kept is the session that sign-in creates, on that computer, never your password.'
	},
	{
		q: 'Do my files go through your servers?',
		a: 'No. Where a marketplace publishes no interface for tools like this one, every request to it is sent from your own computer under your own login, and an upload is one of those requests.'
	},
	{
		q: 'Do I need to install anything?',
		a: 'For the marketplaces that publish no interface, yes: a small app you install once on the computer that does that work. Your catalogue, your settings and your results are on our side and open in any browser.'
	},
	{
		q: 'What happens to a listing when I change it?',
		a: 'Teachouse edits the listing that is already there. It does not delete your listing and create a new one, which is the thing that would cost you its address, its reviews and the followers attached to it.'
	},
	{
		q: 'What does a catalogue import include?',
		a: `We bring your existing resources into Teachouse for you, at the one-off price of the band you pick, from ${rungPrice(pricedRungs[0])} for ${rungCap(pricedRungs[0]).toLowerCase()} resources to ${rungPrice(pricedRungs[pricedRungs.length - 1])} for ${rungCap(pricedRungs[pricedRungs.length - 1]).toLowerCase()}, and a conversation above that. A resource you sell in two places is counted once, after we merge the duplicates. You look over every listing we created before anything is published.`
	},
	{
		q: 'Do I pay for import?',
		a: 'Not on the subscription: bringing your catalogue in is part of it. The one-off price is for a seller who wants their catalogue brought across without subscribing.'
	},
	{
		q: 'What does AI fill do?',
		a: `It reads the file you are listing and fills the form in from it, for you to check and change before anything is saved. It never writes to a marketplace on its own. It is coming soon, ${AI.included_fills} fills a month are part of the subscription, and we are not putting a date on it.`
	},
	{
		q: 'What is the Founding 100?',
		a: `The first ${FOUNDING.places} teachers to join: ${FOUNDING.discount_year_one_pct}% off your first year, ${FOUNDING.discount_ongoing_pct}% off for the ${FOUNDING.ongoing_years} years after that, and your first ${FOUNDING.free_imports} resources imported free. When the ${FOUNDING.places} places are taken, the offer closes.`
	},
	{
		q: 'Can I pay monthly?',
		a: 'Yes. The subscription is monthly or annual, the annual price is two months cheaper, and you can cancel from the billing screen. Prices are in US dollars.'
	},
	{
		q: 'Are you part of any of these marketplaces?',
		a: 'No. Teachouse is an independent tool, no marketplace endorses it, and your relationship with each one stays your own.'
	}
];
