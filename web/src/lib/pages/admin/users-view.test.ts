import { describe, expect, it } from 'vitest';
import type { AdminUserRow } from '$lib/admin';
import type { AdminUserView } from '$lib/api';
import type { IdentityUser } from '$lib/auth-client';
import {
	CONSENT_KIND_WORDS,
	consentCell,
	deleteConfirmed,
	inFilter,
	initials,
	partyOf,
	rowMatches,
	userChips
} from './users-view';

const account = (over: Partial<IdentityUser> = {}): IdentityUser => ({
	id: 'id-1',
	email: 'ana@example.test',
	name: 'Ana Ruiz',
	emailVerified: true,
	role: 'user',
	banned: false,
	...over
});

const platform = (over: Partial<AdminUserView> = {}): AdminUserView => ({
	user: 'u-1',
	email: 'ana@example.test',
	auth_subject: 'id-1',
	organisation: { org: 'o-1', name: 'Maths Corner' },
	plan: 'free',
	created_at: 0,
	operator: false,
	...over
});

const row = (
	identity: Partial<IdentityUser> = {},
	app: AdminUserView | null = platform()
): AdminUserRow => ({
	identity: account(identity),
	platform: app
});

describe('avatar initials', () => {
	it('takes the first and last word of the name', () => {
		expect(initials('Ana María Ruiz', 'x@example.test')).toBe('AR');
	});

	it('takes one letter from a single-word name', () => {
		expect(initials('  ana  ', 'x@example.test')).toBe('A');
	});

	it('falls back to the address when there is no name', () => {
		expect(initials('', '9lives@example.test')).toBe('9');
		expect(initials(null, '_bob@example.test')).toBe('B');
	});

	it('is never empty', () => {
		expect(initials('', '@example.test')).toBe('?');
	});
});

describe('status chips', () => {
	it('shows nothing for an ordinary verified seller', () => {
		expect(userChips(row())).toEqual([]);
	});

	it('names the operator marking and the identity admin role separately', () => {
		expect(userChips(row({ role: 'admin' }, platform({ operator: true })))).toEqual([
			'operator',
			'admin'
		]);
	});

	it('does not call a banned account unverified as well', () => {
		expect(userChips(row({ banned: true, emailVerified: false }))).toEqual(['banned']);
	});

	it('flags an unverified account and one that never reached the app', () => {
		expect(userChips(row({ emailVerified: false }, null))).toEqual(['unverified', 'no-app-user']);
	});
});

describe('filter chips', () => {
	const rows = [
		row({ id: 'a' }),
		row({ id: 'b', emailVerified: false }),
		row({ id: 'c', banned: true, emailVerified: false }),
		row({ id: 'd' }, platform({ operator: true })),
		row({ id: 'e' }, null)
	];
	const ids = (filter: Parameters<typeof inFilter>[1]) =>
		rows.filter((each) => inFilter(each, filter)).map((each) => each.identity.id);

	it('sorts each row under the chip its status names', () => {
		expect(ids('all')).toEqual(['a', 'b', 'c', 'd', 'e']);
		expect(ids('unverified')).toEqual(['b']);
		expect(ids('banned')).toEqual(['c']);
		expect(ids('operators')).toEqual(['d']);
		expect(ids('no-app-user')).toEqual(['e']);
	});
});

describe('searching as you type', () => {
	it('matches the name, the address or the organisation, ignoring case', () => {
		expect(rowMatches(row(), 'RUIZ')).toBe(true);
		expect(rowMatches(row(), 'ana@')).toBe(true);
		expect(rowMatches(row(), 'corner')).toBe(true);
	});

	it('needs every word somewhere, not together', () => {
		expect(rowMatches(row(), 'ana maths')).toBe(true);
		expect(rowMatches(row(), 'ana science')).toBe(false);
	});

	it('matches everything when the box is blank', () => {
		expect(rowMatches(row(), '   ')).toBe(true);
	});

	it('does not match an organisation a row does not have', () => {
		expect(rowMatches(row({}, null), 'maths')).toBe(false);
	});
});

describe('typed delete confirmation', () => {
	it('accepts the address with stray spaces and in any case', () => {
		expect(deleteConfirmed('  Ana@Example.test ', 'ana@example.test')).toBe(true);
	});

	it('refuses a prefix, a near miss or nothing', () => {
		expect(deleteConfirmed('ana@example', 'ana@example.test')).toBe(false);
		expect(deleteConfirmed('ana@example.tset', 'ana@example.test')).toBe(false);
		expect(deleteConfirmed('', 'ana@example.test')).toBe(false);
	});

	it('never confirms against an empty address', () => {
		expect(deleteConfirmed('', '')).toBe(false);
	});
});

describe('naming a party in the impersonation trail', () => {
	const accounts = new Map([
		['id-1', account()],
		['id-2', account({ id: 'id-2', name: ' ', email: 'bo@example.test' })]
	]);

	it('reads "name (email)" for a known account', () => {
		expect(partyOf('id-1', accounts)).toEqual({
			label: 'Ana Ruiz (ana@example.test)',
			known: true
		});
	});

	it('reads the address alone when the account has no name', () => {
		expect(partyOf('id-2', accounts).label).toBe('bo@example.test');
	});

	it('falls back to a shortened id for an account not on the page', () => {
		expect(partyOf('0f3a9c21-aaaa-bbbb-cccc-000000000000', accounts)).toEqual({
			label: '0f3a9c21…',
			known: false
		});
	});
});

describe('the consent column', () => {
	const DAY = 24 * 60 * 60 * 1000;
	const accepted = Date.UTC(2026, 9, 4, 9, 30);

	it('reads none when no agreement is on record', () => {
		expect(consentCell(undefined, accepted)).toEqual({ agreed: false, label: 'none' });
	});

	it('reads how long ago and which version, with the exact instant as its title', () => {
		expect(
			consentCell(
				{ subject: 'id-1', document_version: '2026-10-04', accepted_at: accepted },
				accepted + 3 * DAY
			)
		).toEqual({
			agreed: true,
			label: '✓ 3 days ago · v2026-10-04',
			title: '2026-10-04 09:30:00Z'
		});
	});

	it('names every kind of agreement in words', () => {
		expect(CONSENT_KIND_WORDS).toEqual({
			terms_privacy: 'Terms and Privacy',
			ip_ownership: 'Owns what they publish',
			age_18: '18 or older'
		});
	});
});
