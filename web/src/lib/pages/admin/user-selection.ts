// Which accounts the operator has ticked on the users page. Pure, so it
// tests without a component.
//
// Two shapes, because "these twelve" and "everyone this search matches" are
// different promises. The first is a set of ids the operator saw. The second
// names the search rather than the ids, since most of the matching accounts
// are on pages nobody has loaded; the bulk action reads them when it runs.
// Unticking a row under the second shape records an exception rather than
// falling back to the first, so "all 240 but these two" stays one choice.

export type Selection =
	| { kind: 'ids'; ids: ReadonlySet<string> }
	| {
			kind: 'matching';
			/** The applied search the selection was made under. */
			search: string;
			/** How many accounts that search matched when it was made. */
			total: number;
			/** When it was made, in epoch milliseconds. */
			at: number;
			/** Rows unticked since. */
			except: ReadonlySet<string>;
	  };

export const NOTHING: Selection = { kind: 'ids', ids: new Set() };

export function isSelected(selection: Selection, id: string): boolean {
	return selection.kind === 'ids' ? selection.ids.has(id) : !selection.except.has(id);
}

/** How many accounts the bulk bar would act on. */
export function selectedCount(selection: Selection): number {
	return selection.kind === 'ids'
		? selection.ids.size
		: Math.max(0, selection.total - selection.except.size);
}

/** Tick or untick one row. */
export function toggle(selection: Selection, id: string): Selection {
	const flipped = new Set(selection.kind === 'ids' ? selection.ids : selection.except);
	if (!flipped.delete(id)) flipped.add(id);
	if (selection.kind === 'ids') {
		return { kind: 'ids', ids: flipped };
	}
	const except = flipped;
	// Every matching account unticked one by one is nothing selected, and
	// saying "0 selected" under a "matching" banner would read as a bug.
	return except.size >= selection.total ? NOTHING : { ...selection, except };
}

export type PageTick = 'none' | 'some' | 'all';

/** What the header checkbox shows for the rows on screen. */
export function pageTick(selection: Selection, pageIds: readonly string[]): PageTick {
	const ticked = pageIds.filter((id) => isSelected(selection, id)).length;
	if (ticked === 0 || pageIds.length === 0) return 'none';
	return ticked === pageIds.length ? 'all' : 'some';
}

/**
 * The header checkbox: tick every row on screen, or untick them all when
 * they already are. Ticking keeps rows ticked on other pages, so an operator
 * can gather accounts across pages.
 */
export function togglePage(selection: Selection, pageIds: readonly string[]): Selection {
	const allTicked = pageTick(selection, pageIds) === 'all';
	if (selection.kind === 'ids') {
		const ids = new Set(selection.ids);
		for (const id of pageIds) {
			if (allTicked) ids.delete(id);
			else ids.add(id);
		}
		return { kind: 'ids', ids };
	}
	const except = new Set(selection.except);
	for (const id of pageIds) {
		if (allTicked) except.add(id);
		else except.delete(id);
	}
	return except.size >= selection.total ? NOTHING : { ...selection, except };
}

/** Everyone the applied search matches, on every page, as of `at`. */
export function selectMatching(search: string, total: number, at: number): Selection {
	return total <= 0 ? NOTHING : { kind: 'matching', search, total, at, except: new Set() };
}

/**
 * The selection that survives a new search. Ids the operator ticked are
 * kept, since they name people; "everyone matching" named the old search, so
 * it does not carry over to a different one.
 */
export function afterSearch(selection: Selection, search: string): Selection {
	return selection.kind === 'matching' && selection.search !== search ? NOTHING : selection;
}

/**
 * The accounts a bulk action runs on, given every account the search
 * matches (read only when the selection is `matching`). An account made
 * after the selection was taken is left out: the confirm dialog named a
 * count, and the action must not quietly reach someone the operator never
 * had in front of them.
 */
export function targets<T extends { id: string; createdAt?: string | Date | null }>(
	selection: Selection,
	matching: readonly T[]
): T[] {
	if (selection.kind === 'ids') {
		return matching.filter((row) => selection.ids.has(row.id));
	}
	return matching.filter(
		(row) =>
			!selection.except.has(row.id) &&
			!(row.createdAt != null && new Date(row.createdAt).getTime() > selection.at)
	);
}
