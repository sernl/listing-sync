// The Marketplace Migration page's rows: one card per shop the seller has
// brought across, the same set again as activity-log lines, and the per-source
// count the left column carries. Pure, so it tests without a component.
//
// Every fact here is read through `sync-request`, which already owns what a
// request's state means. This module chooses the words a card carries; it
// decides nothing about the state itself.

import type { JobDeletionStatus, SyncRequestHead } from '$lib/api';
import type { LogEntry } from '$lib/ActivityLog.svelte';
import { agoLabel } from '$lib/elapsed';
import { DISPOSITION_WORD } from '$lib/migration-plan';
import type { InventoryId } from '$lib/generated/vocab';
import { platformTitle } from '$lib/platforms';
import {
	headStage,
	listRowLine,
	presentStage,
	type StageTone
} from '$lib/sync-request';

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
	/** Whether this migration is on its way out of the history, and how far
	 *  that has got. Null for one nobody has asked to delete: a deleted
	 *  request is filtered out by the server, so only the retained arms reach
	 *  here. */
	deletion: JobDeletionStatus | null;
}

/** Copy and Move as a seller reads them, indexed by whatever string the wire
 *  carried. A `Record<string, string | undefined>` view of the console's own
 *  table rather than a cast: `disposition` is a wire string, and a value this
 *  version does not know is shown as it came rather than mislabelled as one of
 *  the two. */
const KIND_WORD: Record<string, string | undefined> = DISPOSITION_WORD;

// The rows arrive narrowed — or deliberately not narrowed — by the server:
// `/v1/sync` takes the disposition as a query filter, applied before its
// limit. This module never filters the list itself. That was correct only
// while the list was a single bounded read of everything: against a page of
// ten requests it would answer "the Moves among the newest ten", so a seller
// whose last ten transfers were Copies saw an empty history.
//
// Both dispositions are one history, because both are a transfer between two
// shops and a seller who copied on Monday and moved on Tuesday has made two
// transfers rather than one of each. Each row therefore says which it was.

export function migrationRows(
	requests: readonly SyncRequestHead[],
	now: number
): MigrationRow[] {
	return requests.map((row) => {
		const stage = headStage(row);
		const shown = presentStage(stage);
		const kind = KIND_WORD[row.disposition] ?? row.disposition;
		return {
			request: row.request,
			href: `/sync/requests/${row.request}`,
			source: row.source,
			target: row.target,
			meta: `${kind} · ${listRowLine(stage)} · started ${agoLabel(row.created_at, now)}`,
			label: shown.label,
			tone: PILL[shown.tone],
			deletion: row.deletion_status ?? null
		};
	});
}

/** The same migrations as log lines. The log is the page's history rather than
 *  a second list of the same cards: it says what happened and when, in one
 *  line, and the cards above it carry the state that is still live. */
export function migrationLog(requests: readonly SyncRequestHead[], now: number): LogEntry[] {
	return requests.map((row) => {
		const shown = presentStage(headStage(row));
		return {
			id: row.request,
			what: `${KIND_WORD[row.disposition] ?? row.disposition} · ${platformTitle(row.source)} → ${platformTitle(row.target)} · ${shown.label}`,
			at: agoLabel(row.created_at, now)
		};
	});
}

// There is no per-marketplace count here any more. It fed the left column's
// badge and was summed over the whole request list, which was one bounded
// read; the list is a page now, so the same sum would mean "migrations from
// this marketplace on the page you are looking at" while reading as a total.
// The server answers no total for this list and the console invents none.

/** The selection after the seller ticks or unticks the whole page.
 *
 * Only the rows on screen move; everything else the seller has chosen stays
 * chosen. The control used to replace the selection with the visible ids,
 * which was the same thing while the tick list was the whole catalogue and is
 * silent data loss now the list is a page: four resources chosen on page one
 * vanished the moment the seller pressed it on page two, and unticking it
 * emptied the selection outright.
 *
 * Pure and here rather than inline in the page, because it is the rule a
 * migration's contents depend on and it is worth a test of its own. */
export function pageSelection(
	ticked: ReadonlySet<string>,
	shown: readonly string[],
	untick: boolean
): Set<string> {
	const next = new Set(ticked);
	for (const id of shown) {
		if (untick) {
			next.delete(id);
		} else {
			next.add(id);
		}
	}
	return next;
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
