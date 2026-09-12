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
	type StageTone
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

/** What a migration does, said once and before anything is chosen.
 *
 * Carried over from the import screen when that screen stopped offering this
 * request, so the explanation a seller met the first time is not lost with the
 * door it stood behind. It names no marketplace: the seller chooses the pair
 * on this page now, and a sentence that said TPT stood above a To select that
 * might say something else. What travels and what does not is
 * `FILES_STAY_ON_YOUR_COMPUTER`, set under the card rather than repeated
 * here. */
export const WHAT_A_MIGRATION_IS =
	'A migration takes resources you already sell on one marketplace and drafts them on ' +
	'another — nothing is published without you. Your own device does the sending, so it ' +
	'starts the next time that device checks in.';
