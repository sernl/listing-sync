// The marketplace status page beyond its per-marketplace rows: what a seller
// does about each state, and the recent events the page can vouch for.
//
// The events are read off the two answers the page already holds — the status
// endpoint's pauses and the device registry's sessions — rather than off a
// history the server does not serve. The page is readable signed out, so a
// signed-out reader sees the pauses alone, which is the honest subset.

import type { DeviceView, InventoryStatus } from '$lib/api';
import type { Marketplace } from '$lib/generated/vocab';
import type { MarketplaceStatusRow } from '$lib/pages/account/status-line';

/** What the seller does about one marketplace's state, in one sentence. */
export function statusAdvice(row: MarketplaceStatusRow): string {
	if (row.label === 'Coming soon') {
		return 'Nothing yet.';
	}
	if (row.tone === 'bad') {
		return 'Your changes wait here. Send us this page’s link if it stays paused.';
	}
	return 'Nothing to do.';
}

/** What each state on this page means, for the page's Explain. */
export const STATE_MEANINGS: readonly { label: string; tone: MarketplaceStatusRow['tone']; meaning: string }[] = [
	{
		label: 'Working',
		tone: 'ok',
		meaning: 'Teachouse can send to this marketplace right now.'
	},
	{
		label: 'Paused',
		tone: 'bad',
		meaning:
			'Sending to this marketplace has stopped, for the reason shown. Your queued changes wait until sending starts again.'
	},
	{
		label: 'Coming soon',
		tone: 'soon',
		meaning: 'Teachouse cannot send resources here yet.'
	}
];

export type StatusEventKind = 'paused' | 'used' | 'signed_in';

export interface StatusEvent {
	at: number;
	marketplace: Marketplace;
	kind: StatusEventKind;
	/** What happened, in the seller's words. */
	what: string;
}

/** The newest events the page can state, newest first, at most `limit`.
 *
 *  A pause is its own event where it was stamped. A session contributes when
 *  it was signed in and when it was last used, on a device the seller still
 *  holds: a signed-out machine is not evidence of anything current, which is
 *  the rule `status-line` keeps for "last used" too. */
export function statusEvents(
	entries: readonly InventoryStatus[],
	devices: readonly DeviceView[],
	limit = 8
): StatusEvent[] {
	const events: StatusEvent[] = [];
	for (const entry of entries) {
		if (entry.halted && entry.raised_at !== undefined) {
			events.push({
				at: entry.raised_at,
				marketplace: entry.marketplace,
				kind: 'paused',
				what: `Sending paused: ${entry.reason ?? 'no reason recorded'}`
			});
		}
	}
	for (const device of devices) {
		if (device.revoked_at !== null) {
			continue;
		}
		for (const session of device.sessions) {
			if (session.status !== 'connected') {
				continue;
			}
			events.push({
				at: session.linked_at,
				marketplace: session.marketplace,
				kind: 'signed_in',
				what: `Signed in on ${device.name}`
			});
			if (session.last_used_at > session.linked_at) {
				events.push({
					at: session.last_used_at,
					marketplace: session.marketplace,
					kind: 'used',
					what: `Used by the app on ${device.name}`
				});
			}
		}
	}
	return events.sort((left, right) => right.at - left.at).slice(0, limit);
}
