// Why a picker closes.
//
// `menu-dismissal.ts` answers the same question for a menu and cannot answer it
// for a picker: a menu closes when one of its items is chosen, and a
// multi-select picker must not, because the seller is part-way through choosing
// several. That one difference is why both files exist.
//
// Escape, a press outside and focus leaving keep what is ticked rather than
// reverting it, which departs from the listbox convention where Escape
// cancels. Every tick has already reached the draft through `onChange`, so
// there is no pending buffer to revert to; reverting would mean inventing one,
// and a seller who ticks four subjects and taps away to read the next field
// would lose all four.

export type PickerEvent =
	| { kind: 'escape' }
	/** A pointer press, and whether it landed inside the picker's own box. */
	| { kind: 'press'; inside: boolean }
	/** The scrim dimming the page behind the phone sheet was clicked. A press
	 *  on it is inside, per `pressedInside`, so this is the tap completing on
	 *  the scrim rather than the press starting on it. */
	| { kind: 'scrim' }
	/** The Done control was pressed. */
	| { kind: 'done' }
	/** One of the picker's options was ticked or unticked. */
	| { kind: 'pick' }
	/** Focus moved to something outside the picker, per `focusLeft`. */
	| { kind: 'focusLeft' };

/** Whether this event closes an open picker.
 *
 * A total switch rather than a chain of `if`s, so a reason added to
 * `PickerEvent` stops the type check instead of silently leaving the picker
 * open. */
export function closes(event: PickerEvent): boolean {
	switch (event.kind) {
		case 'escape':
			return true;
		case 'scrim':
			return true;
		case 'done':
			return true;
		case 'pick':
			return false;
		case 'press':
			return !event.inside;
		case 'focusLeft':
			return true;
	}
}

/** Whatever can say whether a node lies within it: the picker's own element. */
export interface PickerBox {
	contains(node: Node | null): boolean;
}

/** Whether a press landed inside the picker's own box.
 *
 * The scrim that dims the page behind the phone sheet is drawn inside the
 * picker's element, and a press on it counts as inside on purpose. Closing on
 * the press removes the sheet before the finger lifts, and the rest of the tap
 * is hit-tested afresh against what the dimming covered: a field takes the
 * keyboard, a checkbox toggles, the form's submit button submits. So the scrim
 * closes the picker on its click, once the tap has completed on it. */
export function pressedInside(target: EventTarget | null, box: PickerBox | null): boolean {
	if (box === null || target === null) {
		return false;
	}
	return box.contains(target as Node);
}

/** Whether focus has moved to a target outside the picker's own box.
 *
 * `target` is what is receiving focus, and null is not a departure: a press
 * on the sheet's own padding, a tap on the scrim, the browser's own chrome and
 * a window that lost focus all report null, and closing on any of them would
 * shut the picker under the seller's finger or while they are in another app.
 * A departure is a target the box does not contain, which on the phone is
 * where Tab past Done lands: behind the scrim, with the sheet otherwise still
 * up. */
export function focusLeft(target: EventTarget | null, box: PickerBox | null): boolean {
	if (box === null || target === null) {
		return false;
	}
	return !box.contains(target as Node);
}

/** Whether focus goes back to the search box once the picker has closed.
 *
 * Only when it was inside the picker to begin with, so a press that lands on
 * the next field is not undone, and never where the primary pointer is coarse:
 * there, focusing a text box raises the soft keyboard, for a box the seller
 * has just said they are finished with. */
export function returnsFocus(held: boolean, coarse: boolean): boolean {
	return held && !coarse;
}

/** The Done control's words, counting what has been chosen so far. */
export function doneLabel(chosen: number): string {
	return chosen === 0 ? 'Done' : `Done, ${chosen} chosen`;
}
