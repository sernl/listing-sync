// The Marketplace Migration page's rows: one card per shop the seller has
// brought across, the same set again as activity-log lines, and the per-source
// count the left column carries. Pure, so it tests without a component.
//
// Every fact here is read through `sync-request`, which already owns what a
// request's state means. This module chooses the words a card carries; it
// decides nothing about the state itself.

import type { SyncRequestHead } from '$lib/api';
import type { LogEntry } from '$lib/ActivityLog.svelte';
import { agoLabel } from '$lib/elapsed';
import type { InventoryId, Marketplace } from '$lib/generated/vocab';
import { MARKETPLACE_OF } from '$lib/listings-view';
import { platformTitle } from '$lib/platforms';
import {
	headStage,
	listRowLine,
	presentStage,
	type StageTone,
	type SyncStage
} from '$lib/sync-request';
import type { Counts } from './marketplace-list';

export type RowTone = 'ok' | 'warn' | 'bad' | 'run' | 'soon';

/** The status pill's spelling of a stage tone. `mut` is the request
 *  vocabulary's grey, which the pill calls `soon`. */
const PILL: Record<StageTone, RowTone> = { ok: 'ok', mut: 'soon', run: 'run', bad: 'bad' };

export function pillTone(tone: StageTone): RowTone {
	return PILL[tone];
}

export interface MigrationRow {
	request: string;
	href: string;
	/** Which shop went where, named in full rather than by acronym: the row
	 *  stands on its own in a list and the short name is not a sentence. */
	source: InventoryId;
	target: InventoryId;
	meta: string;
	label: string;
	tone: RowTone;
}

/** Only the migrate half of the request list. A sync request and a migration
 *  are the same record under two dispositions, and this page owns one. */
export function migrations(requests: readonly SyncRequestHead[]): SyncRequestHead[] {
	return requests.filter((row) => row.disposition === 'migrate');
}

export function migrationRows(
	requests: readonly SyncRequestHead[],
	now: number
): MigrationRow[] {
	return migrations(requests).map((row) => {
		const stage = headStage(row);
		const shown = presentStage(stage);
		return {
			request: row.request,
			href: `/sync/requests/${row.request}`,
			source: row.source,
			target: row.target,
			meta: `${listRowLine(stage)} · started ${agoLabel(row.created_at, now)}`,
			label: shown.label,
			tone: PILL[shown.tone]
		};
	});
}

/** The same migrations as log lines. The log is the page's history rather than
 *  a second list of the same cards: it says what happened and when, in one
 *  line, and the cards above it carry the state that is still live. */
export function migrationLog(requests: readonly SyncRequestHead[], now: number): LogEntry[] {
	return migrations(requests).map((row) => {
		const shown = presentStage(headStage(row));
		return {
			id: row.request,
			what: `${platformTitle(row.source)} → ${platformTitle(row.target)} · ${shown.label}`,
			at: agoLabel(row.created_at, now)
		};
	});
}

/** How many shops have come across from each marketplace, for the left
 *  column's count badge. Keyed by marketplace rather than by inventory,
 *  because a seller holds one login for Tes and not three. */
export function migrationCounts(requests: readonly SyncRequestHead[]): Counts {
	const counts: Partial<Record<Marketplace, number>> = {};
	for (const row of migrations(requests)) {
		const marketplace = MARKETPLACE_OF[row.source];
		counts[marketplace] = (counts[marketplace] ?? 0) + 1;
	}
	return counts;
}

export const NO_MIGRATION_YET = 'No migration has run yet.';


// ------------------------------------------------ starting a second migration

/** Whether a request has finished, whatever it finished as.
 *
 * An exhaustive switch rather than a list of the open kinds, so a stage added
 * to the union stops the type check here and someone decides which side it
 * falls on, instead of it defaulting to whichever the list happened to omit.
 *
 * `unrecognised` counts as still going. The choice is deliberate and it is the
 * cautious one: a request in a state this client does not know might still be
 * reading the seller's shop, and letting a second migration of the same shop
 * start is worse than making the seller open the first one to see. The banner
 * that stands in its place always links to that request, so this never leaves
 * the seller without a route. */
function settled(stage: SyncStage): boolean {
	switch (stage.kind) {
		case 'imported':
		case 'nothing_to_import':
		case 'failed':
			return true;
		case 'waiting_for_device':
		case 'queued':
		case 'importing':
		case 'unrecognised':
			return false;
	}
}

export interface OpenMigration {
	request: string;
	href: string;
	/** What is already happening, in the seller's own terms. */
	line: string;
}

/** The migration already under way from this shop, or null where there is
 *  none.
 *
 * Matched on the inventory rather than on the marketplace, so the guard is
 * stated in the same terms the request carries.
 *
 * The newest is answered where somehow more than one is open, since that is
 * the one the seller most recently meant. */
export function openFromSource(
	requests: readonly SyncRequestHead[],
	source: InventoryId | null
): OpenMigration | null {
	if (source === null) {
		return null;
	}
	const open = migrations(requests)
		.filter((row) => row.source === source && !settled(headStage(row)))
		.sort((left, right) => right.created_at - left.created_at)[0];
	if (open === undefined) {
		return null;
	}
	return {
		request: open.request,
		href: `/sync/requests/${open.request}`,
		line: `${platformTitle(open.source)} is already being migrated: ${listRowLine(headStage(open))}`
	};
}

export const ALREADY_RUNNING_TITLE = 'This shop is already being migrated';

/** What a migration does, said once and before anything is chosen.
 *
 * Carried over from the import screen when that screen stopped offering this
 * request, so the explanation a seller met the first time is not lost with the
 * door it stood behind. It says the write as well as the read, which the
 * import wording did not have to: this page is where the TPT drafts are asked
 * for. What travels and what does not is `FILES_STAY_ON_YOUR_COMPUTER`, set
 * under the card rather than repeated here. */
export const WHAT_A_MIGRATION_IS =
	'A migration reads the whole shop you already sell on and drafts every listing on TPT — ' +
	'nothing is published without you. Your own device does the reading, so it starts the ' +
	'next time that device checks in.';
