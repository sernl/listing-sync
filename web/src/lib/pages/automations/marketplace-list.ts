// The left column every Automations page carries: one row per marketplace the
// seller is connected to, its link status, and the figure the page in question
// has for it. Pure, so it tests without a component.

import type { ConnectionView } from '$lib/api';
import { present } from '$lib/connection-status';
import type { Marketplace } from '$lib/generated/vocab';
import { MARKETPLACE_NAME } from '$lib/platforms';

export type RowTone = 'ok' | 'warn' | 'bad' | 'soon';

export interface MarketplaceRow {
	marketplace: Marketplace;
	name: string;
	/** The server's own word for the link state, rendered verbatim: the
	 *  vocabulary is generated from Rust, and a client that re-spells it puts a
	 *  second vocabulary in front of the seller. */
	status: string;
	tone: RowTone;
	/** What the page counts for this marketplace, or null where it counts
	 *  nothing. */
	count: number | null;
}

/** Where each marketplace sits in the column. A total record rather than a
 *  list, so a marketplace added to the generated union stops the type check
 *  here instead of quietly dropping out of every Automations page. */
const RANK: Record<Marketplace, number> = { Tpt: 0, Tes: 1, Etsy: 2 };

/** The status pill's spelling of a connection tone. `connection-status` answers
 *  in the older four-value vocabulary the connections page was written
 *  against, where `mut` is the grey the pill calls `soon` and `run` is the
 *  amber it calls `warn`. */
const PILL: Record<'ok' | 'mut' | 'run' | 'bad', RowTone> = {
	ok: 'ok',
	mut: 'soon',
	run: 'warn',
	bad: 'bad'
};

export type Counts = Partial<Record<Marketplace, number>>;

/** One row per marketplace the seller has a connection for, in column order.
 *
 * A marketplace with no connection is absent rather than shown empty: these
 * pages act on a marketplace, and a row that cannot be acted on is a row that
 * has to explain itself. The Marketplaces page is where a missing connection
 * is answered. */
export function marketplaceRows(
	connections: readonly ConnectionView[],
	counts: Counts = {}
): MarketplaceRow[] {
	const newest = new Map<Marketplace, ConnectionView>();
	for (const connection of connections) {
		const held = newest.get(connection.marketplace);
		if (held === undefined || connection.updated_at > held.updated_at) {
			newest.set(connection.marketplace, connection);
		}
	}
	return [...newest.values()]
		.sort((left, right) => RANK[left.marketplace] - RANK[right.marketplace])
		.map((connection) => {
			const shown = present(connection.status);
			return {
				marketplace: connection.marketplace,
				name: MARKETPLACE_NAME[connection.marketplace],
				status: shown.label,
				tone: PILL[shown.tone],
				count: counts[connection.marketplace] ?? null
			};
		});
}

/** The row the page opens on: the first one, or nothing where the seller has
 *  connected no marketplace at all. Separate from the list so a page can hold
 *  a selection across a refetch and fall back only when it has to. */
export function firstSelection(rows: readonly MarketplaceRow[]): Marketplace | null {
	return rows[0]?.marketplace ?? null;
}

/** The selection to hold after a refetch: the one the seller made, unless the
 *  marketplace it named has left the list. */
export function heldSelection(
	rows: readonly MarketplaceRow[],
	held: Marketplace | null
): Marketplace | null {
	if (held !== null && rows.some((row) => row.marketplace === held)) {
		return held;
	}
	return firstSelection(rows);
}
