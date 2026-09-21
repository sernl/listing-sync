// What the landing page's pricing CTA asked to buy, carried across the
// account the seller has to create first.
//
// The trip is longer than a navigation: sign-up sends a verification link
// that is often opened in another tab, a social sign-up leaves through the
// provider's own redirect, and a newly provisioned seller meets the claim
// screen before the console. None of those keep component state, so the
// intent is written down. `localStorage` is the only store that survives all
// three; `sessionStorage` dies with the tab the verification link replaced.
//
// The signup and login pages write it. `/settings/subscription` is the only
// reader, and reading spends it.

import { PRICE_KEYS, type PriceKey } from '$lib/generated/vocab';

export const INTENT_KEY = 'teachouse.signup.intent';

/** How long a written intent still means what it said.
 *
 *  A day covers the slowest honest path — sign up at night, open the
 *  verification email in the morning — and stops a key left behind by an
 *  abandoned sign-up from opening a checkout weeks later, when the seller
 *  has no idea what asked for it. */
const INTENT_LIFE_MS = 24 * 60 * 60 * 1000;

export interface CheckoutIntent {
	/** Where to land once signed in, or null where the link named no page. */
	next: string | null;
	/** The checkout to open on arrival, or null where the link named none. */
	price: PriceKey | null;
	/** When it was written, as epoch milliseconds. */
	at: number;
}

/** A path on this origin, or null.
 *
 *  Anything else is dropped rather than followed: `next` arrives as a query
 *  parameter, and a value that leaves this origin turns our own sign-in into
 *  somebody else's redirector. `//host` is rejected with it — it is a
 *  protocol-relative URL, not a path, however much it looks like one. */
export function safeNext(value: string | null): string | null {
	if (value === null || !value.startsWith('/') || value.startsWith('//')) {
		return null;
	}
	return value;
}

/** A price key this deployment actually sells, or null.
 *
 *  Validated against the generated vocabulary rather than trusted: the key
 *  comes out of a URL or another tab's storage, and a stale or hand-edited
 *  one would open a checkout the API refuses. */
export function safePrice(value: string | null | undefined): PriceKey | null {
	return PRICE_KEYS.find((key) => key === value) ?? null;
}

/** What a stored record means, or null where it means nothing.
 *
 *  Separate from the storage read so it can be checked directly: this is
 *  where a hand-written, half-written or long-dead record is turned away. */
export function parseIntent(raw: string | null, now: number): CheckoutIntent | null {
	if (raw === null) {
		return null;
	}
	let held: unknown;
	try {
		held = JSON.parse(raw);
	} catch {
		return null;
	}
	if (held === null || typeof held !== 'object') {
		return null;
	}
	const { next, price, at } = held as { next?: unknown; price?: unknown; at?: unknown };
	if (typeof at !== 'number' || !Number.isFinite(at) || now - at >= INTENT_LIFE_MS) {
		return null;
	}
	const landing = typeof next === 'string' ? safeNext(next) : null;
	const wanted = typeof price === 'string' ? safePrice(price) : null;
	if (landing === null && wanted === null) {
		return null;
	}
	return { next: landing, price: wanted, at };
}

/** Write down what this sign-up is for. Silent where it cannot: private
 *  browsing refuses storage, and a sign-up that cannot be recorded is still
 *  a sign-up. */
export function writeIntent(intent: { next: string | null; price: PriceKey | null }): void {
	if (intent.next === null && intent.price === null) {
		return;
	}
	try {
		localStorage.setItem(
			INTENT_KEY,
			JSON.stringify({ next: intent.next, price: intent.price, at: Date.now() })
		);
	} catch {
		/* No store to write to. The sign-in link carries both in its query,
		   which covers the one path that does not leave this tab. */
	}
}

/** The standing intent, left where it is.
 *
 *  For the router: which page the sign-in was for is a question that can be
 *  asked without spending the checkout the same record names. */
export function peekIntent(): CheckoutIntent | null {
	try {
		return parseIntent(localStorage.getItem(INTENT_KEY), Date.now());
	} catch {
		return null;
	}
}

/** The standing intent, once.
 *
 *  Reading spends it: the key is dropped before the caller acts on it, so a
 *  refused checkout, a reload, or a second visit does not reopen Stripe on a
 *  seller who never asked twice. A record that no longer parses is dropped
 *  on the same grounds. */
export function readIntent(): CheckoutIntent | null {
	const held = peekIntent();
	try {
		localStorage.removeItem(INTENT_KEY);
	} catch {
		/* Nothing to clear, and nothing that could have been read either. */
	}
	return held;
}
