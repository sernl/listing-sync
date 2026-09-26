// The Resources board's own view logic: which tab a resource falls in, which
// rows survive the filter card, how a row's one meta line reads, and the order
// the stack is shown in. Pure, so it tests without a component.
//
// The marketplace and standing semantics are `$lib/inventory`'s own
// `matchesFilters`, called once per selected marketplace rather than reimplemented
// here: the filter card offers several marketplaces where the old board offered
// one, and a second implementation of "in this state on that marketplace" is a
// second thing to keep true.

import type { ProductHead } from '$lib/api';
import type { BulkVerb } from '$lib/bulk-verbs';
import { agoLabel } from '$lib/elapsed';
import {
	ATTENTION_STATES,
	STATE_LABEL,
	matchesFilters,
	type InventoryRow,
	type MarketplaceState,
	type StandingFilter
} from '$lib/inventory';
import { formatPrice } from '$lib/listings-view';
import { AUTHORABLE, SHORT_NAME } from '$lib/platforms';
import type { Tone } from '$lib/StatusPill.svelte';
import type { ChipTone } from '$lib/inventory';
import type { InventoryId } from '$lib/generated/vocab';
import type { IconName } from '$lib/icons';

/** The status badge's tone for a chip's or a row's own tone, which are the
 *  same four words. `mut` is this board's word for nothing to act on, which
 *  the badge spells `soon`; the other three are already the badge's own. */
export const PILL_TONE: Record<ChipTone, Tone> = {
	ok: 'ok',
	run: 'run',
	bad: 'bad',
	mut: 'soon'
};

/** The five counted segments. `all` is where the board opens and is the only
 *  one that hides nothing; the next three partition the catalogue; the last
 *  deliberately overlaps all of them, which its own tooltip says.
 *
 *  `all` is a segment rather than the absence of one so that the bar always has
 *  a selected tab: with none selected the four labels read as text rather than
 *  as a control, and the state the board is actually in is named nowhere. */
export type TabId = 'all' | 'not_listed' | 'draft' | 'listed' | 'attention';

export const DEFAULT_TAB: TabId = 'all';

export interface TabDefinition {
	id: TabId;
	label: string;
	hint: string;
	icon: IconName;
}

export const RESOURCE_TABS: readonly TabDefinition[] = [
	{ id: 'all', label: 'All', hint: 'All your resources.', icon: 'layout-list' },
	{
		id: 'not_listed',
		label: 'Not listed',
		hint: 'Not on any marketplace yet.',
		icon: 'minus'
	},
	{
		id: 'draft',
		label: 'Draft',
		hint: 'On a marketplace as a draft, not shown to buyers.',
		icon: 'pencil'
	},
	{
		id: 'listed',
		label: 'Listed',
		hint: 'Shown to buyers on at least one marketplace.',
		icon: 'circle-check'
	},
	{
		id: 'attention',
		label: 'Needs you',
		hint: 'Resources that need you to act. They also appear in the other tabs.',
		icon: 'triangle-alert'
	}
];

/** A glyph for every bulk verb, so the bar and the menu read as controls
 *  rather than as a column of words. Total over `BulkVerb`, so a verb added
 *  to `$lib/bulk-verbs` is drawn rather than silently glyphless. */
export const BULK_ICON: Record<BulkVerb, IconName> = {
	cross_list: 'share-2',
	mark_listed: 'circle-check',
	move: 'arrow-right-left',
	price: 'tag',
	map_terms: 'sliders-horizontal',
	labels: 'tag',
	add_to_collection: 'layers',
	apply_template: 'layout-template',
	edit: 'pencil',
	delete: 'trash-2'
};

/** Where a resource stands, read from its chips: listed if any marketplace
 *  shows it, else draft if any holds it, else not listed. */
export function standingOfRow(row: InventoryRow): 'not_listed' | 'draft' | 'listed' {
	if (row.chips.some((chip) => chip.state === 'listed')) {
		return 'listed';
	}
	if (row.chips.some((chip) => chip.state === 'draft')) {
		return 'draft';
	}
	return 'not_listed';
}

export function needsYou(row: InventoryRow): boolean {
	return row.chips.some((chip) => ATTENTION_STATES.includes(chip.state));
}

export function inTab(row: InventoryRow, tab: TabId): boolean {
	if (tab === 'all') {
		return true;
	}
	return tab === 'attention' ? needsYou(row) : standingOfRow(row) === tab;
}

export function tabCounts(rows: readonly InventoryRow[]): Record<TabId, number> {
	const counts: Record<TabId, number> = {
		all: rows.length,
		not_listed: 0,
		draft: 0,
		listed: 0,
		attention: 0
	};
	for (const row of rows) {
		counts[standingOfRow(row)] += 1;
		if (needsYou(row)) {
			counts.attention += 1;
		}
	}
	return counts;
}

/**
 * The counts a tab bar may state, or nulls where the catalogue has not been
 * read.
 *
 * Null rather than zero, and the distinction is the whole point: zero is what a
 * seller with an empty catalogue sees, so showing it to a seller whose read
 * failed tells them something false in a figure they have no reason to
 * distrust. The nulls render as bare labels.
 */
export function countsFor(
	rows: readonly InventoryRow[],
	read: boolean
): Record<TabId, number | null> {
	if (!read) {
		return { all: null, not_listed: null, draft: null, listed: null, attention: null };
	}
	return tabCounts(rows);
}

/** The three tiles the filter card shows, in the order the specification names
 *  them, which is not the order rows draw their chips in. Etsy is offered and
 *  refused rather than hidden: it is a declared marketplace with no adapter,
 *  and a filter that silently omitted it would read as a marketplace we do not
 *  know about. */
export const ETSY_REASON = 'Etsy is coming soon. You can’t send resources to it yet';

export interface MarketplaceTile {
	inventory: InventoryId;
	label: string;
	disabled: boolean;
	reason: string | null;
}

const TILE_ORDER: readonly InventoryId[] = ['Tes', 'Tpt', 'Etsy'];

export const MARKETPLACE_TILES: readonly MarketplaceTile[] = TILE_ORDER.map((inventory) => ({
	inventory,
	label: SHORT_NAME[inventory],
	disabled: !AUTHORABLE[inventory],
	reason: AUTHORABLE[inventory] ? null : ETSY_REASON
}));

/** The standing select's options: any, the overlap, then the eight state words
 *  the chips themselves use, so the filter and the chip never name one state
 *  two ways. */
export const STANDING_OPTIONS: readonly { value: StandingFilter; label: string }[] = [
	{ value: 'all', label: 'Any status' },
	{ value: 'attention', label: 'Needs you' },
	...(
		[
			'not_listed',
			'draft',
			'listed',
			'in_flight',
			'blocked',
			'needs_signin',
			'stranded',
			'failed'
		] as MarketplaceState[]
	).map((state) => ({ value: state as StandingFilter, label: STATE_LABEL[state] }))
];

export interface ResourceFilters {
	query: string;
	/** Empty means every marketplace, which is not the same as none. */
	marketplaces: readonly InventoryId[];
	standing: StandingFilter;
	tab: TabId;
}

export const NO_RESOURCE_FILTERS: ResourceFilters = {
	query: '',
	marketplaces: [],
	standing: 'all',
	tab: DEFAULT_TAB
};

/**
 * Whether a row survives the filter card and the tab bar together.
 *
 * Several marketplaces read as "on any of these": the marketplace filter
 * narrows which chips the standing filter reads, so TPT and TES with
 * "Failed" finds rows failing on either, and never a row failing somewhere
 * else entirely.
 */
export function matchesResource(row: InventoryRow, filters: ResourceFilters): boolean {
	if (!inTab(row, filters.tab)) {
		return false;
	}
	const { query, standing } = filters;
	if (filters.marketplaces.length === 0) {
		return matchesFilters(row, { query, standing, marketplace: 'all' });
	}
	return filters.marketplaces.some((marketplace) =>
		matchesFilters(row, { query, standing, marketplace })
	);
}

/** Whether the clear control has anything to clear. The label filter is passed
 *  separately because it narrows the query the server answers rather than the
 *  rows this module reads. */
export function filtersActive(
	filters: ResourceFilters,
	labels: readonly string[] = []
): boolean {
	return (
		filters.query.trim().length > 0 ||
		filters.marketplaces.length > 0 ||
		filters.standing !== 'all' ||
		filters.tab !== DEFAULT_TAB ||
		labels.length > 0
	);
}

/** The marketplaces showing this resource, shortest name each, in chip order. */
export function listedOn(row: InventoryRow): string[] {
	return row.chips
		.filter((chip) => chip.state === 'listed')
		.map((chip) => SHORT_NAME[chip.inventory]);
}

/**
 * The row's single meta line.
 *
 * Three facts at most, because a row that lists four is read as none: how long
 * ago it changed, what it costs, and who is showing it. A resource no
 * marketplace shows says nothing rather than saying "nowhere", which the chip
 * strip directly beneath already says in full.
 */
export function metaLine(product: ProductHead, row: InventoryRow, now: number): string {
	const parts = [`Updated ${agoLabel(product.updated_at, now)}`, formatPrice(product.price)];
	const showing = listedOn(row);
	if (showing.length > 0) {
		parts.push(showing.join(', '));
	}
	return parts.join(' · ');
}

export type SortId = 'updated_desc' | 'updated_asc' | 'created_desc' | 'title_asc';

/** The orders the catalogue can actually be put in, which is exactly the four
 *  fields `ProductHead` carries. Newest first is the default because the board
 *  is read after work, not before it. */
export const SORTS: readonly { id: SortId; label: string }[] = [
	{ id: 'updated_desc', label: 'Date updated (newest)' },
	{ id: 'updated_asc', label: 'Date updated (oldest)' },
	{ id: 'created_desc', label: 'Date created (newest)' },
	{ id: 'title_asc', label: 'Title (A–Z)' }
];

const BY_TITLE = new Intl.Collator('en', { sensitivity: 'base', numeric: true });

export function sortRows(rows: readonly InventoryRow[], sort: SortId): InventoryRow[] {
	const ordered = [...rows];
	switch (sort) {
		case 'updated_desc':
			return ordered.sort((left, right) => right.product.updated_at - left.product.updated_at);
		case 'updated_asc':
			return ordered.sort((left, right) => left.product.updated_at - right.product.updated_at);
		case 'created_desc':
			return ordered.sort((left, right) => right.product.created_at - left.product.created_at);
		case 'title_asc':
			return ordered.sort((left, right) =>
				BY_TITLE.compare(left.product.title, right.product.title)
			);
	}
}

/**
 * One catalogue from several label-filtered walks of it.
 *
 * The products endpoint answers one label at a time, so several chosen labels
 * are several reads; a resource carrying two of them arrives twice and is kept
 * once, in the order the first read found it. Any of the chosen labels rather
 * than all of them, which is what a multi-select means everywhere else in this
 * console.
 */
export function unionById(pages: readonly (readonly ProductHead[])[]): ProductHead[] {
	const seen = new Set<string>();
	const products: ProductHead[] = [];
	for (const page of pages) {
		for (const product of page) {
			if (seen.has(product.id)) {
				continue;
			}
			seen.add(product.id);
			products.push(product);
		}
	}
	return products;
}

/** How each bulk verb reads inside "Select the resources to …", which the
 *  select-all bar states so the seller knows which verb the selection is for.
 *  A total map, so a verb added to `$lib/bulk-verbs` is worded here or fails
 *  the type check rather than rendering a sentence with a noun in it. */
export const VERB_PHRASE: Record<BulkVerb, string> = {
	cross_list: 'cross-list',
	mark_listed: 'mark as listed',
	labels: 'label',
	add_to_collection: 'add to a collection',
	apply_template: 'apply a template to',
	delete: 'delete',
	edit: 'edit',
	move: 'copy or move to another marketplace',
	price: 'set the price on another marketplace for',
	map_terms: 'set the licence and resource type for'
};

/**
 * The labels a URL asks the catalogue to be narrowed to.
 *
 * Repeated `label` parameters rather than one comma-joined value, because a
 * label is any word the seller chose and may hold a comma. Blank and repeated
 * names are dropped: a hand-typed URL narrows to what it can rather than
 * asking the catalogue for a label twice.
 */
export function labelsFromUrl(params: URLSearchParams): string[] {
	const seen = new Set<string>();
	for (const raw of params.getAll('label')) {
		const name = raw.trim();
		if (name.length > 0) {
			seen.add(name);
		}
	}
	return [...seen];
}

/**
 * The query string for a search and a set of labels, without its `?`.
 *
 * The two filters the server answers, and the two the URL therefore carries:
 * the Labels page links straight to a narrowed catalogue, and the shell's own
 * search box writes `q` here. Empty for no filter at all, so a cleared board
 * lands on a bare path rather than on a trailing question mark.
 */
export function filterSearch(query: string, labels: readonly string[]): string {
	const params = new URLSearchParams();
	const trimmed = query.trim();
	if (trimmed.length > 0) {
		params.set('q', trimmed);
	}
	for (const name of labels) {
		params.append('label', name);
	}
	return params.toString();
}

/**
 * The full stop a phrase needs, or the empty string where it closed itself.
 *
 * Some verdict lines end in an ellipsis and some in nothing, so a template that
 * always appends one writes "requires…." and a template that never does leaves
 * sentences running together.
 */
export function fullStop(phrase: string): string {
	return /[.…!?]$/.test(phrase.trimEnd()) ? '' : '.';
}

/** How many rows the stack draws before the seller asks for more. The whole
 *  catalogue is loaded — the tab counts and the viewing figure are counts of
 *  all of it — and this bounds only what is drawn, because a row card is a
 *  dozen elements and a catalogue of thousands would build them all at once. */
export const PAGE_STEP = 60;
