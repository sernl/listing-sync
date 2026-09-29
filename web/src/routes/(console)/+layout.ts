import { api, ApiFailure, type SiteView, type Whoami } from '$lib/api';
import { describeUnreachable, type Unreachable } from '$lib/unreachable';
import { setLedgerScope } from '$lib/ledger';
import { identifyOrg, startTelemetry } from '$lib/posthog';

/** What the one request every console visit begins with came back as.
 *
 *  A session, no session, or a reason the question could not be answered.
 *  The third is a value rather than a throw on purpose: this load used to be
 *  the root layout's, and a throw from there is the one failure `+error.svelte`
 *  cannot render, so SvelteKit fell back to `error.html` reading "Internal
 *  Error (500)" — which is what the Android app showed a seller when an edge
 *  in front of this origin answered `/v1/whoami` with something other than
 *  JSON. A value still beats a throw here: the card below names the status
 *  that actually came back, and whether anything came back at all, which a
 *  generic error page cannot say.
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
	// pageview rather than a gap in the funnel. The library itself arrives
	// once the page has loaded and gone idle; see `$lib/posthog`.
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
