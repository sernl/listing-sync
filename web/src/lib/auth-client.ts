// The identity boundary as the client sees it. better-auth owns who the human
// is; the API owns what they may do. This module is the only place the two
// meet: it redeems the identity service's login assertion for the API's own
// session cookie, and it ends both sessions on sign-out.
//
// The assertion never rests anywhere. It is fetched, posted same-origin, and
// discarded, exactly as `docs/notes/design/better-auth-integration.md`
// ("Session bridging") specifies.

import { passkeyClient } from '@better-auth/passkey/client';
import { adminClient, jwtClient } from 'better-auth/client/plugins';
import { createAuthClient } from 'better-auth/svelte';
import { api, type Whoami } from '$lib/api';
import type { CaptchaOptions } from '$lib/captcha';
import type { BrowserSession } from '$lib/device-merge';
import type { SocialProvider } from '$lib/social-providers';

/** Where the identity service is reached. Same-origin is a requirement rather
 * than a convenience: better-auth's session cookie has to be first-party
 * relative to the dashboard, so `/api/auth` is proxied onto this origin in
 * development and at the ingress in production. */
export const AUTH_BASE_PATH = '/api/auth';

export const authClient = createAuthClient({
	basePath: AUTH_BASE_PATH,
	plugins: [passkeyClient(), jwtClient(), adminClient()]
});

/** Where a provider returns the browser. Both land on the sign-in page, which
 * is the page that knows how to finish the exchange. */
const SOCIAL_RETURN = '/login';

/** Where better-auth's reset-link callback returns the browser. Relative on
 * purpose: the identity service resolves it against its own base URL, which is
 * the dashboard's origin, and its origin check trusts a relative path without
 * needing the origin listed. */
const PASSWORD_RESET_RETURN = '/reset/confirm';

/** What the identity service says about the signed-in human, reduced to the
 * facts this client acts on. */
export interface Identity {
	email: string;
	emailVerified: boolean;
	/** The display name better-auth holds. It is the only part of the identity
	 * the settings page can change: `/update-user` refuses an address outright
	 * (`BASE_ERROR_CODES.EMAIL_CAN_NOT_BE_UPDATED`), because changing one is
	 * the identity service's own verification flow. */
	name: string;
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
	return {
		email: data.user.email,
		emailVerified: data.user.emailVerified,
		name: data.user.name
	};
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

export async function signInWithPassword(
	email: string,
	password: string,
	captcha?: CaptchaOptions
) {
	return authClient.signIn.email({ email, password }, captcha);
}

export async function signUpWithPassword(
	name: string,
	email: string,
	password: string,
	captcha?: CaptchaOptions
) {
	return authClient.signUp.email({ name, email, password, callbackURL: SOCIAL_RETURN }, captcha);
}

/**
 * Ask for a reset link. The identity service answers the same way whether or
 * not the address is registered, performing a dummy verification lookup when
 * it finds no user, so its answer carries no evidence about the address. The
 * page that renders the answer carries none either.
 */
export async function requestPasswordReset(email: string, captcha?: CaptchaOptions) {
	return authClient.requestPasswordReset({ email, redirectTo: PASSWORD_RESET_RETURN }, captcha);
}

/** Spend a reset token on a new password. Not a guarded endpoint: the token is
 * the proof, so the captcha plugin does not stand in front of it. */
export async function resetPassword(token: string, newPassword: string) {
	return authClient.resetPassword({ token, newPassword });
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

// ------------------------------------------------- the account settings page

/**
 * A refusal from the identity service, or from the browser's own WebAuthn
 * ceremony, carrying the code the settings page branches on.
 *
 * better-auth's client answers with `{ data, error }` rather than rejecting,
 * and its passkey plugin reports a cancelled ceremony the same way. The query
 * cache branches on a rejected promise, so the wrappers below raise the
 * refusal once here instead of at each call site.
 */
export class AuthFailure extends Error {
	readonly code: string | undefined;
	/** The HTTP status the identity service answered with, where it answered
	 * with one. The admin surface's 403 is a distinct fact from a fault, and
	 * `identityAdminRefusal` is what reads it. */
	readonly status: number | undefined;

	constructor(message: string, code?: string, status?: number) {
		super(message);
		this.code = code;
		this.status = status;
	}
}

/** The code the passkey plugin reports when the human dismissed the browser's
 * prompt. Not a fault: the page says so and leaves the list alone. */
export const CEREMONY_ABORTED = 'ERROR_CEREMONY_ABORTED';

/** The code the plugin reports when the authenticator offered is already
 * registered to this account. */
export const ALREADY_REGISTERED = 'ERROR_AUTHENTICATOR_PREVIOUSLY_REGISTERED';

interface ClientRefusal {
	message?: string | undefined;
	code?: string | undefined;
	status?: number | undefined;
}

function refused(error: ClientRefusal | null | undefined, fallback: string): AuthFailure {
	const message = error?.message;
	return new AuthFailure(
		typeof message === 'string' && message.length > 0 ? message : fallback,
		error?.code,
		error?.status
	);
}

/** Change the display name better-auth holds for the signed-in human.
 * `authClient.updateUser` is the client method better-auth derives from
 * `POST /update-user`; it answers `{ status: true }` rather than the updated
 * user, so the caller refetches the session to render the stored name. */
export async function updateDisplayName(name: string): Promise<void> {
	const { error } = await authClient.updateUser({ name });
	if (error) {
		throw refused(error, 'The name could not be changed.');
	}
}

/** One registered passkey, narrowed to what the settings list renders.
 * `createdAt` is typed loosely because better-auth stores a date and what
 * survives the wire depends on the client's parser; the page hands it to
 * `Date` either way. */
export interface PasskeyRecord {
	id: string;
	name?: string | null;
	backedUp?: boolean;
	createdAt?: string | Date | null;
}

/**
 * Whether this browser exposes WebAuthn at all.
 *
 * A browser without it cannot register or present a passkey, and the settings
 * page says so rather than offering a control whose only outcome is a failure
 * the human cannot act on. This is the capability check, not a claim that a
 * ceremony will succeed: an authenticator can still be absent or declined.
 */
export function passkeysSupported(): boolean {
	return typeof PublicKeyCredential !== 'undefined';
}

/** The account's registered passkeys. `authClient.passkey.listUserPasskeys` is
 * the client method the plugin derives from `GET /passkey/list-user-passkeys`. */
export async function listPasskeys(): Promise<PasskeyRecord[]> {
	const { data, error } = await authClient.passkey.listUserPasskeys();
	if (error) {
		throw refused(error, 'We could not load your passkeys.');
	}
	return (data ?? []) as PasskeyRecord[];
}

/**
 * Register a passkey against the signed-in account.
 *
 * The label is what the list shows afterwards; better-auth stores it verbatim
 * and treats a blank one as absent, so an empty box is passed as no label
 * rather than as an empty name. This opens the browser's WebAuthn prompt and
 * cannot complete without the human, which is why the refusal path carries the
 * plugin's code rather than a generic failure.
 */
export async function registerPasskey(label: string): Promise<void> {
	const name = label.trim();
	const { error } = await authClient.passkey.addPasskey(name.length > 0 ? { name } : {});
	if (error) {
		throw refused(error, 'The passkey was not added.');
	}
}

/** Remove one registered passkey. `authClient.passkey.deletePasskey` is the
 * client method the plugin derives from `POST /passkey/delete-passkey`; the
 * server refuses an identifier the session does not own. */
export async function deletePasskey(id: string): Promise<void> {
	const { error } = await authClient.passkey.deletePasskey({ id });
	if (error) {
		throw refused(error, 'The passkey was not removed.');
	}
}

// ------------------------------- the identity plane's administration surface

/**
 * One identity-plane account as the admin plugin lists it.
 *
 * Not a platform user. These are `auth."user"` rows, which reach `app_user`
 * only through the `auth_subject` join, so an id here names a subject and
 * never an organisation's member.
 */
export interface IdentityUser {
	id: string;
	email: string;
	name: string;
	emailVerified: boolean;
	role?: string | null;
	banned?: boolean | null;
	banReason?: string | null;
	createdAt?: string | Date | null;
}

/** The two roles this console sets. better-auth's plugin accepts any string;
 * the console offers only the pair its server configuration defines, so a
 * typo cannot mint a role nothing grants. */
export const IDENTITY_ROLES = ['admin', 'user'] as const;
export type IdentityRole = (typeof IDENTITY_ROLES)[number];

/**
 * The identity plane's accounts, optionally filtered.
 *
 * The search is a `contains` match on the address, which is the field an
 * operator has when a human writes in. A blank search lists the newest
 * accounts rather than none.
 *
 * Refused with 403 for a signed-in human whose identity account carries no
 * admin role — a distinct fact from the platform operator marking, and one the
 * page explains rather than reports as an error.
 */
export async function listIdentityUsers(search: string, limit: number): Promise<IdentityUser[]> {
	const trimmed = search.trim();
	const { data, error } = await authClient.admin.listUsers({
		query: {
			limit,
			sortBy: 'createdAt',
			sortDirection: 'desc',
			...(trimmed.length === 0
				? {}
				: { searchField: 'email' as const, searchOperator: 'contains' as const, searchValue: trimmed })
		}
	});
	if (error) {
		throw refused(error, 'The identity accounts could not be listed.');
	}
	return (data?.users ?? []) as IdentityUser[];
}

/** Ban an account, recording why. better-auth stores the reason and shows it
 * back on the row; the ban has no expiry unless the server configures one. */
export async function banIdentityUser(userId: string, reason: string): Promise<void> {
	const trimmed = reason.trim();
	const { error } = await authClient.admin.banUser({
		userId,
		...(trimmed.length === 0 ? {} : { banReason: trimmed })
	});
	if (error) {
		throw refused(error, 'The account was not banned.');
	}
}

export async function unbanIdentityUser(userId: string): Promise<void> {
	const { error } = await authClient.admin.unbanUser({ userId });
	if (error) {
		throw refused(error, 'The account was not unbanned.');
	}
}

export async function setIdentityRole(userId: string, role: IdentityRole): Promise<void> {
	const { error } = await authClient.admin.setRole({ userId, role });
	if (error) {
		throw refused(error, 'The role was not changed.');
	}
}

/**
 * Every live sign-in on one account, as the admin plugin lists them.
 *
 * The API cannot answer this. `tam_app` holds `SELECT` on `auth.auth_event`
 * and nothing else in that schema — no user, no session, no account, written
 * down as a boundary in `db/auth/0002_audit_event.sql` — so the session rows
 * come from the identity service itself and are joined to the platform's own
 * user rows in the browser, on `auth_subject`. The alternative was a grant on
 * `auth."session"`, which would have reversed that line for a column an
 * operator page reads once.
 *
 * Listed per account rather than in bulk, because that is the shape of the
 * plugin's endpoint: `GET /admin/list-user-sessions` takes one `userId`.
 */
export async function listUserSessions(userId: string): Promise<BrowserSession[]> {
	const { data, error } = await authClient.admin.listUserSessions({ userId });
	if (error) {
		throw refused(error, "That account's sign-ins could not be listed.");
	}
	return (data?.sessions ?? []) as BrowserSession[];
}

/**
 * End every sign-in on one account.
 *
 * Immediate, because the session cookie cache is off (`auth/src/auth.ts`): with
 * it on, a revoked session would keep working for up to five minutes, and a
 * sign-out-everywhere that does not sign anybody out for five minutes is worse
 * than none. Revoking all rather than one row at a time is the whole control
 * an operator wants here — a single stolen session is the account's own
 * concern on Settings.
 */
export async function revokeUserSessions(userId: string): Promise<void> {
	const { error } = await authClient.admin.revokeUserSessions({ userId });
	if (error) {
		throw refused(error, "That account's sign-ins were not ended.");
	}
}

/**
 * Sign in as another identity account, and carry the console with it.
 *
 * Two steps, and the second is the one that matters. `impersonateUser` moves
 * the *identity* session onto the target and nothing more; the console's own
 * session is a separate cookie the API minted, and until it is re-established
 * every page would still be reading the operator's organisation while the
 * banner claimed otherwise. `establishSession` is what makes the two agree.
 *
 * The exchange can refuse — an unverified target is the case that will actually
 * occur — and a refusal here leaves the identity session impersonating with no
 * matching app session, which is the one state that must not persist. So a
 * refusal is undone rather than reported: the impersonation is stopped, the
 * operator's own app session is restored, and the refusal is raised for the
 * page to render.
 */
export async function impersonateAndCarry(userId: string): Promise<Whoami> {
	const { error } = await authClient.admin.impersonateUser({ userId });
	if (error) {
		throw refused(error, 'The impersonation was refused.');
	}
	try {
		return await establishSession();
	} catch (failure) {
		await stopImpersonatingAndRestore().catch(() => undefined);
		throw failure;
	}
}

/**
 * End an impersonation and return the console to the operator's own session.
 *
 * The same two steps in reverse, and the second is again what keeps the two
 * planes in agreement: `stopImpersonating` restores the identity session, and
 * the app session is re-minted from it.
 */
export async function stopImpersonatingAndRestore(): Promise<Whoami> {
	const { error } = await authClient.admin.stopImpersonating();
	if (error) {
		throw refused(error, 'The impersonation was not stopped.');
	}
	return establishSession();
}

/** The identity session as the impersonation banner reads it: who the console
 * is acting as, and whether anybody is acting at all. */
export async function impersonatedSession(): Promise<{
	user: { name?: string | null; email?: string | null };
	session: { impersonatedBy?: string | null };
} | null> {
	const { data } = await authClient.getSession();
	if (!data) {
		return null;
	}
	return {
		user: { name: data.user.name, email: data.user.email },
		session: { impersonatedBy: data.session.impersonatedBy ?? null }
	};
}
