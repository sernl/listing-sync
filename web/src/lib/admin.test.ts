import { describe, expect, it } from 'vitest';
import { ApiFailure } from './api';
import {
	barWidth,
	dayLabel,
	identityAdminRefusal,
	impersonationState,
	operatorVerdict,
	outsiderReason,
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
