// The verbs the inventory board offers over a selection, and what each one is
// waiting on where it is not built. Pure, so it tests without a component.
//
// The set is founder decision Q3 — edit, cross-list, labels, delete and
// mark-as-listed — with `move`, which hands a tick list to the Migrations page
// so a selection made on this board can be copied or moved to another
// marketplace without being re-picked there. Delist-and-relist is refused
// under any name, because it discards the reviews, ratings, sales history and
// URL a teaching resource accumulates; the reconciliation verb is
// revise-in-place (`docs/notes/design/seller-dashboard.md`).

export type BulkVerb =
	| 'cross_list'
	| 'move'
	| 'edit'
	| 'apply_template'
	| 'add_to_collection'
	| 'labels'
	| 'delete'
	| 'mark_listed';

export interface BulkAction {
	verb: BulkVerb;
	label: string;
	/** What the console is missing before this verb can run, or null where it
	 *  is built. Rendered in the control's tooltip and again in the footnote
	 *  under the table, because a disabled control with no stated reason reads
	 *  as a fault. */
	missing: string | null;
}

/** A total map over the verb set, so a verb added above is either built or
 *  carries the reason it is not, rather than rendering as a control that does
 *  nothing. */
const MISSING: Record<BulkVerb, string | null> = {
	cross_list: null,
	delete: null,
	// Still missing, and `apply_template` is not it: a template names the
	// fields it carries, so applying one chooses nothing across the selection.
	// The screen that lets a seller say "set the price on these forty to £3" is
	// what does not exist.
	edit: 'No screen chooses which fields change across a selection. Editing one resource at a time works; the screen for editing several at once is what does not exist.',
	apply_template: null,
	add_to_collection: null,
	labels: null,
	mark_listed: null,
	move: null
};

const LABEL: Record<BulkVerb, string> = {
	cross_list: 'Cross-list',
	edit: 'Edit',
	apply_template: 'Apply a template',
	add_to_collection: 'Add to collection',
	labels: 'Labels',
	delete: 'Delete',
	mark_listed: 'Mark as listed',
	move: 'Copy or move'
};

/** The order the board shows them in: what is built first, then what is
 *  waiting, with the destructive verb last in its group. */
export const BULK_VERBS: readonly BulkVerb[] = [
	'cross_list',
	'mark_listed',
	'move',
	'labels',
	'add_to_collection',
	'apply_template',
	'delete',
	'edit'
];

export const BULK_ACTIONS: readonly BulkAction[] = BULK_VERBS.map((verb) => ({
	verb,
	label: LABEL[verb],
	missing: MISSING[verb]
}));

/** The verbs that cannot run, for the footnote that says why once rather than
 *  once per hover. */
export function unavailable(actions: readonly BulkAction[] = BULK_ACTIONS): BulkAction[] {
	return actions.filter((action) => action.missing !== null);
}
