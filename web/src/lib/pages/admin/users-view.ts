// The operator's user list and the impersonation trail, as the pages draw
// them: who a row is, which chips it carries, which filter it falls under,
// and when a typed confirmation is enough to delete an account. Pure, so it
// tests without a component.

import type { AdminUserRow } from '$lib/admin';
import type { AdminConsentSummary, ConsentKind } from '$lib/api';
import type { IdentityUser } from '$lib/auth-client';
import { agoLabel, utcInstant } from '$lib/elapsed';
import type { Tone } from '$lib/StatusPill.svelte';

/** Up to two letters for the avatar: the first letters of the first and last
 *  words of the name, or of the address's local part when there is no name.
 *  Never empty, because an empty circle reads as a broken image. */
export function initials(name: string | null | undefined, email: string): string {
	const words = (name ?? '')
		.trim()
		.split(/\s+/)
		.filter((word) => word.length > 0);
	if (words.length > 0) {
		const first = words[0]!.charAt(0);
		const last = words.length > 1 ? words[words.length - 1]!.charAt(0) : '';
		return (first + last).toUpperCase();
	}
	const local = email.split('@')[0] ?? '';
	const letter = local.match(/[a-z0-9]/i)?.[0];
	return (letter ?? '?').toUpperCase();
}

/** The label a row is known by: its name when it has one, else its address. */
export function displayName(user: Pick<IdentityUser, 'name' | 'email'>): string {
	const name = user.name?.trim() ?? '';
	return name.length > 0 ? name : user.email;
}

export type UserChip = 'operator' | 'admin' | 'banned' | 'unverified' | 'no-app-user';

/** How each chip reads on screen. */
export const CHIP_WORDS: Record<UserChip, { tone: Tone; label: string }> = {
	operator: { tone: 'run', label: 'operator' },
	admin: { tone: 'run', label: 'identity admin' },
	banned: { tone: 'bad', label: 'banned' },
	unverified: { tone: 'warn', label: 'unverified' },
	'no-app-user': { tone: 'soon', label: 'no app user' }
};

/**
 * The status chips a row carries, most consequential first.
 *
 * `operator` is the platform marking (reads across tenants); `admin` is the
 * identity role (bans, impersonates). They are granted separately, so a row
 * can carry either or both. `unverified` is not shown on a banned account:
 * the ban is what matters about it.
 */
export function userChips(row: AdminUserRow): UserChip[] {
	const chips: UserChip[] = [];
	if (row.platform?.operator === true) {
		chips.push('operator');
	}
	if (row.identity.role === 'admin') {
		chips.push('admin');
	}
	if (row.identity.banned === true) {
		chips.push('banned');
	} else if (!row.identity.emailVerified) {
		chips.push('unverified');
	}
	if (row.platform === null) {
		chips.push('no-app-user');
	}
	return chips;
}

export type UserFilter = 'all' | 'unverified' | 'banned' | 'operators' | 'no-app-user';

export const USER_FILTERS: readonly { id: UserFilter; label: string }[] = [
	{ id: 'all', label: 'All' },
	{ id: 'unverified', label: 'Unverified' },
	{ id: 'banned', label: 'Banned' },
	{ id: 'operators', label: 'Operators' },
	{ id: 'no-app-user', label: 'No app user' }
];

export function inFilter(row: AdminUserRow, filter: UserFilter): boolean {
	const chips = userChips(row);
	switch (filter) {
		case 'all':
			return true;
		case 'unverified':
			return chips.includes('unverified');
		case 'banned':
			return chips.includes('banned');
		case 'operators':
			return chips.includes('operator');
		case 'no-app-user':
			return chips.includes('no-app-user');
	}
}

/**
 * Whether a row matches what is typed in the search box, as you type.
 *
 * Case-insensitive, over the name, the address and the organisation's name,
 * every word of the query required somewhere. The identity service's own
 * search runs on submit and matches the address only; this narrows the page
 * already loaded, so "maths co" finds a teacher by her organisation.
 */
export function rowMatches(row: AdminUserRow, query: string): boolean {
	const words = query
		.toLowerCase()
		.split(/\s+/)
		.filter((word) => word.length > 0);
	if (words.length === 0) {
		return true;
	}
	const haystack = [row.identity.name, row.identity.email, row.platform?.organisation.name ?? '']
		.join(' ')
		.toLowerCase();
	return words.every((word) => haystack.includes(word));
}

/**
 * Whether the typed confirmation names this account.
 *
 * Trimmed and compared case-insensitively, because addresses are, and
 * nothing looser: a prefix or a near miss is exactly the slip the typed
 * confirmation exists to catch.
 */
export function deleteConfirmed(typed: string, email: string): boolean {
	const expected = email.trim().toLowerCase();
	return expected.length > 0 && typed.trim().toLowerCase() === expected;
}

/** A subject id cut to something a person can compare at a glance. */
export function shortId(id: string): string {
	return id.length > 8 ? `${id.slice(0, 8)}…` : id;
}

/**
 * Who an identity subject is, in words: "Name (email)", the address alone
 * when there is no name, and the shortened id when the account is not on the
 * loaded identity page — deleted since, or past the listing's bound.
 */
export interface Party {
	label: string;
	known: boolean;
}

export function partyOf(id: string, accounts: ReadonlyMap<string, IdentityUser>): Party {
	const account = accounts.get(id);
	if (account === undefined) {
		return { label: shortId(id), known: false };
	}
	const name = account.name?.trim() ?? '';
	return {
		label: name.length > 0 ? `${name} (${account.email})` : account.email,
		known: true
	};
}

/** When an identity account was made, in epoch milliseconds, or null when
 *  the service did not say or said something unreadable. */
export function joinedAt(user: Pick<IdentityUser, 'createdAt'>): number | null {
	if (user.createdAt === null || user.createdAt === undefined) return null;
	const parsed = new Date(user.createdAt).getTime();
	return Number.isNaN(parsed) ? null : parsed;
}

export type SortKey = 'joined' | 'signin';
export interface UserSort {
	key: SortKey;
	direction: 'asc' | 'desc';
}

/**
 * The rows on screen in last-sign-in order. Within the page only: the
 * identity service pages by when accounts were made, and the sign-in trail
 * lives in the other plane. Rows with no sign-in go last either way, since
 * "never seen" is not the oldest sign-in.
 */
export function bySignIn(rows: readonly AdminUserRow[], direction: 'asc' | 'desc'): AdminUserRow[] {
	const sign = direction === 'asc' ? 1 : -1;
	return [...rows].sort((a, b) => {
		const left = a.platform?.last_sign_in_at;
		const right = b.platform?.last_sign_in_at;
		if (left === undefined || right === undefined) {
			return (left === undefined ? 1 : 0) - (right === undefined ? 1 : 0);
		}
		return sign * (left - right);
	});
}

/** The users page's Consent cell: when this account last agreed to the terms,
 *  and to which version, or that no agreement is on record. */
export type ConsentCell =
	{ agreed: true; label: string; title: string } | { agreed: false; label: 'none' };

export function consentCell(summary: AdminConsentSummary | undefined, now: number): ConsentCell {
	if (summary === undefined) {
		return { agreed: false, label: 'none' };
	}
	return {
		agreed: true,
		label: `✓ ${agoLabel(summary.accepted_at, now)} · v${summary.document_version}`,
		title: utcInstant(summary.accepted_at)
	};
}

/** What each recorded agreement covered, in the words the user sheet shows. */
export const CONSENT_KIND_WORDS: Record<ConsentKind, string> = {
	terms_privacy: 'Terms and Privacy',
	ip_ownership: 'Owns what they publish',
	age_18: '18 or older'
};
