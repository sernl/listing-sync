// The marketplace status page's rows. One per marketplace, because a seller
// has one login per marketplace and "is it working" is a question about the
// marketplace and not about a catalogue inside it. Assembled here rather than
// in the markup because the parts are conditional, and a conditional built
// from template whitespace loses its separators to Svelte's own trimming.

import type { DeviceView, InventoryStatus } from '$lib/api';
import { agoLabel } from '$lib/elapsed';
import type { Marketplace } from '$lib/generated/vocab';
import { INVENTORY_ORDER, MARKETPLACE_OF } from '$lib/listings-view';
import { AUTHORABLE, MARKETPLACE_NAME } from '$lib/platforms';

export type StatusTone = 'ok' | 'bad' | 'soon';

export interface MarketplaceStatusRow {
	marketplace: Marketplace;
	/** The full name, which is what the row's mark is announced as. */
	name: string;
	/** The pill's word, in the seller's vocabulary rather than the halt
	 *  table's. */
	label: string;
	tone: StatusTone;
	/** What is worth saying beyond the pill, and '' where the pill said it
	 *  all: a working marketplace has nothing to report, and inventing a
	 *  reassurance would put words on the screen the server never said. */
	why: string;
	/** When this seller's own device last worked with this marketplace, and ''
	 *  where no device has, or where the page is being read signed out. D1
	 *  puts every marketplace request on the device, so this is the only
	 *  evidence that the seller's own path to the marketplace is open. */
	checked: string;
}

/** The marketplaces in the console's own platform order, from the inventories
 *  the status endpoint serves. Deduplicated because a marketplace that ever
 *  regains a second catalogue must still be one row. */
function marketplacesIn(entries: readonly InventoryStatus[]): Marketplace[] {
	const held = new Set(entries.map((entry) => entry.marketplace));
	const ordered: Marketplace[] = [];
	for (const inventory of INVENTORY_ORDER) {
		const marketplace = MARKETPLACE_OF[inventory];
		if (held.has(marketplace) && !ordered.includes(marketplace)) {
			ordered.push(marketplace);
		}
	}
	return ordered;
}

/** When a device last used its session on this marketplace, or null.
 *
 * The newest across every device the seller still holds, and revoked devices
 * are skipped: a machine the seller signed out of is not evidence that the
 * path still works. `wiped` and `signed_out` sessions are skipped for the same
 * reason — they record that the session ended, not that it was used. */
function lastChecked(devices: readonly DeviceView[], marketplace: Marketplace): number | null {
	let newest: number | null = null;
	for (const device of devices) {
		if (device.revoked_at !== null) {
			continue;
		}
		for (const session of device.sessions) {
			if (session.marketplace !== marketplace || session.status !== 'connected') {
				continue;
			}
			if (newest === null || session.last_used_at > newest) {
				newest = session.last_used_at;
			}
		}
	}
	return newest;
}

/** Why sending is paused, in the seller's words.
 *
 * The recorded reason verbatim, because it is what an operator wrote about
 * this marketplace and paraphrasing it would lose the only specific fact on
 * the row, and how long the pause has stood where the halt was stamped. */
function haltLine(halted: readonly InventoryStatus[], now: number): string {
	const [first] = halted;
	if (first === undefined) {
		return '';
	}
	const reason = first.reason ?? 'no reason recorded';
	const since = first.raised_at === undefined ? '' : ` Paused ${agoLabel(first.raised_at, now)}.`;
	return `Sending here is paused: ${reason}.${since}`;
}

/** A marketplace this console cannot write to yet says so instead of claiming
 *  to be working: nothing is sent there, so "working" would be a claim about
 *  a path that does not exist. */
const NOT_BUILT_YET = 'Teachouse cannot send resources here yet.';

export function statusRows(
	entries: readonly InventoryStatus[],
	devices: readonly DeviceView[],
	now: number
): MarketplaceStatusRow[] {
	return marketplacesIn(entries).map((marketplace) => {
		const mine = entries.filter((entry) => entry.marketplace === marketplace);
		const halted = mine.filter((entry) => entry.halted);
		const built = mine.some((entry) => AUTHORABLE[entry.inventory]);
		const at = lastChecked(devices, marketplace);
		const checked = at === null ? '' : `Last used by your Teachouse app ${agoLabel(at, now)}`;
		if (!built) {
			return {
				marketplace,
				name: MARKETPLACE_NAME[marketplace],
				label: 'Coming soon',
				tone: 'soon' as const,
				why: NOT_BUILT_YET,
				checked: ''
			};
		}
		return {
			marketplace,
			name: MARKETPLACE_NAME[marketplace],
			label: halted.length > 0 ? 'Paused' : 'Working',
			tone: halted.length > 0 ? ('bad' as const) : ('ok' as const),
			why: haltLine(halted, now),
			checked
		};
	});
}
