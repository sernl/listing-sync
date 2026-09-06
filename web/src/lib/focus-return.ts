// Where focus goes when the control that had it is removed by its own action.
//
// Dismissing is the one interaction in this console that deletes the control
// the seller just used. The browser's answer to a focused element leaving the
// document is to drop focus to `<body>`, which returns a keyboard seller to the
// top of the page and costs them every tab they spent getting where they were.
//
// The precedence differs by surface, which is why this is two functions rather
// than one. A toast interrupted something, so focus returns to whatever it
// interrupted. A banner is part of the page rather than an interruption of it,
// so focus stays where the banner was rather than jumping somewhere the seller
// was before they ever reached it.

/** What is still available to receive focus, once the control that held it has
 *  gone. Both are answered by the caller, because only it can say whether an
 *  element is still in the document. */
export interface FocusOffer {
	/** The control focused before this surface appeared, still in the document. */
	previous: boolean;
	/** The region the surface sat inside, still in the document. */
	region: boolean;
}

/** `none` is a real answer rather than a failure: on the signed-out screens
 *  there is no console region to fall back to, and leaving focus where the
 *  browser put it is better than inventing a destination. */
export type FocusTarget = 'previous' | 'region' | 'none';

export function afterToastDismissed(offer: FocusOffer): FocusTarget {
	if (offer.previous) {
		return 'previous';
	}
	return offer.region ? 'region' : 'none';
}

export function afterBannerDismissed(offer: FocusOffer): FocusTarget {
	// `previous` is deliberately not consulted. A seller reaches a banner by
	// tabbing through the page it belongs to, so the place they were before is
	// behind them, and sending focus back there would undo the tabbing that got
	// them to the control they just used.
	return offer.region ? 'region' : 'none';
}

/** Which toasts need their interrupted control recorded, and which recordings
 *  have outlived the toast they belong to.
 *
 * One slot for the whole stack was wrong in one specific way: a second toast
 * arriving while the first still stood found the slot occupied and left it, so
 * closing the second returned focus to whatever the *first* had interrupted.
 * Keying by toast id is what makes the two answers separate, and this is the
 * part of that which can be wrong, so it is a function rather than a condition
 * inside an effect.
 *
 * `add` names the ids that have never been asked, not the ids whose answer was
 * empty: a toast raised while focus was already inside the stack has no control
 * to return to, and asking again later would answer with whichever control had
 * since taken focus. The caller therefore records the empty answer too. */
export function captureSlots(
	live: readonly number[],
	held: readonly number[]
): { add: number[]; drop: number[] } {
	const standing = new Set(live);
	const recorded = new Set(held);
	return {
		add: live.filter((id) => !recorded.has(id)),
		drop: held.filter((id) => !standing.has(id))
	};
}

/** The element a programmatic focus move can land on, which is narrower than
 *  `HTMLElement` on purpose: these three members are the whole of what the move
 *  needs, so the sequence tests against a stub in the node lane rather than
 *  needing a DOM. */
export interface FocusableRegion {
	hasAttribute(name: string): boolean;
	setAttribute(name: string, value: string): void;
	focus(): void;
}

/** Make a region a legal destination for a programmatic focus move, then move
 *  focus there.
 *
 * `-1` keeps the region out of the tab sequence while making it focusable, and
 * it is set only when the region carries no `tabindex` of its own: a page that
 * deliberately made its region tabbable with `0` would be taken back out of the
 * tab order by an unconditional stamp. */
export function focusRegion(region: FocusableRegion): void {
	if (!region.hasAttribute('tabindex')) {
		region.setAttribute('tabindex', '-1');
	}
	region.focus();
}
