// The billing checkout as the client decides to offer it.
//
// The same dormant shape as `captcha.ts` and `social-providers.ts`: a build
// that was not given Paddle's client token and price map renders the plan
// state and no purchase buttons, and loads nothing from Paddle. Told at
// build time deliberately — a runtime probe would publish which credentials
// the deployment holds, and the deployment already knows the answer when it
// builds the client.

/** The key the checkout puts the organisation identifier under, mirroring
 *  `tam_api::billing::ORG_CUSTOM_DATA_KEY`. The webhook reads this key and no
 *  other, so a checkout that stopped setting it would produce events the
 *  server silently ignores. */
export const ORG_CUSTOM_DATA_KEY = 'org';

/** Paddle's documented Billing v2 script. */
export const PADDLE_SCRIPT = 'https://cdn.paddle.com/paddle/v2/paddle.js';

export type PaddleEnvironment = 'production' | 'sandbox';

export interface PaddleConfig {
	clientToken: string;
	/** Paddle price identifiers by the key the page asks for.
	 *
	 *  A map rather than the one identifier this used to carry: the
	 *  deployment now sells a recurring plan at two cadences and five
	 *  one-off ladder rungs, and one price id across seven buttons would
	 *  charge the same amount for seven different promises. Keys are
	 *  `subscriber_monthly`, `subscriber_yearly` and `rung_<n>`, which is
	 *  what the server's `--paddle-price-map` names the other side of. */
	prices: Readonly<Record<string, string>>;
	environment: PaddleEnvironment;
}

function readText(raw: unknown): string | null {
	if (typeof raw !== 'string') {
		return null;
	}
	const trimmed = raw.trim();
	return trimmed.length > 0 ? trimmed : null;
}

/** The price map as the build was given it, or null.
 *
 *  Null rather than a partial map for anything that is not a JSON object of
 *  non-empty strings: a half-read map renders some buttons and silently
 *  withholds others, which reads as a broken page rather than as a
 *  deployment that was not configured. An empty object is null too — there
 *  is nothing to sell. */
export function readPriceMap(raw: unknown): Record<string, string> | null {
	const text = readText(raw);
	if (text === null) {
		return null;
	}
	let parsed: unknown;
	try {
		parsed = JSON.parse(text);
	} catch {
		return null;
	}
	if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
		return null;
	}
	const prices: Record<string, string> = {};
	for (const [key, value] of Object.entries(parsed)) {
		const id = readText(value);
		if (id === null) {
			return null;
		}
		prices[key] = id;
	}
	return Object.keys(prices).length === 0 ? null : prices;
}

/**
 * The checkout this build offers, or none.
 *
 * An unset environment is production, which is Paddle's own default. A set
 * one that names neither environment yields no configuration at all rather
 * than falling back to production: the two differ in whether a real card is
 * charged, so a typo must disable the buttons rather than pick the answer
 * that spends money.
 */
export function readPaddleConfig(
	clientToken: unknown,
	prices: unknown,
	environment: unknown
): PaddleConfig | null {
	const token = readText(clientToken);
	const map = readPriceMap(prices);
	if (token === null || map === null) {
		return null;
	}
	const named = readText(environment)?.toLowerCase() ?? 'production';
	if (named !== 'production' && named !== 'sandbox') {
		return null;
	}
	return { clientToken: token, prices: map, environment: named };
}

/** The key a recurring plan's price is held under, at one cadence. */
export function planPriceKey(plan: string, cadence: 'monthly' | 'annual'): string {
	return `${plan}_${cadence === 'annual' ? 'yearly' : 'monthly'}`;
}

/** The key one import ladder rung's price is held under. */
export function rungPriceKey(upTo: number): string {
	return `rung_${upTo}`;
}

/** Substituted by Vite at build time. Absent from every build that does not
 *  define both the token and the map, which is what keeps the buttons and
 *  Paddle's script dormant. */
export const PADDLE_CONFIG = readPaddleConfig(
	import.meta.env.VITE_PADDLE_CLIENT_TOKEN,
	import.meta.env.VITE_PADDLE_PRICES,
	import.meta.env.VITE_PADDLE_ENVIRONMENT
);

/** How the billing panel tints a subscription status.
 *
 * Paddle's vocabulary is open and passed through by the server, so a status
 * this client does not recognise is rendered plainly rather than tinted as
 * something it may not be. */
export function subscriptionTone(status: string): 'ok' | 'run' | 'bad' | 'mut' {
	switch (status) {
		case 'active':
		case 'trialing':
			return 'ok';
		case 'past_due':
			return 'run';
		case 'canceled':
			return 'bad';
		default:
			return 'mut';
	}
}

interface PaddleGlobal {
	Environment: { set(environment: PaddleEnvironment): void };
	Initialize(options: { token: string }): void;
	Checkout: {
		open(options: {
			items: { priceId: string; quantity: number }[];
			customData: Record<string, string>;
			customer?: { email: string };
		}): void;
	};
}

declare global {
	interface Window {
		Paddle?: PaddleGlobal;
	}
}

let loading: Promise<PaddleGlobal> | null = null;

/** Fetches Paddle's script once per page, and only from a call that a
 *  configured build made. */
function loadPaddle(): Promise<PaddleGlobal> {
	loading ??= new Promise<PaddleGlobal>((resolve, reject) => {
		const existing = window.Paddle;
		if (existing !== undefined) {
			resolve(existing);
			return;
		}
		const script = document.createElement('script');
		script.src = PADDLE_SCRIPT;
		script.async = true;
		script.onload = () => {
			const loaded = window.Paddle;
			if (loaded === undefined) {
				reject(new Error('Paddle loaded without defining its global'));
				return;
			}
			resolve(loaded);
		};
		script.onerror = () => reject(new Error('Paddle could not be loaded'));
		document.head.append(script);
	});
	return loading;
}

let initialised = false;

/** Opens Paddle's overlay checkout for one price, for this organisation.
 *
 * The organisation travels in `customData` under the key the webhook reads,
 * because that payload is the only thing the webhook can learn a tenant from:
 * Paddle calls the server with no session of ours.
 *
 * A key the map does not carry throws rather than opening a checkout for
 * some other price: the page renders a button only for a key it found, so
 * reaching here without one is a wiring fault and not a seller's mistake. */
export async function openCheckout(
	config: PaddleConfig,
	priceKey: string,
	org: string,
	email?: string
): Promise<void> {
	const priceId = config.prices[priceKey];
	if (priceId === undefined) {
		throw new Error(`this build carries no Paddle price for ${priceKey}`);
	}
	const paddle = await loadPaddle();
	if (!initialised) {
		if (config.environment === 'sandbox') {
			paddle.Environment.set('sandbox');
		}
		paddle.Initialize({ token: config.clientToken });
		initialised = true;
	}
	paddle.Checkout.open({
		items: [{ priceId, quantity: 1 }],
		customData: { [ORG_CUSTOM_DATA_KEY]: org },
		...(email === undefined ? {} : { customer: { email } })
	});
}
