// The console's half of product analytics: one init, one identity, and the
// named events a server cannot see.
//
// Three properties hold this safe, and each is a line below rather than a
// habit:
//
//   - Same origin. `api_host: '/ingest'` sends every request to our own
//     Caddy, which forwards it. The console's CSP is `connect-src 'self'`
//     and stays that way; nothing here asks for a grant.
//   - No person. The distinct id is the organisation's UUID, set once on the
//     load that resolves the session. No address is captured, and the
//     denylist names the properties PostHog would otherwise collect for us.
//   - Nothing a seller wrote. `autocapture` is off, because autocaptured
//     click text on a catalogue page is resource titles; replay masks every
//     input and every text node, because the same page shows prices,
//     descriptions and covers.
//
// A build with no key initialises nothing and loads nothing, which is the
// same dormant shape the rest of this client's public configuration takes.

import posthog from 'posthog-js';

/** Substituted by Vite at build time; absent from every build that does not
 *  define it. */
const KEY: string | undefined = import.meta.env.PUBLIC_POSTHOG_KEY;

/** Where the browser talks to. Relative on purpose: the proxy is ours, and a
 *  relative host is what keeps the request same-origin under every one of
 *  the console's three shells — browser, desktop webview and Android. */
const API_HOST = '/ingest';

/** Where the toolbar and the replay player live. Required whenever `api_host`
 *  is a proxy, or both break. */
const UI_HOST = 'https://eu.posthog.com';

let started = false;

/** Starts capture, once. Safe to call from a load that runs on every
 *  navigation, and a no-op in a build with no key. */
export function startTelemetry(): void {
	if (started || !KEY) {
		return;
	}
	started = true;
	posthog.init(KEY, {
		api_host: API_HOST,
		ui_host: UI_HOST,
		// Identified events are the priced ones, and an anonymous visitor who
		// never signs in should not become a person.
		person_profiles: 'identified_only',
		capture_pageview: true,
		// Named events only; see the header.
		autocapture: false,
		// Nothing here is a property we would keep, and two of them are how an
		// address reaches an analytics tool by accident.
		property_denylist: ['email', 'seller_email', '$initial_referrer'],
		session_recording: {
			maskAllInputs: true,
			// Every non-input text node too. A catalogue console's text is the
			// seller's own listings.
			maskTextSelector: '*'
		}
	});
}

/** Binds everything captured from here on to one organisation.
 *
 *  The organisation, never the person: a Teachouse account is one org, so
 *  org-as-person is the true unit and there is no second identity space to
 *  keep in step. */
export function identifyOrg(org: string): void {
	if (!started) {
		return;
	}
	posthog.identify(org);
}

/** Forgets who this browser was. Called from sign-out, because without it
 *  the next seller on a shared machine inherits the last one's identity. */
export function resetIdentity(): void {
	if (!started) {
		return;
	}
	posthog.reset();
}

/** Records one named event. A no-op in a build with no key, so a call site
 *  never has to ask whether analytics is on. */
export function capture(event: string, props: Record<string, unknown>): void {
	if (!started) {
		return;
	}
	posthog.capture(event, props);
}
