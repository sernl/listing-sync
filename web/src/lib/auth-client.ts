// The identity boundary as the client sees it. better-auth owns who the human
// is; the API owns what they may do. This module is the only place the two
// meet: it redeems the identity service's login assertion for the API's own
// session cookie, and it ends both sessions on sign-out.
//
// The assertion never rests anywhere. It is fetched, posted same-origin, and
// discarded, exactly as `docs/notes/design/better-auth-integration.md`
// ("Session bridging") specifies.

import { passkeyClient } from '@better-auth/passkey/client';
import { jwtClient } from 'better-auth/client/plugins';
import { createAuthClient } from 'better-auth/svelte';
import { api, type Whoami } from '$lib/api';

/** Where the identity service is reached. Same-origin is a requirement rather
 * than a convenience: better-auth's session cookie has to be first-party
 * relative to the dashboard, so `/api/auth` is proxied onto this origin in
 * development and at the ingress in production. */
export const AUTH_BASE_PATH = '/api/auth';

export const authClient = createAuthClient({
	basePath: AUTH_BASE_PATH,
	plugins: [passkeyClient(), jwtClient()]
});

/** The social providers `tam-auth` is configured for (`auth/src/auth.ts`). */
export const SOCIAL_PROVIDERS = [
	{ id: 'google', label: 'Google' },
	{ id: 'microsoft', label: 'Microsoft' }
] as const;

export type SocialProvider = (typeof SOCIAL_PROVIDERS)[number]['id'];

/** Where a provider returns the browser. Both land on the sign-in page, which
 * is the page that knows how to finish the exchange. */
const SOCIAL_RETURN = '/login';

/** What the identity service says about the signed-in human, reduced to the
 * two facts this client acts on. */
export interface Identity {
	email: string;
	emailVerified: boolean;
}

/** Why a session could not be established, in the terms the sign-in page
 * renders. */
export type BridgeRefusal = 'no-identity' | 'unverified-email' | 'refused';

export class BridgeFailure extends Error {
	readonly refusal: BridgeRefusal;

	constructor(refusal: BridgeRefusal) {
		super(`the login assertion was not exchanged: ${refusal}`);
		this.refusal = refusal;
	}
}

/** The current identity-service session, or null when there is none. */
export async function identity(): Promise<Identity | null> {
	const { data } = await authClient.getSession();
	if (!data) {
		return null;
	}
	return { email: data.user.email, emailVerified: data.user.emailVerified };
}

/**
 * Redeem the identity service's assertion for the API's session cookie.
 *
 * The unverified-address check is made here as well as in the API because the
 * two answers serve different readers. The API refuses an unverified subject
 * with the same blank 401 it gives a forged token, deliberately, so that no
 * caller can use it as an oracle (`crates/tam-api/src/auth.rs`). The browser
 * already holds the same fact first-hand from its own session, so it can name
 * the reason without telling an attacker anything they did not supply.
 */
export async function establishSession(): Promise<Whoami> {
	const who = await identity();
	if (!who) {
		throw new BridgeFailure('no-identity');
	}
	if (!who.emailVerified) {
		throw new BridgeFailure('unverified-email');
	}
	const { data, error } = await authClient.token();
	if (error || !data?.token) {
		throw new BridgeFailure('refused');
	}
	return api.exchange(data.token);
}

/**
 * End both sessions, and report whether both actually ended.
 *
 * Signing out of the identity service stops new assertions being issued but
 * does not by itself end an already-minted API session, and the reverse is
 * equally true, so the dashboard is the only thing that can make sign-out
 * exact. Both calls are started before either is awaited, so a failure in one
 * cannot skip the other.
 */
export async function signOutEverywhere(): Promise<boolean> {
	const endingIdentity = authClient.signOut().then(
		({ error }) => !error,
		() => false
	);
	const endingApi = api.logout().then(
		() => true,
		() => false
	);
	const [identityEnded, apiEnded] = await Promise.all([endingIdentity, endingApi]);
	return identityEnded && apiEnded;
}

export async function signInWithPassword(email: string, password: string) {
	return authClient.signIn.email({ email, password });
}

export async function signUpWithPassword(name: string, email: string, password: string) {
	return authClient.signUp.email({ name, email, password, callbackURL: SOCIAL_RETURN });
}

/** Hands the browser to the provider; the redirect is performed by the
 * client's own redirect plugin on the response. */
export async function signInWithProvider(provider: SocialProvider) {
	return authClient.signIn.social({
		provider,
		callbackURL: SOCIAL_RETURN,
		errorCallbackURL: SOCIAL_RETURN
	});
}

export async function signInWithPasskey() {
	return authClient.signIn.passkey();
}

export async function resendVerification(email: string) {
	return authClient.sendVerificationEmail({ email, callbackURL: SOCIAL_RETURN });
}
