export const ssr = false;
export const prerender = false;

import { api, ApiFailure, type Whoami } from '$lib/api';
import { describeUnreachable, type Unreachable } from '$lib/unreachable';
import { setLedgerScope } from '$lib/ledger';

/** What the one request every console visit begins with came back as.
 *
 *  A session, no session, or a reason the question could not be answered.
 *  The third is a value rather than a throw on purpose: a throw from a root
 *  layout load is the one failure `+error.svelte` cannot render, so SvelteKit
 *  falls back to `error.html` reading "Internal Error (500)" — which is what
 *  the Android app showed a seller when an edge in front of this origin
 *  answered `/v1/whoami` with something other than JSON. The status that
 *  actually came back, and whether anything came back at all, is the one thing
 *  that screen needs to say and the one thing it could not. */
export async function load(): Promise<{ session: Whoami | null; unreachable: Unreachable | null }> {
	try {
		const session = await api.whoami();
		setLedgerScope(session.org);
		return { session, unreachable: null };
	} catch (failure) {
		if (failure instanceof ApiFailure && failure.status === 401 && failure.body !== null) {
			setLedgerScope(null);
			return { session: null, unreachable: null };
		}
		return { session: null, unreachable: describeUnreachable(failure) };
	}
}
