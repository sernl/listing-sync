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
