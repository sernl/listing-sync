/** What the operator's guides screens show, worked out apart from the
 *  screens: which guides a filter and a search keep, in what order, and the
 *  words and tones each state is drawn with. The save pipeline itself lives in
 *  `$lib/pages/guides/save`; this module only reads it. */

import type { GuideDetailView, GuideHeadView, GuideStatus, GuideTaxon } from '$lib/api';
import type { Tone } from '$lib/StatusPill.svelte';
import { sameEdit, type GuideEdit } from '$lib/pages/guides/editor';
import type { Halt, Pipeline, WriteKind } from '$lib/pages/guides/save';

// --- the list -------------------------------------------------------------

export type StatusFilter = 'all' | GuideStatus;

export const STATUS_FILTERS: ReadonlyArray<{
	id: StatusFilter;
	label: string;
}> = [
	{ id: 'all', label: 'All' },
	{ id: 'published', label: 'Published' },
	{ id: 'draft', label: 'Drafts' }
];

export type GuideOrder = 'updated' | 'title';

export const GUIDE_ORDERS: ReadonlyArray<{ id: GuideOrder; label: string }> = [
	{ id: 'updated', label: 'Recently changed' },
	{ id: 'title', label: 'Title A–Z' }
];

/** How many guides each filter chip holds. Counted over every guide, not
 *  over the search, so a chip's number does not jump while somebody types. */
export function statusCounts(rows: readonly GuideHeadView[]): Record<StatusFilter, number> {
	let published = 0;
	for (const row of rows) {
		if (row.status === 'published') {
			published += 1;
		}
	}
	return { all: rows.length, published, draft: rows.length - published };
}

/** Whether a guide answers a search. The search is split on whitespace and
 *  every word must appear somewhere in the title, the address, the topic or a
 *  tag, so "pricing etsy" narrows rather than widens. Case is ignored. */
export function matchesSearch(guide: GuideHeadView, query: string): boolean {
	const wanted = query.toLowerCase().split(/\s+/).filter(Boolean);
	if (wanted.length === 0) {
		return true;
	}
	const hay = [
		guide.title,
		guide.slug,
		guide.topic?.name ?? '',
		...guide.tags.map((tag: GuideTaxon) => tag.name)
	]
		.join('\n')
		.toLowerCase();
	return wanted.every((word) => hay.includes(word));
}

/** The guides a status chip and a search keep, in the order asked for.
 *  "Recently changed" puts the newest first and breaks ties by title; "Title"
 *  is alphabetical regardless of case. The input is left untouched. */
export function visibleGuides(
	rows: readonly GuideHeadView[],
	status: StatusFilter,
	query: string,
	order: GuideOrder
): GuideHeadView[] {
	const byTitle = (a: GuideHeadView, b: GuideHeadView) =>
		a.title.localeCompare(b.title, undefined, { sensitivity: 'base' }) ||
		a.slug.localeCompare(b.slug);
	return rows
		.filter((guide) => status === 'all' || guide.status === status)
		.filter((guide) => matchesSearch(guide, query))
		.sort(order === 'title' ? byTitle : (a, b) => b.updated_at - a.updated_at || byTitle(a, b));
}

/** What an empty grid says, which depends on why it is empty. */
export function emptyGridMessage(status: StatusFilter, query: string): string {
	if (query.trim().length > 0) {
		return 'No guides match that search.';
	}
	if (status === 'published') {
		return 'Nothing is published yet.';
	}
	if (status === 'draft') {
		return 'No drafts. Every guide is published.';
	}
	return 'No guides yet.';
}

export function statusChip(status: GuideStatus): { tone: Tone; label: string } {
	return status === 'published'
		? { tone: 'ok', label: 'Published' }
		: { tone: 'soon', label: 'Draft' };
}

// --- the editor -----------------------------------------------------------

/** One guide's or one snapshot's content as an edit, so every place that
 *  compares them reads the shape the same way. */
export function editOf(view: {
	title: string;
	body: string;
	topic: GuideTaxon | null;
	tags: GuideTaxon[];
}): GuideEdit {
	return {
		title: view.title,
		body: view.body,
		topic_id: view.topic?.id ?? null,
		tag_ids: view.tags.map((tag) => tag.id)
	};
}

/** Whether sellers are reading an older version than the saved draft.
 *  Compared by content rather than by revision: publishing moves the revision
 *  on, so a revision test would also flag a guide published a second ago with
 *  nothing changed since. */
export function publishedBehind(
	view: Pick<GuideDetailView, 'status' | 'published'> & {
		title: string;
		body: string;
		topic: GuideTaxon | null;
		tags: GuideTaxon[];
	}
): boolean {
	if (view.status !== 'published' || view.published === null) {
		return false;
	}
	return !sameEdit(editOf(view), editOf(view.published));
}

export type PublishPhase = 'draft' | 'live' | 'behind';

export function publishPhase(status: GuideStatus, behind: boolean): PublishPhase {
	if (status !== 'published') {
		return 'draft';
	}
	return behind ? 'behind' : 'live';
}

/** The pill and the one sentence for where publication stands. */
export function publishWords(phase: PublishPhase): {
	tone: Tone;
	label: string;
	line: string;
} {
	switch (phase) {
		case 'live':
			return {
				tone: 'ok',
				label: 'Published',
				line: 'Sellers see this version.'
			};
		case 'behind':
			return {
				tone: 'warn',
				label: 'Published, older version',
				line: 'Sellers still see an older version. Publish again to update it.'
			};
		case 'draft':
			return {
				tone: 'soon',
				label: 'Draft',
				line: 'Only operators can read this draft.'
			};
	}
}

/** The save pill: what has become of the text in the box. */
export function saveWords(state: { halt: Halt | null; busy: boolean; unsaved: boolean }): {
	tone: Tone;
	label: string;
} {
	if (state.halt !== null) {
		return { tone: 'bad', label: 'Not saved' };
	}
	if (state.busy) {
		return { tone: 'run', label: 'Saving' };
	}
	if (state.unsaved) {
		return { tone: 'warn', label: 'Unsaved changes' };
	}
	return { tone: 'ok', label: 'Saved' };
}

/** What the buttons need to know to decide whether they can run. */
export interface WriteGate {
	/** Why the text cannot be stored at all, or null where it can. */
	refusal: string | null;
	pipeline: Pipeline;
	unsaved: boolean;
}

/** Why a write cannot be pressed now, or `undefined` where it can. A save
 *  needs something new and storable; a publication or an unpublication works
 *  from the stored copy, so it waits for the text to be saved first. Either
 *  waits for a write already out and for a halt to be resolved. */
export function writeBlock(kind: WriteKind, gate: WriteGate): string | undefined {
	if (kind === 'save' && gate.refusal !== null) {
		return gate.refusal;
	}
	if (gate.pipeline.halt !== null) {
		return 'Resolve the message above first.';
	}
	if (gate.pipeline.flight !== null) {
		return 'Wait for the current change to finish.';
	}
	if (kind === 'save') {
		return gate.unsaved ? undefined : 'No changes to save.';
	}
	if (gate.unsaved) {
		return kind === 'publish'
			? 'Save first. Publishing uses the saved version.'
			: 'Save first, then unpublish.';
	}
	return undefined;
}

/** The label a write's button carries, with its in-flight form. */
export function writeLabel(kind: WriteKind, phase: PublishPhase, flight: WriteKind | null): string {
	const going = flight === kind;
	switch (kind) {
		case 'save':
			return going ? 'Saving…' : 'Save';
		case 'unpublish':
			return going ? 'Unpublishing…' : 'Unpublish';
		case 'publish':
			if (going) {
				return 'Publishing…';
			}
			return phase === 'draft' ? 'Publish' : 'Publish this version';
	}
}
