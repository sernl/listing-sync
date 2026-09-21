// The one read of `/v1/entitlement` the console makes, and the three states
// a page reads it in.
//
// The options are defined once and shared rather than repeated at each call
// site, because a second copy with a different `staleTime` is a second read
// per navigation: this answer changes when a purchase lands or an operator
// grants something, neither of which happens while the seller walks between
// pages. The shell asks on mount beside the operator probe, and every other
// page is served from the cache.
//
// It is also the reason the console's gating is courtesy and not a fence: the
// figures here are a snapshot, the request path recomputes them per request,
// and only the latter refuses anything.

import { api, type EntitlementView } from '$lib/api';
import {
	featureReason,
	limitReason,
	type Feature,
	type Limit
} from '$lib/entitlement';
import type { EntitlementRead } from '$lib/pages/account/plans';
import { queryKeys } from '$lib/query';
import { capture } from '$lib/posthog';

export const entitlementRead = {
	queryKey: queryKeys.entitlement,
	queryFn: () => api.entitlement(),
	staleTime: Infinity,
	gcTime: Infinity
};

/** The read's three states, from what the query holds.
 *
 * A pending read and a failed read are both `unread`: neither says which
 * plan the seller is on, and a page that treated a failure as "free" would
 * disable a paying seller's controls and tell them to upgrade. */
export function readOf(query: {
	isSuccess: boolean;
	data: EntitlementView | undefined;
}): EntitlementRead {
	return query.isSuccess && query.data !== undefined
		? { state: 'read', entitlement: query.data }
		: { state: 'unread' };
}

/** Why a counted allowance is full, or null — including null for a read that
 *  has not answered.
 *
 *  Named rather than inlined at each of the seven pages that gate a control,
 *  because the unread case has to behave the same way at all of them: a page
 *  that treated a pending read as "full" would disable a seller's own
 *  controls for as long as the request took, and one that treated a failed
 *  read as "full" would do it permanently. */
export function limitOf(
	held: EntitlementView | undefined,
	limit: Limit
): string | null {
	if (held === undefined) {
		return null;
	}
	const reason = limitReason(held.capabilities, held.usage, limit);
	if (reason !== null) {
		// The wire spells a limit's used figure `<limit>` on the usage and
		// its ceiling `<limit>_max` on the capabilities, so both are read by
		// that rule rather than through a second copy of the mapping.
		const usage = held.usage as unknown as Record<string, number>;
		const caps = held.capabilities as unknown as Record<string, number>;
		gateHit(held, limit, usage[limit] ?? null, caps[`${limit}_max`] ?? null);
	}
	return reason;
}

/** Why a control's capability is not on this plan, or null — including null
 *  for a read that has not answered, for the same reason as above. */
export function featureOf(
	held: EntitlementView | undefined,
	feature: Feature
): string | null {
	if (held === undefined) {
		return null;
	}
	const reason = featureReason(held.capabilities, feature);
	if (reason !== null) {
		gateHit(held, feature, null, null);
	}
	return reason;
}

// Which gates this page load has already reported.
//
// Both functions above are called from `$derived` expressions, which re-run
// on every render of every page that gates a control: without this the one
// seller meeting one paywall would be a thousand events. The interesting
// fact is that they met it at all, which is one row.
const reported = new Set<string>();

function gateHit(
	held: EntitlementView,
	gate: string,
	used: number | null,
	max: number | null
): void {
	const route = typeof window === 'undefined' ? '' : window.location.pathname;
	const key = `${gate}|${route}`;
	if (reported.has(key)) {
		return;
	}
	reported.add(key);
	capture('plan_gate_hit', { gate, plan: held.plan, used, max, route });
}
