// Why a menu closes.
//
// A menu in this console closes for exactly three reasons, and they are named
// here rather than left inline in the two components that open one, because the
// third was missing: a caller renders the menu's items, so only the caller knows
// when one has been chosen, and until this existed it had no way to say so. The
// Labels board found it — choosing an item left the menu standing over the thing
// it had just acted on.

export type MenuEvent =
	| { kind: 'escape' }
	/** A pointer press, and whether it landed inside the menu's own box. */
	| { kind: 'press'; inside: boolean }
	/** One of the menu's items was chosen. */
	| { kind: 'choice' };

/** Whether this event closes an open menu.
 *
 * A total switch rather than a chain of `if`s, so a fourth reason added to
 * `MenuEvent` stops the type check instead of silently leaving the menu open. */
export function closes(event: MenuEvent): boolean {
	switch (event.kind) {
		case 'escape':
			return true;
		case 'choice':
			return true;
		case 'press':
			// A press inside the menu is the caller's to interpret: it may be a
			// choice, and a choice says so itself.
			return !event.inside;
	}
}
