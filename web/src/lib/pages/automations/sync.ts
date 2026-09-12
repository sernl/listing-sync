// The Marketplace Sync page's model: one pull card per marketplace, the
// cadences a plan reaches, the rows a run is drawn as, and the activity log's
// entries. Pure, so it tests without a component.

import type {
	ActivityLine,
	ConnectionView,
	JobHead,
	MarketplaceSyncSettingView
} from '$lib/api';
import type { LogEntry } from '$lib/ActivityLog.svelte';
import { standingMarketplaces } from '$lib/connection-standing';
import { agoLabel } from '$lib/elapsed';
import type { InventoryId, Marketplace } from '$lib/generated/vocab';
import { INVENTORY_ORDER, MARKETPLACE_OF } from '$lib/listings-view';
import { MARKETPLACE_WORD } from '$lib/platforms';

export interface RunRow {
	job: string;
	href: string;
	/** Where the run was sent, drawn by the marketplace's own mark, which is
	 *  what tells one run from another at a glance; the identifier follows it
	 *  in the meta line. */
	inventory: InventoryId;
	meta: string;
	/** The instant the run was created, so the template can render the absolute
	 *  time in the seller's own locale without this module carrying a
	 *  locale-dependent string. */
	at: number;
}

export function runRows(jobs: readonly JobHead[], now: number): RunRow[] {
	return jobs.map((job) => ({
		job: job.job,
		href: `/sync/${job.job}`,
		inventory: job.inventory,
		meta: `Run ${job.job.slice(0, 8)}… · started ${agoLabel(job.created_at, now)}`,
		at: job.created_at
	}));
}

/** The header control's own words. A figure this page could not read is not a
 *  figure of zero, so the control drops the count rather than claiming one. */
export function openQuestionsLabel(open: number | null): string {
	return open === null ? 'Open questions' : `Open questions (${open})`;
}

export const NO_RUN_YET =
	'Start one from Resources: choose what to send, and we take it from there.';

// ------------------------------------------------------------------ cadence

/** The three cadences a seller chooses between, as §5 names them. The figures
 *  are seconds because that is what the setting stores and what the plan's
 *  floor is stated in. */
export const CADENCES: readonly { secs: number; label: string }[] = [
	{ secs: 21_600, label: 'Every 6 hours' },
	{ secs: 86_400, label: 'Daily' },
	{ secs: 604_800, label: 'Weekly' }
];

export interface CadenceOption {
	secs: number;
	label: string;
	enabled: boolean;
	/** Why this cadence cannot be chosen, or null where it can. A disabled
	 *  option with no stated reason reads as a fault. */
	reason: string | null;
}

/** A duration as a sentence names it, so a floor can be quoted in the words
 *  the options are written in rather than in seconds. */
export function intervalPhrase(secs: number): string {
	const known = CADENCES.find((cadence) => cadence.secs === secs);
	if (known !== undefined) {
		return known.label.toLowerCase();
	}
	const hours = Math.round(secs / 3_600);
	return hours === 1 ? 'every hour' : `every ${hours} hours`;
}

/** The cadences offered, with the ones under the plan's floor disabled and
 *  the floor named.
 *
 * Disabled rather than absent: a seller on the free floor asking why they
 * cannot check hourly is owed the figure, and an option that simply is not
 * there answers nothing.
 *
 * A null floor is "no floor to apply here" and leaves every cadence standing,
 * because it is what both a plan-unread entitlement and a marketplace with no
 * stored row look like. A plan that pulls at no cadence at all is the page's
 * own gate, in `featureReason`'s words, and refusing the options as well
 * would state the same thing twice in two different sentences. */
export function cadenceOptions(minimumSecs: number | null): CadenceOption[] {
	if (minimumSecs === null) {
		return CADENCES.map((cadence) => ({ ...cadence, enabled: true, reason: null }));
	}
	const floor = `Your plan checks no more often than ${intervalPhrase(minimumSecs)}.`;
	return CADENCES.map((cadence) => ({
		...cadence,
		enabled: cadence.secs >= minimumSecs,
		reason: cadence.secs >= minimumSecs ? null : floor
	}));
}

/** The cadence a card shows, which is the stored one unless the plan has
 *  since moved above it.
 *
 * A stored interval under a floor is what a downgrade leaves behind, and a
 * select whose value matches only a disabled option shows the seller a
 * setting they cannot save. The fallback is the first cadence the plan
 * actually reaches, which is also what a card with no stored row opens on. */
export function heldCadence(intervalSecs: number, minimumSecs: number | null): number {
	const offered = cadenceOptions(minimumSecs);
	const exact = offered.find((option) => option.secs === intervalSecs && option.enabled);
	if (exact !== undefined) {
		return exact.secs;
	}
	return offered.find((option) => option.enabled)?.secs ?? CADENCES[CADENCES.length - 1].secs;
}

// -------------------------------------------------------------- pull cards

/** One marketplace's pull card.
 *
 * A marketplace the seller has not connected still gets a card, because
 * "connect it first" is the answer they came for and a missing card answers
 * nothing. A marketplace that cannot be pulled from at all does not: Etsy is
 * reached by an official API rather than by the seller's own machine, so
 * there is no shop read to pace and a card offering one would promise a
 * cadence nothing keeps. */
export interface SyncCard {
	inventory: InventoryId;
	marketplace: Marketplace;
	/** The one word the page calls this marketplace inside a sentence. */
	name: string;
	connected: boolean;
	setting: MarketplaceSyncSettingView;
}

/** Why this card cannot be switched on, or null where it can.
 *
 * The server's own refusal, in the server's words: a seller who enables it
 * anyway meets the same sentence from the 422, and two spellings of one
 * refusal read as two different problems. */
export const CONNECT_FIRST =
	'Connect this marketplace on the Marketplaces page first, then switch sync on for it.';

/** One card per marketplace the server serves a row for, in the order the
 *  console shows marketplaces in.
 *
 * The served list is the set, not `INVENTORY_ORDER`: the route answers one
 * row per seller-device-transport marketplace, padded for the ones with
 * nothing stored, so which marketplaces can be pulled from is the server's
 * answer rather than a second list kept here. */
export function syncCards(
	connections: readonly ConnectionView[],
	settings: readonly MarketplaceSyncSettingView[]
): SyncCard[] {
	const standing = standingMarketplaces(connections);
	return [...settings]
		.sort(
			(left, right) =>
				INVENTORY_ORDER.indexOf(left.inventory) - INVENTORY_ORDER.indexOf(right.inventory)
		)
		.map((setting) => ({
			inventory: setting.inventory,
			marketplace: MARKETPLACE_OF[setting.inventory],
			name: MARKETPLACE_WORD[MARKETPLACE_OF[setting.inventory]],
			connected: standing.has(MARKETPLACE_OF[setting.inventory]),
			setting
		}));
}

/** When this marketplace was last read. Never-pulled says so rather than
 *  leaving a blank, which would read as a failed read. */
export function lastPullLine(at: number | null, now: number): string {
	return at === null ? 'Never pulled' : agoLabel(at, now);
}

/** What a newly pulled resource is published with. Templates carry the
 *  marketplace-specific words and are the next release's work, so the rule
 *  says what it does today rather than what it will do. */
export const PUBLISHED_WITH_CATALOGUE_WORDS =
	'A new pull is published with your catalogue’s own title and description. Per-marketplace ' +
	'wording from a template arrives in the next release.';

// ------------------------------------------------------- log and multi-list

/** The server's own sentences, in the shape the log component renders.
 *
 * The line arrives written: it joins a run, a job and a resource's title, and
 * a client that composed it would be a second place the seller's words are
 * decided. The key is the instant and the position, because two lines can
 * share an instant and neither carries an identifier. */
export function activityEntries(lines: readonly ActivityLine[], now: number): LogEntry[] {
	return lines.map((line, index) => ({
		id: `${line.at}-${index}`,
		what: line.line,
		at: agoLabel(line.at, now),
		href: line.href ?? undefined
	}));
}

export const NO_ACTIVITY_YET =
	'Nothing has been pulled or published yet, so there is nothing to log.';

export const NOTHING_MULTI_LISTED =
	'Nothing of yours is on more than one marketplace yet. Send a resource to a second one and ' +
	'it appears here.';
