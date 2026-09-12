import { describe, expect, it } from 'vitest';
import type { Capabilities, EntitlementUsage, Grant } from '$lib/api';
import { PLANS } from '$lib/generated/plans';
import type { Plan } from '$lib/generated/vocab';
import {
	UNLIMITED,
	capabilityLines,
	dayMonth,
	featureReason,
	grantNotice,
	limitReason,
	migrationsLine,
	migrationsReason,
	sectionAllowed,
	sectionReason,
	usageLine,
	usageLines
} from './entitlement';

/** The real capability set for one plan. Read off the generated table rather
 *  than retyped, because the whole point of phase 1 is that these figures
 *  have one definition; a fixture here would be a second one, and the drift
 *  it hid is exactly the drift this work removed. */
function caps(plan: Plan): Capabilities {
	const row = PLANS.find((entry) => entry.id === plan);
	if (row === undefined) {
		throw new Error(`no ${plan} row in the generated plan table`);
	}
	return row.capabilities;
}

function usage(over: Partial<EntitlementUsage> = {}): EntitlementUsage {
	return {
		resources: 0,
		marketplaces: 0,
		migrations_this_month: 0,
		// 1 October 2026, 00:00 UTC.
		migrations_reset_at: Date.UTC(2026, 9, 1),
		templates: 0,
		labels: 0,
		devices: 0,
		...over
	};
}

function grant(over: Partial<Grant> = {}): Grant {
	return {
		plan: 'subscriber',
		rung: null,
		granted_by: 'paddle',
		granted_at: Date.UTC(2026, 8, 1),
		expires_at: null,
		...over
	};
}

describe('which sections a plan reaches', () => {
	it('lets a subscriber everywhere', () => {
		const subscriber = caps('subscriber');
		for (const section of ['import', 'crosslist', 'automations', 'marketplaces', 'account'] as const) {
			expect(sectionAllowed(subscriber, section)).toBe(true);
		}
	});

	// The free plan migrates nothing, schedules nothing and pulls nothing, so
	// the section is three dead pages rather than one the seller can act in.
	it('withholds Automations from the free plan, with a sentence', () => {
		expect(sectionAllowed(caps('free'), 'automations')).toBe(false);
		expect(sectionReason(caps('free'), 'automations')).toContain('Upgrade to schedule');
	});

	// The product sold at $47–$397 is called Catalogue Import and lives in the
	// Import section, so a one-off buyer gated out of it would be gated out of
	// the thing they bought.
	it('lets a one-off buyer reach Import and Export', () => {
		const bought = caps('migration_only');
		expect(sectionAllowed(bought, 'import')).toBe(true);
		expect(bought.export).toBe(true);
		expect(sectionAllowed(bought, 'crosslist')).toBe(true);
	});

	// Account is how a plan is bought, so it can never be the thing a plan
	// withholds; Admin answers to the operator probe and not to a plan.
	it('never withholds Account or Admin', () => {
		for (const plan of PLANS) {
			expect(sectionAllowed(plan.capabilities, 'account')).toBe(true);
			expect(sectionAllowed(plan.capabilities, 'admin')).toBe(true);
		}
	});
});

describe('why a control is disabled', () => {
	it('is nothing where the plan carries the capability', () => {
		expect(featureReason(caps('subscriber'), 'analytics')).toBeNull();
		expect(featureReason(caps('subscriber'), 'import_marketplace')).toBeNull();
		expect(featureReason(caps('subscriber'), 'sync')).toBeNull();
	});

	it('names the capability and what to do about it where it does not', () => {
		expect(featureReason(caps('free'), 'analytics')).toBe(
			'Your plan does not include analytics. Upgrade to see how your listings are doing.'
		);
		expect(featureReason(caps('free'), 'import_marketplace')).toContain('read your shop');
		expect(featureReason(caps('free'), 'scheduling')).toContain('Upgrade to publish on a timetable');
	});

	// The free plan reads a spreadsheet and cannot read a shop, so the two
	// import controls are not one gate.
	it('separates the two ways in', () => {
		expect(featureReason(caps('free'), 'import_spreadsheet')).toBeNull();
		expect(featureReason(caps('free'), 'import_marketplace')).not.toBeNull();
	});

	// Null rather than an interval: a plan that never pulls is not a plan that
	// pulls every zero seconds.
	it('reads a null pull interval as never', () => {
		expect(featureReason(caps('migration_only'), 'sync')).not.toBeNull();
	});
});

describe('why a counted allowance is full', () => {
	it('is nothing while one more would fit', () => {
		expect(limitReason(caps('free'), usage({ resources: 19 }), 'resources')).toBeNull();
	});

	// Drawn before the write, so the question is whether one more fits — not
	// whether the last one did.
	it('refuses at the cap rather than past it', () => {
		expect(limitReason(caps('free'), usage({ resources: 20 }), 'resources')).toBe(
			'Your plan includes 20 resources. Upgrade to add more.'
		);
	});

	it('agrees its noun with the figure, and names the act', () => {
		expect(limitReason(caps('free'), usage({ marketplaces: 1 }), 'marketplaces')).toBe(
			'Your plan includes 1 marketplace. Upgrade to connect more.'
		);
		expect(limitReason(caps('free'), usage({ devices: 1 }), 'devices')).toBe(
			'Your plan includes 1 device. Upgrade to sign in on more.'
		);
	});

	// A one-off buyer's own labels are the marketplaces', which the server
	// excludes from the count; the seller's own vocabulary is zero and the
	// sentence says so rather than implying a cap they could reach.
	it('states a zero allowance as zero', () => {
		expect(limitReason(caps('migration_only'), usage(), 'labels')).toBe(
			'Your plan includes 0 labels. Upgrade to add more.'
		);
	});

	it('never refuses an unlimited allowance', () => {
		expect(limitReason(caps('subscriber'), usage({ marketplaces: 99 }), 'marketplaces')).toBeNull();
		expect(caps('subscriber').marketplaces_max).toBe(UNLIMITED);
	});
});

describe('the allowance lines the Account page reads out', () => {
	it('states the count against the cap', () => {
		expect(usageLine(12, 20, 'resources')).toBe('12 of 20 resources');
	});

	// `4294967295` is the sentinel and not a promise, so it is never printed.
	it('says an unlimited allowance is unlimited rather than printing the sentinel', () => {
		expect(usageLine(3, UNLIMITED, 'marketplaces')).toBe(
			'3 marketplaces, with no limit on your plan'
		);
	});

	it('lists every counted allowance, marking the full ones', () => {
		const rows = usageLines(usage({ resources: 20, labels: 2 }), caps('free'));
		expect(rows.map((row) => row.limit)).toEqual([
			'resources',
			'marketplaces',
			'templates',
			'labels',
			'devices'
		]);
		expect(rows.find((row) => row.limit === 'resources')?.full).toBe(true);
		expect(rows.find((row) => row.limit === 'labels')?.full).toBe(false);
	});
});

describe('the migration line', () => {
	it('states the month and when it resets', () => {
		expect(migrationsLine(usage({ migrations_this_month: 8 }), caps('subscriber'))).toBe(
			'8 of 20 this month; resets 1 October'
		);
	});

	// A plan that migrates nothing has no month to reset, so "0 of 0 this
	// month" would be an allowance the seller could wait for.
	it('says a plan moves nothing rather than counting to zero', () => {
		expect(migrationsLine(usage(), caps('free'))).toBe(
			'Your plan moves no resources between marketplaces.'
		);
	});
});

describe('the allowance a selection is checked against', () => {
	it('counts what is left rather than what is spent, and names the selection', () => {
		const standing = migrationsReason(
			caps('subscriber'),
			usage({ migrations_this_month: 8 }),
			8
		);
		expect(standing.line).toBe(
			'You have 12 of 20 moves left this month; this uses 8. Resets 1 October.'
		);
		expect(standing.refusal).toBeNull();
	});

	it('lets a selection fill the allowance exactly', () => {
		expect(
			migrationsReason(caps('subscriber'), usage({ migrations_this_month: 8 }), 12).refusal
		).toBeNull();
	});

	it('refuses one past it, saying how many would fit and when the rest can go', () => {
		const over = migrationsReason(caps('subscriber'), usage({ migrations_this_month: 8 }), 13);
		expect(over.refusal).toBe(
			'Your plan moves 20 resources a month and you have 12 left, so 13 is more than this month can take. It resets 1 October.'
		);
	});

	// A count the server has already moved past the limit is "none left", not a
	// negative allowance the sentence would print as "-3 of 20".
	it('never counts below nothing left', () => {
		const spent = migrationsReason(caps('subscriber'), usage({ migrations_this_month: 25 }), 1);
		expect(spent.line).toContain('0 of 20');
	});

	it('refuses a plan that moves nothing before counting anything', () => {
		const none = migrationsReason(caps('free'), usage(), 0);
		expect(none.line).toBe('Your plan moves no resources between marketplaces.');
		expect(none.refusal).toContain('Upgrade to migrate.');
	});
});

describe('a date in a sentence', () => {
	// Read in UTC and spelled out, so the figures in the page read the same
	// in every browser rather than flipping to month-first for an American
	// locale.
	it('reads as a day and its month, in UTC', () => {
		expect(dayMonth(Date.UTC(2026, 11, 1))).toBe('1 December');
		expect(dayMonth(Date.UTC(2026, 11, 1, 23, 59))).toBe('1 December');
	});
});

describe('the notice over a plan a person set', () => {
	it('names Teachouse and the date it ends', () => {
		expect(grantNotice(grant({ granted_by: 'operator', expires_at: Date.UTC(2026, 11, 1) }))).toBe(
			'Set by Teachouse until 1 December.'
		);
	});

	it('says so where a manual grant has no end', () => {
		expect(grantNotice(grant({ granted_by: 'operator' }))).toBe(
			'Set by Teachouse, with no end date.'
		);
	});

	// Paddle's own record is already on the page; a notice over it would be a
	// second account of one fact.
	it('is nothing for a plan that arrived through checkout, or through nothing', () => {
		expect(grantNotice(grant())).toBeNull();
		expect(grantNotice(grant({ plan: 'free', granted_by: null }))).toBeNull();
	});
});

describe('what a plan card lists', () => {
	it('reads the founder’s own table for the subscription', () => {
		const lines = capabilityLines(caps('subscriber'));
		expect(lines).toContain('400 resources');
		expect(lines).toContain('All marketplaces');
		expect(lines).toContain('Import from a spreadsheet or a marketplace');
		expect(lines).toContain('20 resources copied or moved a month');
		expect(lines).toContain('Sync every 6 hours');
		expect(lines).toContain('Analytics');
		expect(lines).toContain('2 devices');
		expect(lines).toContain('200 AI auto-fills a month, once AI arrives');
		expect(lines).toContain('Email support, two business days');
	});

	// Only what the plan carries: a card that also listed what it withholds
	// would be a comparison table with one column.
	it('leaves out what a plan does not carry', () => {
		const lines = capabilityLines(caps('free'));
		expect(lines).toContain('Import from a spreadsheet');
		expect(lines).not.toContain('Analytics');
		expect(lines.some((line) => line.includes('Sync every'))).toBe(false);
		expect(lines.some((line) => line.includes('copied or moved'))).toBe(false);
	});

	it('states the one-off plan’s editing window, which is the thing that lapses', () => {
		expect(capabilityLines(caps('migration_only'))).toContain(
			'Edit and delete your listings for 30 days'
		);
	});

	// Export is on every plan, and it is the one line that has to be there:
	// a seller who cannot get their catalogue out will not put one in.
	it('lists export on every plan', () => {
		for (const plan of PLANS) {
			expect(capabilityLines(plan.capabilities)).toContain('Spreadsheet export');
		}
	});
});
