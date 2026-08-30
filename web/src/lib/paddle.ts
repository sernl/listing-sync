// The billing checkout as the client decides to offer it.
//
// The same dormant shape as `captcha.ts` and `social-providers.ts`: a build
// that was not given Paddle's client token and price identifier renders the
// subscription state and no Subscribe button, and loads nothing from Paddle.
// Told at build time deliberately — a runtime probe would publish which
// credentials the deployment holds, and the deployment already knows the
// answer when it builds the client.

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
	priceId: string;
	environment: PaddleEnvironment;
}

function readText(raw: unknown): string | null {
	if (typeof raw !== 'string') {
		return null;
	}
	const trimmed = raw.trim();
	return trimmed.length > 0 ? trimmed : null;
}

/**
 * The checkout this build offers, or none.
 *
 * An unset environment is production, which is Paddle's own default. A set
 * one that names neither environment yields no configuration at all rather
 * than falling back to production: the two differ in whether a real card is
 * charged, so a typo must disable the button rather than pick the answer that
 * spends money.
 */
export function readPaddleConfig(
	clientToken: unknown,
	priceId: unknown,
	environment: unknown
): PaddleConfig | null {
	const token = readText(clientToken);
	const price = readText(priceId);
	if (token === null || price === null) {
		return null;
	}
	const named = readText(environment)?.toLowerCase() ?? 'production';
	if (named !== 'production' && named !== 'sandbox') {
		return null;
	}
	return { clientToken: token, priceId: price, environment: named };
}

/** Substituted by Vite at build time. Absent from every build that does not
 *  define both, which is what keeps the button and Paddle's script dormant. */
export const PADDLE_CONFIG = readPaddleConfig(
	import.meta.env.VITE_PADDLE_CLIENT_TOKEN,
	import.meta.env.VITE_PADDLE_PRICE_ID,
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

/** Opens Paddle's overlay checkout for this organisation.
 *
 * The organisation travels in `customData` under the key the webhook reads,
 * because that payload is the only thing the webhook can learn a tenant from:
 * Paddle calls the server with no session of ours. */
export async function openCheckout(
	config: PaddleConfig,
	org: string,
	email?: string
): Promise<void> {
	const paddle = await loadPaddle();
	if (!initialised) {
		if (config.environment === 'sandbox') {
			paddle.Environment.set('sandbox');
		}
		paddle.Initialize({ token: config.clientToken });
		initialised = true;
	}
	paddle.Checkout.open({
		items: [{ priceId: config.priceId, quantity: 1 }],
		customData: { [ORG_CUSTOM_DATA_KEY]: org },
		...(email === undefined ? {} : { customer: { email } })
	});
}
