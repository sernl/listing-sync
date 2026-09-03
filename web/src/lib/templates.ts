// What the Templates screen's form holds, and how a saved override loads back
// into it. Pure, so it tests without a component: the screen owns the
// rendering and this owns the shape.

import type { OverrideView } from '$lib/api';
import type { InventoryId, TermKind } from '$lib/generated/vocab';

/** The axes a seller may override.
 *
 *  Licence is false because `ProjectionOverride::new` and the database CHECK
 *  both refuse it, and a control that always errors is worse than no control.
 *  A total map over the generated union rather than a filter, so an axis added
 *  in Rust stops this file type-checking instead of quietly becoming
 *  unofferable. */
export const OVERRIDABLE: Record<TermKind, boolean> = {
	subject: true,
	topic: true,
	resource_type: true,
	phase: true,
	licence: false
};

export const AXES: TermKind[] = (Object.keys(OVERRIDABLE) as TermKind[]).filter(
	(axis) => OVERRIDABLE[axis]
);

export const AXIS_LABEL: Record<TermKind, string> = {
	subject: 'Subject',
	topic: 'Topic',
	resource_type: 'Resource type',
	phase: 'Phase',
	licence: 'Licence'
};

/** Everything the add form holds. */
export interface OverrideDraft {
	inventory: InventoryId;
	axis: TermKind;
	from: string;
	toNative: string;
	kind: 'exact' | 'broader';
}

/** A saved override loaded back into the form.
 *
 *  Editing is the same write as adding, because the server upserts on the
 *  marketplace, axis and term together: loading a row and saving replaces it
 *  rather than adding a second. The destination comes back as the identifier
 *  rather than the words, because that is what the picker's options are keyed
 *  by and what the next save must send. */
export function draftOf(row: OverrideView): OverrideDraft {
	return {
		inventory: row.inventory,
		axis: row.axis,
		from: row.from_term,
		toNative: row.native_id ?? '',
		kind: row.kind
	};
}

/** Whether a draft names everything a save needs. The kind always has a value
 *  and the marketplace and axis are selects with no empty option, so the two
 *  that can be unset are the two checked here. */
export function isComplete(draft: OverrideDraft): boolean {
	return draft.from.length > 0 && draft.toNative.length > 0;
}
