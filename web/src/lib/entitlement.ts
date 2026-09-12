// What the seller's plan lets them do, rendered into the sentences the
// console says about it. Pure, so it tests without a component.
//
// Every refusal here is courtesy: the server computes the same `Capabilities`
// on every request and refuses the route itself with a 422. This module
// exists so the refusal arrives before the seller has filled a form, and so
// the sentence explaining it is written once rather than once per page.

import type { Capabilities, EntitlementUsage, Grant } from '$lib/api';
import type { SupportLevel } from '$lib/generated/vocab';
import type { SectionId } from '$lib/nav';

/** The figure a capability carries where the answer is "every one there is".
 *  `u32::MAX` on the Rust side, which is what the wire carries. */
export const UNLIMITED = 4_294_967_295;

/** Whether a maximum is the no-limit sentinel rather than a number to count
 *  against. */
export function unlimited(max: number): boolean {
	return max >= UNLIMITED;
}

// ------------------------------------------------------------------ sections

/** Why a section is not on this plan, or null where it is.
 *
 * Section-level rather than page-level because the rail is drawn from
 * sections; the pages inside a reachable section carry their own reasons.
 * Crosslist is reachable on every plan because Export lives inside it and
 * export is never gated — a seller who cannot get their catalogue out will
 * not put one in — so its pages, not the section, are what a thin plan
 * refuses. */
export function sectionReason(caps: Capabilities, section: SectionId): string | null {
	switch (section) {
		case 'import':
			return caps.import_spreadsheet || caps.import_marketplace
				? null
				: 'Your plan does not include import. Upgrade to bring your portfolio in.';
		case 'crosslist':
			return null;
		case 'automations':
			return caps.scheduling || caps.sync_pull_interval_secs !== null || caps.migrations_per_month > 0
				? null
				: 'Your plan does not include automations. Upgrade to schedule, sync, and move resources between marketplaces.';
		case 'marketplaces':
			return caps.marketplaces_max > 0
				? null
				: 'Your plan connects no marketplaces. Upgrade to connect one.';
		// Account is how a plan is bought, so it is never the thing a plan
		// withholds. Admin is the operator's own section and answers to the
		// operator probe, not to a plan.
		case 'account':
		case 'admin':
			return null;
	}
}

/** Whether this plan reaches a section at all. */
export function sectionAllowed(caps: Capabilities, section: SectionId): boolean {
	return sectionReason(caps, section) === null;
}

// ------------------------------------------------------------------ features

/** The capabilities a control is switched on or off by, named as the console
 *  asks about them. `sync` is `sync_pull_interval_secs`, which answers "how
 *  often" and null for "never". */
export type Feature =
	| 'import_spreadsheet'
	| 'import_marketplace'
	| 'duplicate_review'
	| 'scheduling'
	| 'sync'
	| 'auto_publish_rules'
	| 'analytics';

/** Why a control is disabled, or null where it is not.
 *
 * One sentence saying what the plan does not carry and one saying what to do
 * about it. Both are needed: a disabled control with no stated reason reads
 * as a fault, and a stated reason with no way forward reads as a dead end. */
export function featureReason(caps: Capabilities, feature: Feature): string | null {
	switch (feature) {
		case 'import_spreadsheet':
			return caps.import_spreadsheet
				? null
				: 'Your plan does not include spreadsheet import. Upgrade to bring a spreadsheet in.';
		case 'import_marketplace':
			return caps.import_marketplace
				? null
				: 'Your plan cannot read your shop from a marketplace. Upgrade to import from one.';
		case 'duplicate_review':
			return caps.duplicate_review
				? null
				: 'Your plan does not include duplicate review. Upgrade to merge duplicates before they are committed.';
		case 'scheduling':
			return caps.scheduling
				? null
				: 'Your plan does not include scheduling. Upgrade to publish on a timetable.';
		case 'sync':
			return caps.sync_pull_interval_secs !== null
				? null
				: 'Your plan does not include marketplace sync. Upgrade to pull changes from your marketplaces.';
		case 'auto_publish_rules':
			return caps.auto_publish_rules
				? null
				: 'Your plan does not include auto-publish rules. Upgrade to republish a listing when its resource changes.';
		case 'analytics':
			return caps.analytics
				? null
				: 'Your plan does not include analytics. Upgrade to see how your listings are doing.';
	}
}

// -------------------------------------------------------------------- limits

/** The counted allowances, named as the console asks about them. */
export type Limit =
	| 'resources'
	| 'marketplaces'
	| 'templates'
	| 'collections'
	| 'labels'
	| 'devices';

/** What each counted allowance is called, and what adding to it is called.
 *
 * The verb differs per allowance — a marketplace is connected, a machine is
 * signed in — and a generic "add more" on every one of them was the reading
 * that made the marketplace cap sound like a catalogue cap. */
const LIMITS: Record<Limit, { one: string; many: string; verb: string }> = {
	resources: { one: 'resource', many: 'resources', verb: 'add more' },
	marketplaces: { one: 'marketplace', many: 'marketplaces', verb: 'connect more' },
	templates: { one: 'template', many: 'templates', verb: 'add more' },
	collections: { one: 'collection', many: 'collections', verb: 'add more' },
	labels: { one: 'label', many: 'labels', verb: 'add more' },
	devices: { one: 'device', many: 'devices', verb: 'sign in on more' }
};

function maxOf(caps: Capabilities, limit: Limit): number {
	switch (limit) {
		case 'resources':
			return caps.resources_max;
		case 'marketplaces':
			return caps.marketplaces_max;
		case 'templates':
			return caps.templates_max;
		case 'collections':
			return caps.collections_max;
		case 'labels':
			return caps.labels_max;
		case 'devices':
			return caps.devices_max;
	}
}

function usedOf(usage: EntitlementUsage, limit: Limit): number {
	switch (limit) {
		case 'resources':
			return usage.resources;
		case 'marketplaces':
			return usage.marketplaces;
		case 'templates':
			return usage.templates;
		case 'collections':
			return usage.collections;
		case 'labels':
			return usage.labels;
		case 'devices':
			return usage.devices;
	}
}

/** A count and its noun, agreeing. Every figure this module prints goes
 *  through here: "1 marketplaces" was the reading that made the free plan's
 *  one connection look like a fault. */
function counted(n: number, one: string, many: string): string {
	return `${n} ${n === 1 ? one : many}`;
}

/** Why a counted allowance is full, or null where there is room.
 *
 * At-or-over rather than over: the control is being drawn before the write,
 * so the question is whether one more would fit. A plan whose allowance is
 * zero says so in the same sentence, because "includes 0 labels" is the
 * honest reading of a plan that carries only the marketplaces' own. */
export function limitReason(
	caps: Capabilities,
	usage: EntitlementUsage,
	limit: Limit
): string | null {
	const max = maxOf(caps, limit);
	if (unlimited(max) || usedOf(usage, limit) < max) {
		return null;
	}
	return `Your plan includes ${counted(max, LIMITS[limit].one, LIMITS[limit].many)}. Upgrade to ${LIMITS[limit].verb}.`;
}

// --------------------------------------------------------------- usage lines

/** One allowance, as the Account page reads it out: "12 of 20 resources".
 *
 * An unlimited allowance states the count and says so rather than printing
 * `4294967295`, which is the sentinel and not a promise. */
export function usageLine(used: number, max: number, limit: Limit): string {
	const { one, many } = LIMITS[limit];
	if (unlimited(max)) {
		return `${counted(used, one, many)}, with no limit on your plan`;
	}
	return `${used} of ${counted(max, one, many)}`;
}

export interface UsageRow {
	limit: Limit;
	line: string;
	/** Whether this allowance is full, which is what the page marks. */
	full: boolean;
}

/** Every counted allowance, in the order the Account page lists them. */
export function usageLines(usage: EntitlementUsage, caps: Capabilities): UsageRow[] {
	const order: Limit[] = [
		'resources',
		'marketplaces',
		'templates',
		'collections',
		'labels',
		'devices'
	];
	return order.map((limit) => ({
		limit,
		line: usageLine(usedOf(usage, limit), maxOf(caps, limit), limit),
		full: limitReason(caps, usage, limit) !== null
	}));
}

/** The months, spelled out, so a date reads the same in every browser.
 *
 * `toLocaleDateString` would put the month first for an American locale and
 * the figures in the page are written as "1 October" throughout. */
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

/** A day and its month, in UTC, which is the month the server counts in. */
export function dayMonth(instant: number): string {
	const at = new Date(instant);
	return `${at.getUTCDate()} ${MONTHS[at.getUTCMonth()]}`;
}

/** The migration allowance and when it returns: "8 of 20 this month; resets 1
 *  October".
 *
 * The reset date is stated rather than implied, because the seller's question
 * at the cap is when they can move the rest and a monthly allowance with no
 * date attached does not answer it. A plan that migrates nothing has no month
 * to reset, and says that instead. */
export function migrationsLine(usage: EntitlementUsage, caps: Capabilities): string {
	if (caps.migrations_per_month === 0) {
		return 'Your plan moves no resources between marketplaces.';
	}
	return `${usage.migrations_this_month} of ${caps.migrations_per_month} this month; resets ${dayMonth(usage.migrations_reset_at)}`;
}

/** The monthly allowance as a control reads it: the sentence to print, and the
 *  refusal that disables the confirm where the selection does not fit.
 *
 * Structural parameters rather than `Capabilities` and `EntitlementUsage`
 * themselves, so the same writer serves both doors this figure arrives
 * through: the entitlement read the shell already holds, and the `cap` block
 * a migration plan answers with. Both satisfy these shapes, and the seller
 * must not be told one figure before the preview and another after it.
 *
 * The refusal is a separate field rather than a null line, because a seller
 * over the cap still needs the figures — how many are left and when the month
 * turns — beside the reason they cannot press the button. */
export function migrationsReason(
	caps: { migrations_per_month: number },
	usage: { migrations_this_month: number; migrations_reset_at: number },
	requested: number
): { line: string; refusal: string | null } {
	const limit = caps.migrations_per_month;
	if (limit === 0) {
		return {
			line: 'Your plan moves no resources between marketplaces.',
			refusal: 'Your plan does not move resources between marketplaces. Upgrade to migrate.'
		};
	}
	const left = Math.max(0, limit - usage.migrations_this_month);
	const resets = dayMonth(usage.migrations_reset_at);
	const line =
		requested === 0
			? `You have ${left} of ${limit} moves left this month.`
			: `You have ${left} of ${limit} moves left this month; this uses ${requested}.`;
	if (requested > left) {
		return {
			line,
			refusal: `Your plan moves ${limit} resources a month and you have ${left} left, so ${requested} is more than this month can take. It resets ${resets}.`
		};
	}
	return { line: `${line} Resets ${resets}.`, refusal: null };
}

/** Why this organisation is on the plan it is on, where a person put it
 *  there.
 *
 * Null for a plan that arrived through checkout: Paddle's own record is what
 * the billing panel already shows, and a notice over it would be a second
 * account of the same fact. A manual grant is the one the seller cannot
 * explain from their own receipts, so it is the one that gets a sentence. */
export function grantNotice(grant: Grant): string | null {
	if (grant.granted_by !== 'operator') {
		return null;
	}
	return grant.expires_at === null
		? 'Set by Teachouse, with no end date.'
		: `Set by Teachouse until ${dayMonth(grant.expires_at)}.`;
}

// --------------------------------------------------------- the plan's offer

/** What each level of support promises, in the words the plan card uses. */
const SUPPORT: Record<SupportLevel, string> = {
	guides: 'Guides',
	email_2_days: 'Email support, two business days',
	email_1_day: 'Email support, one business day',
	email_30_days_after_purchase: 'Email support for 30 days after your purchase'
};

/** How often a plan pulls, read out. */
function syncLine(seconds: number): string {
	const hours = Math.round(seconds / 3600);
	if (hours <= 1) {
		return 'Sync every hour';
	}
	return `Sync every ${hours} hours`;
}

/**
 * What a plan includes, as its card lists it.
 *
 * Only what the plan carries. A card that also listed what it withholds
 * would be a comparison table with one column, and the sections the plan
 * does not reach already say so where the seller meets them. The order is
 * the founder's own table: catalogue, marketplaces, import, publishing,
 * automations, the counted objects, then support.
 */
export function capabilityLines(caps: Capabilities): string[] {
	const lines: string[] = [];
	lines.push(
		unlimited(caps.resources_max)
			? 'Unlimited resources'
			: counted(caps.resources_max, 'resource', 'resources')
	);
	lines.push(
		unlimited(caps.marketplaces_max)
			? 'All marketplaces'
			: `${counted(caps.marketplaces_max, 'marketplace', 'marketplaces')} connected`
	);
	if (caps.import_spreadsheet && caps.import_marketplace) {
		lines.push('Import from a spreadsheet or a marketplace');
	} else if (caps.import_spreadsheet) {
		lines.push('Import from a spreadsheet');
	} else if (caps.import_marketplace) {
		lines.push('Import from a marketplace');
	}
	if (caps.duplicate_review) {
		lines.push('Duplicate review before anything is committed');
	}
	lines.push(
		unlimited(caps.publish_marketplaces_max)
			? 'Publish to every marketplace you have connected'
			: `Publish to ${counted(caps.publish_marketplaces_max, 'marketplace', 'marketplaces')}`
	);
	if (caps.edit_days_after_purchase !== null) {
		lines.push(`Edit and delete your listings for ${caps.edit_days_after_purchase} days`);
	}
	if (caps.migrations_per_month > 0) {
		lines.push(`${caps.migrations_per_month} resources copied or moved a month`);
	}
	if (caps.scheduling) {
		lines.push('Schedule what publishes when');
	}
	if (caps.sync_pull_interval_secs !== null) {
		lines.push(syncLine(caps.sync_pull_interval_secs));
	}
	if (caps.auto_publish_rules) {
		lines.push('Republish a listing when its resource changes');
	}
	if (caps.templates_max > 0) {
		lines.push(counted(caps.templates_max, 'template', 'templates'));
	}
	if (caps.collections_max > 0) {
		lines.push(counted(caps.collections_max, 'collection', 'collections'));
	}
	if (caps.labels_max > 0) {
		lines.push(counted(caps.labels_max, 'label', 'labels'));
	}
	if (caps.analytics) {
		lines.push('Analytics');
	}
	if (caps.export) {
		lines.push('Spreadsheet export');
	}
	lines.push(counted(caps.devices_max, 'device', 'devices'));
	if (caps.ai_fills_per_month > 0) {
		lines.push(`${caps.ai_fills_per_month} AI auto-fills a month, once AI arrives`);
	}
	lines.push(SUPPORT[caps.support]);
	return lines;
}
