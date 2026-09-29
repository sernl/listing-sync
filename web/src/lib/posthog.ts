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
//
// And nothing here is on the way to a first paint. The library is a third of
// a megabyte of script, so it is a separate chunk fetched only once the page
// has loaded and gone idle, and the signed-out screens do not ask for it until
// the visitor first presses or types (`(auth)/+layout.svelte`). Every call
// below is queued behind that fetch rather than dropped, so an identity set or
// an event captured before the library arrives still lands.

import { afterLoadIdle } from '$lib/idle';

type PostHog = (typeof import('posthog-js'))['default'];

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

/** The initialised client once it has been fetched, or null before
 *  `startTelemetry` and in a build with no key. */
let client: Promise<PostHog> | null = null;

/** Starts capture, once, after the page has loaded and gone idle. Safe to
 *  call from a load that runs on every navigation, and a no-op in a build
 *  with no key. */
export function startTelemetry(): void {
	const key = KEY;
	if (client !== null || !key) {
		return;
	}
	client = afterLoadIdle(window)
		.then(() => import('posthog-js'))
		.then(({ default: posthog }) => {
			posthog.init(key, {
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
			return posthog;
		});
	// A chunk that did not arrive is an analytics gap, not a console fault;
	// the queued calls below are dropped with it.
	client.catch(() => undefined);
}

/** Runs `call` against the client once it is ready. Nothing runs before
 *  `startTelemetry`, and nothing in a build with no key. */
function whenStarted(call: (posthog: PostHog) => void): void {
	void client?.then(call, () => undefined);
}

/** Binds everything captured from here on to one organisation.
 *
 *  The organisation, never the person: a Teachouse account is one org, so
 *  org-as-person is the true unit and there is no second identity space to
 *  keep in step. */
export function identifyOrg(org: string): void {
	whenStarted((posthog) => posthog.identify(org));
}

/** Forgets who this browser was. Called from sign-out, because without it
 *  the next seller on a shared machine inherits the last one's identity. */
export function resetIdentity(): void {
	whenStarted((posthog) => posthog.reset());
}

/** Records one named event. A no-op in a build with no key, so a call site
 *  never has to ask whether analytics is on. */
export function capture(event: string, props: Record<string, unknown>): void {
	whenStarted((posthog) => posthog.capture(event, props));
}
