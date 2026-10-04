// The identity boundary as the client sees it. better-auth owns who the human
// is; the API owns what they may do. This module is the only place the two
// meet: it redeems the identity service's login assertion for the API's own
// session cookie, and it ends both sessions on sign-out.
//
// The assertion never rests anywhere. It is fetched, posted same-origin, and
// discarded, exactly as `docs/notes/design/better-auth-integration.md`
// ("Session bridging") specifies.

import type { Whoami } from '$lib/api';
import type { CaptchaOptions } from '$lib/captcha';
import type { BrowserSession } from '$lib/device-merge';
import type { SocialProvider } from '$lib/social-providers';
import type { ConsentBody } from '$lib/legal';

/** Where the identity service is reached. Same-origin is a requirement rather
 * than a convenience: better-auth's session cookie has to be first-party
 * relative to the dashboard, so `/api/auth` is proxied onto this origin in
 * development and at the ingress in production. */
export const AUTH_BASE_PATH = '/api/auth';

/** better-auth's client, fetched the first time something needs it.
 *
 * A chunk of its own rather than a static import: with its plugins it is the
 * largest script the sign-in screens reached for, and a visitor who has not
 * yet pressed anything needs none of it. The session read below does without
 * it, so a signed-out visit downloads it only once the visitor submits. */
async function createClient() {
	const [{ passkeyClient }, { adminClient, jwtClient }, { createAuthClient }] = await Promise.all([
		import('@better-auth/passkey/client'),
		import('better-auth/client/plugins'),
		import('better-auth/svelte')
	]);
	return createAuthClient({
		basePath: AUTH_BASE_PATH,
		plugins: [passkeyClient(), jwtClient(), adminClient()]
	});
}

let client: ReturnType<typeof createClient> | null = null;

export function authClient(): ReturnType<typeof createClient> {
	client ??= createClient();
	return client;
}

/** The identity session as `GET /api/auth/get-session` answers it, reduced to
 * the fields this client reads. */
interface SessionRead {
	user: { id?: string; name: string; email: string; emailVerified: boolean };
	session: { impersonatedBy?: string | null; createdAt: Date | null };
}

/** The identity session, read without better-auth's client: one same-origin
 * GET, the same request `authClient().getSession()` makes, answered with the
 * session or `null`. A failure of any kind reads as no session, which is what
 * the client's own `data` is on a failure. */
async function readSession(): Promise<SessionRead | null> {
	let body: unknown;
	try {
		const response = await fetch(`${AUTH_BASE_PATH}/get-session`, {
			credentials: 'same-origin',
			headers: { accept: 'application/json' }
		});
		if (!response.ok) {
			return null;
		}
		body = await response.json();
	} catch {
		return null;
	}
	if (
		typeof body !== 'object' ||
		body === null ||
		!('user' in body) ||
		!('session' in body) ||
		typeof body.user !== 'object' ||
		body.user === null ||
		typeof body.session !== 'object' ||
		body.session === null ||
		!('email' in body.user) ||
		typeof body.user.email !== 'string'
	) {
		return null;
	}
	const user = body.user as Record<string, unknown>;
	const session = body.session as Record<string, unknown>;
	return {
		user: {
			id: typeof user.id === 'string' ? user.id : undefined,
			name: typeof user.name === 'string' ? user.name : '',
			email: body.user.email,
			emailVerified: user.emailVerified === true
		},
		session: {
			impersonatedBy: typeof session.impersonatedBy === 'string' ? session.impersonatedBy : null,
			createdAt: signedInAt(session.createdAt)
		}
	};
}

/** better-auth's `createdAt` as the wire carries it (an ISO string), or null
 *  where it is missing or unreadable. */
function signedInAt(value: unknown): Date | null {
	if (typeof value !== 'string' && !(value instanceof Date)) {
		return null;
	}
	const at = new Date(value);
	return Number.isNaN(at.getTime()) ? null : at;
}

/** What the identity service will ask of a seller deleting their account:
 *  the password where the account has one, otherwise a sign-in from the last
 *  few minutes. `signedInAt` is when this browser signed in, or null. */
export interface DeletionProof {
	hasPassword: boolean;
	signedInAt: Date | null;
}

/** Reads both facts. better-auth's `/list-accounts` names the
 *  email-and-password sign-in `credential`. */
export async function deletionProof(): Promise<DeletionProof> {
	const [session, accounts] = await Promise.all([
		readSession(),
		(await authClient()).listAccounts()
	]);
	if (accounts.error) {
		throw refused(accounts.error, 'We could not check how you sign in.');
	}
	return {
		hasPassword: (accounts.data ?? []).some((account) => account.providerId === 'credential'),
		signedInAt: session?.session.createdAt ?? null
	};
}

/** Where a provider returns the browser. Both land on the sign-in page, which
 * is the page that knows how to finish the exchange. */
const SOCIAL_RETURN = '/login';

/** Where better-auth's reset-link callback returns the browser. Relative on
 * purpose: the identity service resolves it against its own base URL, which is
 * the dashboard's origin, and its origin check trusts a relative path without
 * needing the origin listed. */
const PASSWORD_RESET_RETURN = '/reset/confirm';

/** Where a refused social sign-up returns the browser: the page with the
 * boxes, so the reason is read where it can be acted on. */
const SIGN_UP_RETURN = '/signup';

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
	const data = await readSession();
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
	const { data, error } = await (await authClient()).token();
	if (error || !data?.token) {
		throw new BridgeFailure('refused');
	}
	const { api } = await import('$lib/api');
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
	const { api } = await import('$lib/api');
	const endingIdentity = (await authClient()).signOut().then(
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
	return (await authClient()).signIn.email({ email, password }, captcha);
}

/** Make an account. `consent` is the two boxes as the page sent them: better-auth
 * passes body fields it does not know through to the identity service's hook,
 * which refuses the sign-up without them and records them the moment the
 * account exists, removing it again if that write fails. Built as a named
 * body rather than inline so the extra field is not an excess-property error
 * against better-auth's own body type. */
export async function signUpWithPassword(
	name: string,
	email: string,
	password: string,
	consent: ConsentBody,
	captcha?: CaptchaOptions
) {
	const body = { name, email, password, callbackURL: SOCIAL_RETURN, consent };
	return (await authClient()).signUp.email(body, captcha);
}

/**
 * Ask for a reset link. The identity service answers the same way whether or
 * not the address is registered, performing a dummy verification lookup when
 * it finds no user, so its answer carries no evidence about the address. The
 * page that renders the answer carries none either.
 */
export async function requestPasswordReset(email: string, captcha?: CaptchaOptions) {
	return (await authClient()).requestPasswordReset({ email, redirectTo: PASSWORD_RESET_RETURN }, captcha);
}

/** Spend a reset token on a new password. Not a guarded endpoint: the token is
 * the proof, so the captcha plugin does not stand in front of it. */
export async function resetPassword(token: string, newPassword: string) {
	return (await authClient()).resetPassword({ token, newPassword });
}

/** Hands the browser to the provider; the redirect is performed by the
 * client's own redirect plugin on the response. Sign-in only: the providers
 * do not make accounts implicitly, so an unknown account comes back to
 * `/login` with `?error=signup_disabled`. */
export async function signInWithProvider(provider: SocialProvider) {
	return (await authClient()).signIn.social({
		provider,
		callbackURL: SOCIAL_RETURN,
		errorCallbackURL: SOCIAL_RETURN
	});
}

/** Makes an account through a provider. The agreement rides better-auth's
 * signed OAuth state through the provider's redirect and is recorded at the
 * callback, as the account is made; a refusal comes back to `/signup`,
 * where the boxes are, with the reason in `?error=`. */
export async function signUpWithProvider(provider: SocialProvider, consent: ConsentBody) {
	return (await authClient()).signIn.social({
		provider,
		callbackURL: SOCIAL_RETURN,
		errorCallbackURL: SIGN_UP_RETURN,
		requestSignUp: true,
		additionalData: { consent }
	});
}

export async function signInWithPasskey() {
	return (await authClient()).signIn.passkey();
}

export async function resendVerification(email: string) {
	return (await authClient()).sendVerificationEmail({ email, callbackURL: SOCIAL_RETURN });
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
	const { error } = await (await authClient()).updateUser({ name });
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
	const { data, error } = await (await authClient()).passkey.listUserPasskeys();
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
	const { error } = await (await authClient()).passkey.addPasskey(name.length > 0 ? { name } : {});
	if (error) {
		throw refused(error, 'The passkey was not added.');
	}
}

/** Remove one registered passkey. `authClient.passkey.deletePasskey` is the
 * client method the plugin derives from `POST /passkey/delete-passkey`; the
 * server refuses an identifier the session does not own. */
export async function deletePasskey(id: string): Promise<void> {
	const { error } = await (await authClient()).passkey.deletePasskey({ id });
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

/** What the identity plane's account listing is asked for. */
export interface IdentityUserQuery {
	/** A `contains` match on the address; blank lists everyone. */
	search: string;
	limit: number;
	/** How many accounts to skip, in the listing's order. */
	offset?: number;
	/** By when the account was made; newest first unless asked otherwise. */
	direction?: 'asc' | 'desc';
}

export interface IdentityUserPage {
	users: IdentityUser[];
	/** Every account the search matches, not just this page's. */
	total: number;
}

/**
 * One page of the identity plane's accounts, optionally filtered.
 *
 * The search is a `contains` match on the address, which is the field an
 * operator has when a human writes in. A blank search lists the newest
 * accounts rather than none.
 *
 * Refused with 403 for a signed-in human whose identity account carries no
 * admin role — a distinct fact from the platform operator marking, and one the
 * page explains rather than reports as an error.
 */
export async function listIdentityUsers(query: IdentityUserQuery): Promise<IdentityUserPage> {
	const trimmed = query.search.trim();
	const { data, error } = await (await authClient()).admin.listUsers({
		query: {
			limit: query.limit,
			offset: query.offset ?? 0,
			sortBy: 'createdAt',
			sortDirection: query.direction ?? 'desc',
			...(trimmed.length === 0
				? {}
				: {
						searchField: 'email' as const,
						searchOperator: 'contains' as const,
						searchValue: trimmed
					})
		}
	});
	if (error) {
		throw refused(error, 'The identity accounts could not be listed.');
	}
	const users = (data?.users ?? []) as IdentityUser[];
	const total = typeof data?.total === 'number' ? data.total : users.length;
	return { users, total };
}

/**
 * Every account the search matches, read a hundred at a time: what "select
 * all matching" acts on. Bounded by the listing's own total, so an account
 * made mid-walk cannot keep it going.
 */
export async function listAllIdentityUsers(search: string): Promise<IdentityUser[]> {
	const size = 100;
	const all: IdentityUser[] = [];
	let total = Infinity;
	while (all.length < total) {
		const page = await listIdentityUsers({ search, limit: size, offset: all.length });
		all.push(...page.users);
		total = page.total;
		if (page.users.length < size) break;
	}
	return all;
}

/** Ban an account, recording why. better-auth stores the reason and shows it
 * back on the row; the ban has no expiry unless the server configures one. */
export async function banIdentityUser(userId: string, reason: string): Promise<void> {
	const trimmed = reason.trim();
	const { error } = await (await authClient()).admin.banUser({
		userId,
		...(trimmed.length === 0 ? {} : { banReason: trimmed })
	});
	if (error) {
		throw refused(error, 'The account was not banned.');
	}
}

export async function unbanIdentityUser(userId: string): Promise<void> {
	const { error } = await (await authClient()).admin.unbanUser({ userId });
	if (error) {
		throw refused(error, 'The account was not unbanned.');
	}
}

/** Delete an identity account outright: the user, their sessions and their
 * linked sign-in methods. Cannot be undone. The console calls this only
 * after the platform half (`api.adminDeleteUser`) has gone, so a refusal
 * there leaves the account standing. The identity service records the act
 * as `user_removed` in its audit trail. */
export async function removeIdentityUser(userId: string): Promise<void> {
	const { error } = await (await authClient()).admin.removeUser({ userId });
	if (error) {
		throw refused(error, 'The sign-in account was not deleted.');
	}
}

export async function setIdentityRole(userId: string, role: IdentityRole): Promise<void> {
	const { error } = await (await authClient()).admin.setRole({ userId, role });
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
	const { data, error } = await (await authClient()).admin.listUserSessions({ userId });
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
	const { error } = await (await authClient()).admin.revokeUserSessions({ userId });
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
	const { error } = await (await authClient()).admin.impersonateUser({ userId });
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
	const { error } = await (await authClient()).admin.stopImpersonating();
	if (error) {
		throw refused(error, 'The impersonation was not stopped.');
	}
	return establishSession();
}

/** The identity session as the impersonation banner reads it: who the console
 * is acting as, and whether anybody is acting at all. */
export async function impersonatedSession(): Promise<{
	user: { id?: string; name?: string | null; email?: string | null };
	session: { impersonatedBy?: string | null };
} | null> {
	const data = await readSession();
	if (!data) {
		return null;
	}
	return {
		user: { id: data.user.id, name: data.user.name, email: data.user.email },
		session: { impersonatedBy: data.session.impersonatedBy ?? null }
	};
}
