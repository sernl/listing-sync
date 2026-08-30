// The social sign-in buttons as the client decides to render them.
//
// `tam-auth` configures a provider only when it was given that provider's
// credentials (`auth/src/auth.ts`), and better-auth refuses an unconfigured
// one at `/sign-in/social` with `Provider not found`. A button whose only
// outcome is that refusal is not an option the human has, so the build that
// renders it is told which providers exist rather than discovering it.
//
// Told at build time, deliberately. Asking the identity service at runtime
// would mean publishing which credentials the deployment holds, and would put
// a network round trip in front of the first paint of a sign-in page. The
// deployment already knows the answer when it builds the client, and this is
// the same shape as the dormant Turnstile widget in `captcha.ts`.

/** Every provider `tam-auth` has code for, in the order the buttons appear. A
 * name absent from here is not a provider this client can offer at all. */
export const SOCIAL_PROVIDERS = [
	{ id: 'google', label: 'Google' },
	{ id: 'microsoft', label: 'Microsoft' }
] as const;

export type SocialProvider = (typeof SOCIAL_PROVIDERS)[number]['id'];

/** One rendered button: the provider to hand the browser to, and its name. */
export interface SocialProviderChoice {
	readonly id: SocialProvider;
	readonly label: string;
}

/**
 * The providers this build renders, read from a comma-separated list.
 *
 * The catalogue is filtered rather than the list mapped, which is what makes
 * an unknown name a provider that is not offered instead of a button that
 * cannot work, and what keeps the button order fixed however the variable is
 * written. Absent, blank, or naming nothing known all mean the same thing:
 * no social sign-in.
 */
export function readEnabledProviders(raw: unknown): readonly SocialProviderChoice[] {
	if (typeof raw !== 'string') {
		return [];
	}
	const named = new Set(
		raw
			.split(',')
			.map((entry) => entry.trim().toLowerCase())
			.filter((entry) => entry.length > 0)
	);
	return SOCIAL_PROVIDERS.filter((provider) => named.has(provider.id));
}

/** Substituted by Vite at build time. Absent from every build that does not
 * define it, which is what keeps the buttons and their divider unrendered. */
export const ENABLED_SOCIAL_PROVIDERS = readEnabledProviders(
	import.meta.env.VITE_SOCIAL_PROVIDERS
);
