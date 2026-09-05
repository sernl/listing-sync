// The one sentence the marketplace status page puts under each inventory's
// name. Assembled here rather than in the markup because the parts are
// conditional, and a conditional built from template whitespace loses its
// separators to Svelte's own trimming.

import type { InventoryStatus } from '$lib/api';
import { agoLabel } from '$lib/elapsed';

/**
 * What is worth saying about one inventory beyond its own name.
 *
 * An operating inventory carries only the marketplace it belongs to: there is
 * nothing else to report, and inventing a reassurance would put words on a
 * screen that the server never said. A halted one carries the recorded reason
 * and, where the halt was stamped, how long it has stood.
 */
export function statusLine(entry: InventoryStatus, now: number): string {
	// Empty rather than the marketplace's own name where the two coincide: the
	// row already prints the inventory above this line, and `Tpt` over `Tpt`
	// reads as a rendering fault rather than as a fact.
	const where = entry.marketplace === entry.inventory ? '' : entry.marketplace;
	if (!entry.halted) {
		return where;
	}
	const reason = entry.reason ?? 'no reason recorded';
	const since = entry.raised_at === undefined ? '' : `, since ${agoLabel(entry.raised_at, now)}`;
	return where === '' ? `${reason}${since}` : `${where} · ${reason}${since}`;
}
