// The one sentence the marketplace status page puts under each inventory's
// name. Assembled here rather than in the markup because the parts are
// conditional, and a conditional built from template whitespace loses its
// separators to Svelte's own trimming.

import type { InventoryStatus } from '$lib/api';
import { agoLabel } from '$lib/elapsed';

/**
 * What is worth saying about one inventory beyond its own name.
 *
 * An operating inventory says nothing at all: there is nothing to report, and
 * inventing a reassurance would put words on a screen that the server never
 * said. The marketplace is not named here either, because the row's heading is
 * `platformTitle`, which already carries it — printing it again put a
 * lower-cased "Tes" under a heading reading "TES (Tes.com) · United Kingdom".
 * A halted inventory carries the recorded reason and, where the halt was
 * stamped, how long it has stood.
 */
export function statusLine(entry: InventoryStatus, now: number): string {
	if (!entry.halted) {
		return '';
	}
	const reason = entry.reason ?? 'no reason recorded';
	const since = entry.raised_at === undefined ? '' : `, since ${agoLabel(entry.raised_at, now)}`;
	return `${reason}${since}`;
}
