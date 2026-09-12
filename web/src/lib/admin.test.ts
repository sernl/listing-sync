import { describe, expect, it } from 'vitest';
import { ApiFailure, type AdminUserView } from './api';
import type { IdentityUser } from './auth-client';
import {
	barWidth,
	dayLabel,
	identityAdminRefusal,
	impersonationState,
	mergeUsers,
	operatorVerdict,
	outsiderReason,
	sessionWords,
	signInTrailVisible,
	signupPeak,
	signupSeries
} from './admin';

const DAY = 86_400_000;
const MONDAY = Date.UTC(2026, 7, 24);

function failure(status: number, code?: string): ApiFailure {
	return new ApiFailure(status, {
		status,
		errors: [{ message: 'refused', ...(code === undefined ? {} : { code: code as never }) }]
	});
}

describe('the operator probe', () => {
	it('shows nothing while the answer is still outstanding', () => {
		expect(operatorVerdict({ answered: false, failure: null })).toBe('checking');
	});

	it('admits only on an answer, never on the absence of a failure', () => {
		expect(operatorVerdict({ answered: true, failure: null })).toBe('operator');
	});

	it('treats the blank refusal as not-an-operator rather than as a fault', () => {
		expect(operatorVerdict({ answered: false, failure: failure(401) })).toBe('outsider');
	});

	it('hides the surface on any failure, because the refusal is deliberately blank', () => {
		expect(operatorVerdict({ answered: false, failure: new Error('offline') })).toBe('outsider');
	});
});

describe('reading a refusal on the operator surface', () => {
	it('names an unconfigured deployment from the code the server sends', () => {
		expect(outsiderReason(failure(503, 'backoffice_unavailable'))).toBe('unconfigured');
	});

	it('reads a bare 401 as not-an-operator', () => {
		expect(outsiderReason(failure(401))).toBe('not-an-operator');
	});

	it('claims nothing about a failure that is neither', () => {
		expect(outsiderReason(failure(500))).toBe('unreadable');
		expect(outsiderReason(new Error('offline'))).toBe('unreadable');
	});
});

describe('the signup series', () => {
	it('unions the days of both planes, newest first', () => {
		const rows = signupSeries({
			provisioned: [
				{ day: MONDAY, count: 3 },
				{ day: MONDAY - DAY, count: 1 }
			],
			identity: [
				{ day: MONDAY + DAY, count: 5 },
				{ day: MONDAY, count: 4 }
			]
		});
		expect(rows.map((row) => row.day)).toEqual([MONDAY + DAY, MONDAY, MONDAY - DAY]);
	});

	it('counts zero for a day one plane recorded and the other did not', () => {
		const rows = signupSeries({
			provisioned: [{ day: MONDAY, count: 3 }],
			identity: [{ day: MONDAY + DAY, count: 5 }]
		});
		expect(rows).toEqual([
			{ day: MONDAY + DAY, provisioned: 0, identity: 5 },
			{ day: MONDAY, provisioned: 3, identity: 0 }
		]);
	});

	it('reports an absent identity trail as null rather than as zero', () => {
		const rows = signupSeries({ provisioned: [{ day: MONDAY, count: 3 }] });
		expect(rows).toEqual([{ day: MONDAY, provisioned: 3, identity: null }]);
	});

	it('keeps only the newest days when the window is smaller than the series', () => {
		const rows = signupSeries(
			{
				provisioned: [
					{ day: MONDAY, count: 1 },
					{ day: MONDAY - DAY, count: 1 },
					{ day: MONDAY - 2 * DAY, count: 1 }
				]
			},
			2
		);
		expect(rows.map((row) => row.day)).toEqual([MONDAY, MONDAY - DAY]);
	});

	it('labels a day by the UTC date the server aggregated by', () => {
		expect(dayLabel(MONDAY)).toBe('2026-08-24');
	});
});

describe('the bar widths', () => {
	it('takes the peak across both series', () => {
		expect(signupPeak([{ day: MONDAY, provisioned: 2, identity: 9 }])).toBe(9);
		expect(signupPeak([{ day: MONDAY, provisioned: 7, identity: null }])).toBe(7);
	});

	it('is zero for an empty window', () => {
		expect(signupPeak([])).toBe(0);
		expect(barWidth(0, 0)).toBe(0);
	});

	it('draws the peak full width', () => {
		expect(barWidth(9, 9)).toBe(100);
	});

	it('never draws a real count as nothing', () => {
		expect(barWidth(1, 1000)).toBeGreaterThan(0);
	});

	it('draws an absent count as nothing', () => {
		expect(barWidth(0, 9)).toBe(0);
	});
});

describe('the impersonation state', () => {
	const session = (over: Record<string, unknown>, actor?: string | null) => ({
		user: { name: null, email: null, ...over },
		session: { impersonatedBy: actor ?? null }
	});

	it('is absent when nobody is impersonating', () => {
		expect(impersonationState(session({ name: 'Ada' }))).toBeNull();
		expect(impersonationState(null)).toBeNull();
	});

	it('is absent when the actor field is present but empty', () => {
		expect(impersonationState(session({ name: 'Ada' }, ''))).toBeNull();
	});

	it('names the impersonated human when one is being acted as', () => {
		expect(impersonationState(session({ name: 'Ada' }, 'admin-1'))).toEqual({ who: 'Ada' });
	});

	it('falls back to the address rather than showing an empty banner', () => {
		expect(impersonationState(session({ name: '  ', email: 'ada@example.test' }, 'a'))).toEqual({
			who: 'ada@example.test'
		});
	});

	it('still warns when the identity carries neither a name nor an address', () => {
		expect(impersonationState(session({}, 'admin-1'))).toEqual({ who: 'another account' });
	});
});

describe('a refusal from the identity plane', () => {
	it('reads 403 as the missing identity-admin role', () => {
		expect(identityAdminRefusal(403)).toBe('not-identity-admin');
	});

	it('reads 401 the same way, because an unauthenticated admin call is the same gap', () => {
		expect(identityAdminRefusal(401)).toBe('not-identity-admin');
	});

	it('claims nothing about any other status', () => {
		expect(identityAdminRefusal(500)).toBe('unreadable');
		expect(identityAdminRefusal(undefined)).toBe('unreadable');
	});
});

describe('the two planes joined on auth_subject', () => {
	const account = (id: string, email: string): IdentityUser => ({
		id,
		email,
		name: email,
		emailVerified: true
	});

	const appUser = (user: string, subject?: string): AdminUserView => ({
		user,
		email: `${user}@example.test`,
		...(subject === undefined ? {} : { auth_subject: subject }),
		organisation: { org: `org-${user}`, name: `Org ${user}`, slug: user },
		plan: 'free',
		created_at: 0
	});

	it('attaches each app user to the account whose subject it carries', () => {
		const merged = mergeUsers(
			[account('s1', 'ada@example.test'), account('s2', 'bea@example.test')],
			[appUser('b', 's2'), appUser('a', 's1')]
		);
		expect(merged.rows.map((row) => [row.identity.id, row.platform?.user])).toEqual([
			['s1', 'a'],
			['s2', 'b']
		]);
		expect(merged.unlinked).toBe(0);
	});

	it('never matches an app user carrying no subject', () => {
		// The key is absent on that row, which is a user provisioned without an
		// identity subject. Matching it to whichever account came next would
		// put somebody else's organisation and plan on that row.
		const merged = mergeUsers([account('s1', 'ada@example.test')], [appUser('a')]);
		expect(merged.rows[0]?.platform).toBeNull();
		expect(merged.unlinked).toBe(1);
	});

	it('never matches a subject the server sent as an explicit null either', () => {
		// Belt and braces on the guard rather than on the schema: a null here
		// would become a map key and match an account whose id is missing, and
		// that row would be somebody else's organisation.
		const nulled = { ...appUser('a', 's1'), auth_subject: null } as unknown as AdminUserView;
		const merged = mergeUsers([account('s1', 'ada@example.test')], [nulled]);
		expect(merged.rows[0]?.platform).toBeNull();
		expect(merged.unlinked).toBe(1);
	});

	it('leaves an account with no app user of its own unattached', () => {
		const merged = mergeUsers([account('s1', 'ada@example.test')], []);
		expect(merged.rows).toEqual([{ identity: account('s1', 'ada@example.test'), platform: null }]);
		expect(merged.unlinked).toBe(0);
	});

	it('counts app users the listing does not show, which a narrowed search is full of', () => {
		const merged = mergeUsers(
			[account('s1', 'ada@example.test')],
			[appUser('a', 's1'), appUser('b', 's2'), appUser('c')]
		);
		expect(merged.rows[0]?.platform?.user).toBe('a');
		expect(merged.unlinked).toBe(2);
	});

	it('keeps the first of two app users claiming one subject', () => {
		// A broken unique index rather than a case to resolve, so the row is
		// drawn from one of them and the other is reported rather than merged
		// over the top.
		const merged = mergeUsers(
			[account('s1', 'ada@example.test')],
			[appUser('a', 's1'), appUser('b', 's1')]
		);
		expect(merged.rows[0]?.platform?.user).toBe('a');
		expect(merged.unlinked).toBe(1);
	});
});

describe('the sign-ins column', () => {
	it('says a count has not been read, which is not the same as none', () => {
		expect(sessionWords(null)).toBe('not read');
		expect(sessionWords(0)).toBe('none');
	});

	it('counts in words that agree with the figure', () => {
		expect(sessionWords(1)).toBe('1 sign-in');
		expect(sessionWords(4)).toBe('4 sign-ins');
	});
});

describe('the sign-in trail', () => {
	const user = (at?: number): AdminUserView => ({
		user: 'a',
		email: 'a@example.test',
		organisation: { org: 'o', name: 'O' },
		plan: 'free',
		...(at === undefined ? {} : { last_sign_in_at: at }),
		created_at: 0
	});

	it('is visible as soon as one row carries a sign-in', () => {
		expect(signInTrailVisible([user(), user(1)])).toBe(true);
	});

	it('is not claimed visible by a page with no sign-in on it, which is what an unreadable schema looks like', () => {
		expect(signInTrailVisible([user(), user()])).toBe(false);
		expect(signInTrailVisible([])).toBe(false);
	});
});
