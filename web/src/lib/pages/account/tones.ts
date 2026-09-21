// The badge's vocabulary, against the older one a shared module still speaks.
//
// `$lib/connection-status` answers with the pill modifiers the sheet carried
// before the redraw — `ok`, `run`, `bad`, `mut` — and `StatusPill` takes a
// different closed set. That module is not this slice's to change, so the
// translation lives here, once, rather than as an inline record on each page
// that renders one of its answers.
//
// It translated a processor's subscription status too, until the billing rail
// moved to Stripe and `/v1/billing` stopped serving one: the page now answers
// the plan, the cadence and the renewal date, none of which is a status word
// needing a tone.

import type { Tone } from '$lib/StatusPill.svelte';

/** The pill modifiers `$lib/connection-status` returns. */
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
