// The identity service's half of "Your devices", reached through its own
// client SDK.
//
// These live beside the page rather than in `$lib/auth-client` because they
// are the only consumer: the browser session list is a device-page concern,
// and the identity boundary module stays the place where the two planes meet
// rather than a drawer for every endpoint better-auth exposes.
//
// `listSessions` and `revokeSession` are better-auth 1.7.2's own endpoints
// (`/list-sessions`, `/revoke-session`). `revokeSession` takes the session's
// `token`, not its `id`.

import { authClient } from '$lib/auth-client';
import type { BrowserSession } from './merge';

export class SessionFailure extends Error {}

function refused(message: string | undefined, fallback: string): SessionFailure {
	return new SessionFailure(
		typeof message === 'string' && message.length > 0 ? message : fallback
	);
}

/** Every live browser sign-in on this account. */
export async function listBrowserSessions(): Promise<BrowserSession[]> {
	const { data, error } = await authClient.listSessions();
	if (error) {
		throw refused(error.message, 'Your browser sign-ins could not be listed.');
	}
	return (data ?? []) as BrowserSession[];
}

/** The token of the session this browser is using, so the page can mark the
 *  row whose sign-out ends the session the seller is reading it in. */
export async function currentSessionToken(): Promise<string | null> {
	const { data } = await authClient.getSession();
	return data?.session.token ?? null;
}

/** End one browser sign-in. */
export async function revokeBrowserSession(token: string): Promise<void> {
	const { error } = await authClient.revokeSession({ token });
	if (error) {
		throw refused(error.message, 'That sign-in was not ended.');
	}
}
