// How a counted stat card on the operator surface draws itself.
//
// The decision is three-way, not two-way, and that is the whole reason this
// lives here rather than inline: written as `count > 0 ? bad : good` a figure
// that was never read falls into the `good` arm and the card paints a green
// tick over the em dash its own value slot prints. Four cards across the
// overview and the sync-health screen make the same decision, so they make it
// once.

import type { IconName } from '$lib/icons';

/** The tint a stat card carries. The empty string is `StatCard`'s own default,
 *  the accent tint, which the uncounted cards beside these already use and
 *  which asserts nothing about health. */
export type TileTone = '' | 'ok' | 'warn' | 'bad';

export interface Tile {
	icon: IconName;
	tone: TileTone;
}

/** What a tile draws where the figure behind it was never read.
 *
 *  Deliberately not `ok`: an unread count is not a healthy one, and an
 *  operator opening this page to find out whether anything is wrong must not
 *  be told that nothing is by a card that has no figure. */
export const UNREAD: Tile = { icon: 'minus', tone: '' };

/**
 * The glyph and tint for a counted tile.
 *
 * `raised` is drawn when the count is above zero and `clear` when it is
 * exactly zero; an absent count takes neither and draws `UNREAD`. Pass the
 * same expression the card's value slot reads, so the glyph, the tint and the
 * figure can never disagree about whether there is a figure.
 */
export function tileFor(count: number | undefined, raised: Tile, clear: Tile): Tile {
	if (count === undefined) {
		return UNREAD;
	}
	return count > 0 ? raised : clear;
}

/**
 * The total of several ledger figures, or nothing.
 *
 * Absent when any part is, rather than treating a missing field as zero: a sum
 * built from `undefined` is `NaN`, which `?? '—'` does not catch and which
 * `tileFor` would send to its clear arm — a green tick over the literal `NaN`.
 */
export function sumOf(...parts: readonly (number | undefined)[]): number | undefined {
	let total = 0;
	for (const part of parts) {
		if (part === undefined || Number.isNaN(part)) {
			return undefined;
		}
		total += part;
	}
	return total;
}
