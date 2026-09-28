// What the bulk bar on the users page does to each ticked account, one at
// a time. A loop in the browser rather than a bulk endpoint: the identity
// half of every action is better-auth's own admin API, which takes one
// account per call, and a loop over a page or two of accounts is quick
// enough that a server-side batch would buy nothing but a second code path.
// Each account gets its own result, so a refusal for one never hides what
// happened to the rest.

import { ApiFailure, api } from '$lib/api';
import {
	banIdentityUser,
	removeIdentityUser,
	setIdentityRole,
	unbanIdentityUser,
	type IdentityUser
} from '$lib/auth-client';
import type { IconName } from '$lib/icons';

export type BulkAction =
	'delete' | 'ban' | 'unban' | 'role-user' | 'role-admin' | 'make-operator' | 'remove-operator';

export interface BulkWords {
	/** The bulk bar's button. */
	label: string;
	/** The confirm dialog's question, with `{n}` for the count. */
	question: string;
	/** One sentence under it: what happens to each account. */
	effect: string;
	/** The confirm button, with `{n}` for the count. */
	confirm: string;
	/** What one success is called in the results. */
	done: string;
	danger: boolean;
	icon: IconName;
}

export const BULK_WORDS: Record<BulkAction, BulkWords> = {
	delete: {
		label: 'Delete',
		question: 'Delete {n}?',
		effect:
			'Each loses their sign-in account, and their organisation with everything in it. This cannot be undone.',
		confirm: 'Delete {n}',
		done: 'deleted',
		danger: true,
		icon: 'trash-2'
	},
	ban: {
		label: 'Ban',
		question: 'Ban {n}?',
		effect: 'They can no longer sign in, starting now. You can unban them later.',
		confirm: 'Ban {n}',
		done: 'banned',
		danger: true,
		icon: 'lock'
	},
	unban: {
		label: 'Unban',
		question: 'Unban {n}?',
		effect: 'They can sign in again.',
		confirm: 'Unban {n}',
		done: 'unbanned',
		danger: false,
		icon: 'circle-check'
	},
	'role-user': {
		label: 'Make ordinary user',
		question: 'Take identity admin away from {n}?',
		effect: 'They can no longer ban, sign in as, or delete other accounts.',
		confirm: 'Remove admin from {n}',
		done: 'now an ordinary user',
		danger: false,
		icon: 'circle-user'
	},
	'role-admin': {
		label: 'Make identity admin',
		question: 'Make {n} identity admin?',
		effect: 'They can ban, sign in as, and delete other sign-in accounts.',
		confirm: 'Make {n} admin',
		done: 'now identity admin',
		danger: false,
		icon: 'shield-check'
	},
	'make-operator': {
		label: 'Make operator',
		question: 'Make {n} operator?',
		effect:
			'They can open this Admin area and read every organisation. Only people who have opened the app once can be operators.',
		confirm: 'Make {n} operator',
		done: 'now an operator',
		danger: false,
		icon: 'shield-check'
	},
	'remove-operator': {
		label: 'Remove operator',
		question: 'Remove operator from {n}?',
		effect: 'They lose this Admin area on their next page load.',
		confirm: 'Remove operator from {n}',
		done: 'no longer an operator',
		danger: false,
		icon: 'circle-x'
	}
};

/** The actions the bulk bar offers, in the order it shows them. */
export const BULK_ORDER: readonly BulkAction[] = [
	'make-operator',
	'remove-operator',
	'role-admin',
	'role-user',
	'unban',
	'ban',
	'delete'
];

/** `{n}` in a phrase, as "1 account" or "12 accounts". */
export function countWords(phrase: string, count: number): string {
	return phrase.replace('{n}', `${count} ${count === 1 ? 'account' : 'accounts'}`);
}

/**
 * Why an action is not even tried on this account, or null.
 *
 * Only yourself is refused here: every other refusal (an operator cannot
 * be deleted, an account with no app user cannot be an operator) is the
 * server's to make, and it words them itself.
 */
export function refusedBeforehand(
	action: BulkAction,
	target: IdentityUser,
	selfId: string | null
): string | null {
	if (selfId === null || target.id !== selfId) return null;
	switch (action) {
		case 'delete':
			return 'That is you. You cannot delete yourself.';
		case 'ban':
			return 'That is you. You cannot ban yourself.';
		case 'role-user':
			return 'That is you. Ask another admin to change your role.';
		case 'remove-operator':
			return 'That is you. Ask another operator to remove you.';
		default:
			return null;
	}
}

/** Delete one account, platform half first, exactly as the single delete
 *  does: a refusal there leaves the sign-in account standing. */
async function deleteAccount(id: string): Promise<void> {
	try {
		await api.adminDeleteUser(id);
	} catch (failure) {
		// No platform user behind this subject: nothing on that side.
		if (!(failure instanceof ApiFailure && failure.status === 404)) throw failure;
	}
	await removeIdentityUser(id);
}

/** Carry out one action on one account. */
export async function perform(action: BulkAction, id: string, banReason: string): Promise<void> {
	switch (action) {
		case 'delete':
			return deleteAccount(id);
		case 'ban':
			return banIdentityUser(id, banReason);
		case 'unban':
			return unbanIdentityUser(id);
		case 'role-user':
			return setIdentityRole(id, 'user');
		case 'role-admin':
			return setIdentityRole(id, 'admin');
		case 'make-operator':
			await api.adminGrantOperator(id);
			return;
		case 'remove-operator':
			await api.adminRevokeOperator(id);
			return;
	}
}

export interface BulkResult {
	id: string;
	email: string;
	/** Null when it worked; otherwise why not, in the server's words. */
	failure: string | null;
}

/**
 * Run `act` on each target in turn, reporting each result as it lands.
 * One at a time rather than all at once, so the identity service is not
 * asked for a hundred bans in the same instant, and the results arrive in
 * the order the operator will read them.
 */
export async function runBulk(
	targets: readonly IdentityUser[],
	act: (target: IdentityUser) => Promise<void>,
	refuse: (target: IdentityUser) => string | null,
	onResult: (result: BulkResult) => void
): Promise<BulkResult[]> {
	const results: BulkResult[] = [];
	for (const target of targets) {
		let failure = refuse(target);
		if (failure === null) {
			try {
				await act(target);
			} catch (error) {
				failure = error instanceof Error && error.message ? error.message : 'It did not work.';
			}
		}
		const result = { id: target.id, email: target.email, failure };
		results.push(result);
		onResult(result);
	}
	return results;
}
