// What one marketplace offers on one axis, and which of the seller's chosen
// values is still a real one. Pure, so it tests without a component.

import type { NativeValueView, VocabularyView } from '$lib/api';
import type { TermKind } from '$lib/generated/vocab';

/** The four answers, kept apart because they are four different facts.
 *
 *  Collapsing them into an empty list is what let the screen tell a seller
 *  "this marketplace publishes no value list" while the vocabulary was still
 *  being read — a claim about the marketplace made from a read we had not
 *  done. `unread` is the state on every page load, not only on failure. */
export type Destinations =
	| { kind: 'unread' }
	| { kind: 'unbound' }
	| { kind: 'open' }
	| { kind: 'values'; values: NativeValueView[] };

/** Which values this marketplace offers on this axis.
 *
 *  An axis lands in one native field, and a closed field carries its own value
 *  list; a field with no list is one whose values this server has not
 *  captured, so the picker offers none rather than inventing them. */
export function destinationsOf(
	vocabulary: VocabularyView | undefined,
	axis: TermKind
): Destinations {
	if (vocabulary === undefined) {
		return { kind: 'unread' };
	}
	const binding = vocabulary.axes.find((one) => one.axis === axis);
	if (binding === undefined) {
		return { kind: 'unbound' };
	}
	const values = vocabulary.natives.find((native) => native.name === binding.native)?.values;
	if (values === undefined || values.length === 0) {
		return { kind: 'open' };
	}
	return { kind: 'values', values };
}

/** The chosen value, or nothing when it is not one of the values on offer.
 *
 *  The guard rather than the clearing is what makes a stale identifier
 *  unwritable. Changing the marketplace or the axis re-points the picker at a
 *  different native field, and an identifier from the previous field stays a
 *  non-empty string that would otherwise satisfy `isComplete` and be written
 *  as the destination of a field it does not belong to. Clearing on change
 *  keeps the control honest; this keeps the write correct however the state
 *  was reached. */
export function chosenOf(destinations: Destinations, toNative: string): string {
	if (destinations.kind !== 'values') {
		return '';
	}
	return destinations.values.some((value) => value.id === toNative) ? toNative : '';
}
