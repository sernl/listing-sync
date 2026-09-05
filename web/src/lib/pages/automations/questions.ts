// The open-questions drill-down's rows: one card per term sync could not
// translate on its own. Pure, so it tests without a component.
//
// The page itself keeps every behaviour it had — resolving a term to a path,
// recording that a term has no counterpart, and the import-drain series that
// `drain` already models. This module only chooses the words a row carries.

import type { QueueItem } from '$lib/api';
import { agoLabel } from '$lib/elapsed';
import { platformTitle } from '$lib/platforms';

export interface QuestionRow {
	id: string;
	/** The term itself, which is what the seller is being asked about. */
	title: string;
	meta: string;
	/** The queue item this row was drawn from, so acting on the row does not
	 *  have to find it again by identifier. */
	item: QueueItem;
}

export function questionRows(items: readonly QueueItem[], now: number): QuestionRow[] {
	return items.map((item) => ({
		id: item.id,
		title: item.term,
		meta: `${item.kind} · ${platformTitle(item.inventory)} · raised ${agoLabel(item.raised_at, now)}`,
		item
	}));
}

/** What the header says about the queue as a whole. Three figures rather than
 *  one, because a queue that is draining and a queue that was never filled
 *  read the same from the open count alone. */
export function tallyLine(open: number, resolved: number, noCounterpart: number): string {
	return `${open} open · ${resolved} resolved · ${noCounterpart} with no counterpart`;
}

export const DRAINED_TITLE = 'The queue is drained';

export const DRAINED_BODY =
	'New questions appear only when a listing carries a term with no translation yet — and each ' +
	'one you answer stays answered.';

export const TARGET_PLACEHOLDER = 'Target path, e.g. Mathematics / Algebra';

export const TARGET_REFUSAL = 'Give the target path as segments separated by “/”.';
