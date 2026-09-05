// The badge's vocabulary, against the older one two shared modules still speak.
//
// `$lib/paddle` and `$lib/connection-status` both answer with the pill
// modifiers the sheet carried before the redraw — `ok`, `run`, `bad`, `mut` —
// and `StatusPill` takes a different closed set. Neither of those modules is
// this slice's to change, so the translation lives here, once, rather than as
// an inline record on each page that renders one of their answers.

import type { Tone } from '$lib/StatusPill.svelte';
import { subscriptionTone } from '$lib/paddle';

/** The pill modifiers `$lib/paddle` and `$lib/connection-status` return. */
export type LegacyTone = 'ok' | 'run' | 'bad' | 'mut';

/** `mut` is the older set's untinted value and the badge has no untinted tone,
 *  so it takes the grey that means nothing is being reported — the same grey
 *  the specification assigns to `checking`. */
const BADGE: Record<LegacyTone, Tone> = {
	ok: 'ok',
	run: 'run',
	bad: 'bad',
	mut: 'soon'
};

export function badgeTone(legacy: LegacyTone): Tone {
	return BADGE[legacy];
}

/** How the badge renders one of Paddle's own statuses.
 *
 *  Paddle's vocabulary is open and the server passes it through, so a status
 *  neither module recognises reaches the grey rather than being tinted as
 *  something it may not be. */
export function paddleBadge(status: string): Tone {
	return badgeTone(subscriptionTone(status));
}
