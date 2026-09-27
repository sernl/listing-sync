export const ssr = false;
export const prerender = false;

import { api, ApiFailure, type SiteView, type Whoami } from '$lib/api';
import { describeUnreachable, type Unreachable } from '$lib/unreachable';
import { setLedgerScope } from '$lib/ledger';
import { identifyOrg, startTelemetry } from '$lib/posthog';

/** What the one request every console visit begins with came back as.
 *
 *  A session, no session, or a reason the question could not be answered.
 *  The third is a value rather than a throw on purpose: a throw from a root
 *  layout load is the one failure `+error.svelte` cannot render, so SvelteKit
 *  falls back to `error.html` reading "Internal Error (500)" — which is what
 *  the Android app showed a seller when an edge in front of this origin
 *  answered `/v1/whoami` with something other than JSON. The status that
 *  actually came back, and whether anything came back at all, is the one thing
 *  that screen needs to say and the one thing it could not.
 *
 *  The site-wide switches are read beside it, and a failure there is `null`
 *  rather than a reason: maintenance mode and the season mark are courtesies,
 *  and neither may stop the console loading. `operator` is asked only when
 *  maintenance is on and there is a session, because that is the only time
 *  the answer changes what renders; the operator's own read of the switches is
 *  the probe, and its blank 401 is how a seller is told apart. */
export async function load(): Promise<{
	session: Whoami | null;
	unreachable: Unreachable | null;
	site: SiteView | null;
	operator: boolean;
}> {
	// Before the await, so a visit whose session call fails is still a
	// pageview rather than a gap in the funnel.
	startTelemetry();
	const siteRead = api.site().catch(() => null);
	try {
		const session = await api.whoami();
		setLedgerScope(session.org);
		identifyOrg(session.org);
		const site = await siteRead;
		const operator = site?.maintenance.on
			? await api.adminSite().then(
					() => true,
					() => false
				)
			: false;
		return { session, unreachable: null, site, operator };
	} catch (failure) {
		const site = await siteRead;
		if (failure instanceof ApiFailure && failure.status === 401 && failure.body !== null) {
			setLedgerScope(null);
			return { session: null, unreachable: null, site, operator: false };
		}
		return { session: null, unreachable: describeUnreachable(failure), site, operator: false };
	}
}
