// Deleting your own account, the console's half: what the seller is asked
// for, and what each refusal from `DELETE /v1/account` means for the sheet.
// Pure, so the rules test without a component; `DeleteAccount.svelte` is the
// only caller.

import { ApiFailure } from '$lib/http';
import { MARKETING_URL } from '$lib/site';
import type { DeletionProof } from '$lib/auth-client';

/** The word that confirms in place of the account's name. */
export const CONFIRM_WORD = 'DELETE';

/** The identity service's freshness window for an account with no password
 *  (`auth/src/offboarding.ts`). The sheet asks for a fresh sign-in a minute
 *  sooner, so typing the confirmation does not run the clock out. */
export const FRESH_MS = 5 * 60 * 1000;
const TYPING_MARGIN_MS = 60 * 1000;

/** Where a seller sent to sign in again comes back to: this page, with the
 *  sheet open. */
export const REAUTH_RETURN = '/settings?delete=1';

/** The sign-in page, told to come back here afterwards. */
export const REAUTH_URL = `/login?next=${encodeURIComponent(REAUTH_RETURN)}`;

/** The landing page's goodbye. */
export const GOODBYE_URL = `${MARKETING_URL}/?deleted=1`;

/** The longest reason the server keeps (migration 0104). */
export const REASON_MAX_CHARS = 1000;

/** Whether this seller has to sign in again before the sheet can ask for the
 *  rest: an account with no password proves itself by a recent sign-in. */
export function needsFreshSignIn(proof: DeletionProof, now: number): boolean {
	if (proof.hasPassword) {
		return false;
	}
	return (
		proof.signedInAt === null || now - proof.signedInAt.getTime() >= FRESH_MS - TYPING_MARGIN_MS
	);
}

/** Whether the typed confirmation is one the server accepts: the account's
 *  name in any case, or `DELETE` in capitals. */
export function confirmationAccepted(typed: string, slug: string | null): boolean {
	const said = typed.trim();
	if (said === CONFIRM_WORD) {
		return true;
	}
	return slug !== null && said.length > 0 && said.toLowerCase() === slug.toLowerCase();
}

/** The body the sheet sends, with empty optional fields left out. */
export function deletionBody(
	confirm: string,
	password: string,
	reason: string
): { confirm: string; password?: string; reason?: string } {
	const trimmedReason = reason.trim();
	return {
		confirm: confirm.trim(),
		...(password.length > 0 ? { password } : {}),
		...(trimmedReason.length > 0 ? { reason: trimmedReason } : {})
	};
}

/** What the sheet does with a failed deletion. `reauthenticate` sends the
 *  seller to sign in again; everything else is shown in the sheet in the
 *  server's own words, which already say whether anything was deleted. */
export type DeletionOutcome =
	| { kind: 'reauthenticate' }
	| { kind: 'show'; message: string; field: 'password' | 'confirm' | null };

const FALLBACK = "We couldn't delete your account. Nothing was deleted. Try again in a minute.";

export function deletionFailure(failure: unknown): DeletionOutcome {
	if (!(failure instanceof ApiFailure)) {
		return { kind: 'show', message: FALLBACK, field: null };
	}
	const entry = failure.body?.errors?.[0];
	const detail = entry?.detail;
	const refusal =
		typeof detail === 'object' && detail !== null && 'refusal' in detail
			? detail.refusal
			: undefined;
	if (refusal === 'reauthenticate') {
		return { kind: 'reauthenticate' };
	}
	const message = entry?.message ?? FALLBACK;
	const field =
		refusal === 'password' || refusal === 'password_required'
			? 'password'
			: refusal === 'confirmation'
				? 'confirm'
				: null;
	return { kind: 'show', message, field };
}
