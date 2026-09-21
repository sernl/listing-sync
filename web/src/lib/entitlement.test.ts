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
	movesLimit,
	movesLine,
	movesReason,
	packEditLine,
	sectionAllowed,
	sectionReason,
	supportLabel,
	usageLine,
	usageLines
} from './entitlement';

/** The real capability set for one plan. Read off the generated table rather
 *  than retyped, because the whole point of the plan table is that these
 *  figures have one definition; a fixture here would be a second one, and the
 *  drift it hid is exactly the drift that work removed. */
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
		templates: 0,
		collections: 0,
		labels: 0,
		devices: 0,
		...over
	};
}

function grant(over: Partial<Grant> = {}): Grant {
	return {
		plan: 'subscriber',
		rung: null,
		granted_by: 'stripe',
		granted_at: Date.UTC(2026, 8, 1),
		expires_at: null,
		...over
	};
}

describe('which sections a plan reaches', () => {
	it('lets a subscriber everywhere', () => {
		const subscriber = caps('subscriber');
		for (const section of [
			'import',
			'crosslist',
			'automations',
			'marketplaces',
			'account'
		] as const) {
			expect(sectionAllowed(subscriber, section)).toBe(true);
		}
	});

	// The free plan schedules nothing and pulls nothing, but it is handed
	// moves to spend: the section holds a page it can act in, so gating it out
	// would refuse the seller the moves the plan gave them.
	it('lets the free plan into Automations for the moves it holds', () => {
		expect(caps('free').scheduling).toBe(false);
		expect(caps('free').sync_pull_interval_secs).toBeNull();
		expect(caps('free').free_moves_lifetime).toBeGreaterThan(0);
		expect(sectionAllowed(caps('free'), 'automations')).toBe(true);
	});

	// A plan that neither schedules, pulls nor is handed a move has three dead
	// pages in the section, and says so rather than showing them.
	it('withholds Automations from a plan that carries none of it', () => {
		const barren: Capabilities = {
			...caps('free'),
			free_moves_lifetime: 0
		};
		expect(sectionAllowed(barren, 'automations')).toBe(false);
		expect(sectionReason(barren, 'automations')).toContain('Upgrade to schedule');
	});

	// Account is how a plan is bought, so it can never be the thing a plan
	// withholds; Admin answers to the operator probe and not to a plan.
	it('never withholds Account or Admin', () => {
		for (const plan of PLANS) {
			expect(sectionAllowed(plan.capabilities, 'account')).toBe(true);
			expect(sectionAllowed(plan.capabilities, 'admin')).toBe(true);
		}
	});

	// Export lives inside Crosslist and export is never gated: a seller who
	// cannot get their catalogue out will not put one in.
	it('lets every plan into Crosslist', () => {
		for (const plan of PLANS) {
			expect(sectionAllowed(plan.capabilities, 'crosslist')).toBe(true);
			expect(plan.capabilities.export).toBe(true);
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
		expect(featureReason(caps('free'), 'scheduling')).toContain('Upgrade to publish on a timetable');
	});

	// Null rather than an interval: a plan that never pulls is not a plan that
	// pulls every zero seconds.
	it('reads a null pull interval as never', () => {
		expect(caps('free').sync_pull_interval_secs).toBeNull();
		expect(featureReason(caps('free'), 'sync')).not.toBeNull();
	});
});

describe('why a counted allowance is full', () => {
	it('is nothing while one more would fit', () => {
		expect(limitReason(caps('free'), usage({ resources: 499 }), 'resources')).toBeNull();
	});

	// Drawn before the write, so the question is whether one more fits — not
	// whether the last one did.
	it('refuses at the cap rather than past it', () => {
		expect(limitReason(caps('free'), usage({ resources: 500 }), 'resources')).toBe(
			'Your plan includes 500 resources. Upgrade to add more.'
		);
	});

	it('agrees its noun with the figure, and names the act', () => {
		expect(limitReason(caps('free'), usage({ templates: 1 }), 'templates')).toBe(
			'Your plan includes 1 template. Upgrade to add more.'
		);
	});

	// The free plan has no collections of its own, and the sentence says zero
	// rather than implying a cap the seller could reach.
	it('states a zero allowance as zero', () => {
		expect(limitReason(caps('free'), usage(), 'collections')).toBe(
			'Your plan includes 0 collections. Upgrade to add more.'
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
		const rows = usageLines(usage({ resources: 500, labels: 2 }), caps('free'));
		expect(rows.map((row) => row.limit)).toEqual([
			'resources',
			'marketplaces',
			'templates',
			'collections',
			'labels',
			'devices'
		]);
		expect(rows.find((row) => row.limit === 'resources')?.full).toBe(true);
		expect(rows.find((row) => row.limit === 'labels')?.full).toBe(false);
	});
});

describe('the balance a seller holds', () => {
	it('states the figure, agreeing its noun', () => {
		expect(movesLine({ available: 12 })).toBe('You have 12 moves.');
		expect(movesLine({ available: 1 })).toBe('You have 1 move.');
	});

	// "0 moves" reads as a fault in the count rather than as an empty purse.
	it('reads an empty balance as none rather than as a zero', () => {
		expect(movesLine({ available: 0 })).toBe('You have no moves.');
	});
});

describe('the balance a selection is checked against', () => {
	it('names the selection beside the balance', () => {
		const standing = movesReason({ available: 12 }, 8);
		expect(standing.line).toBe('You have 12 moves. This uses 8.');
		expect(standing.refusal).toBeNull();
	});

	it('leaves out the selection clause where nothing is selected yet', () => {
		expect(movesReason({ available: 12 }, 0).line).toBe('You have 12 moves.');
	});

	it('lets a selection spend the balance exactly', () => {
		expect(movesReason({ available: 12 }, 12).refusal).toBeNull();
	});

	// Two acts, so two sentences: a seller with some moves may pick fewer, and
	// a seller with none can only buy.
	it('sends a short balance to buy or to pick fewer', () => {
		expect(movesReason({ available: 12 }, 13).refusal).toBe(
			'You have 12 moves and this needs 13. Buy a pack or pick fewer.'
		);
	});

	it('sends an empty balance to a pack or to Sync', () => {
		const none = movesReason({ available: 0 }, 0);
		expect(none.line).toBe('You have no moves.');
		expect(none.refusal).toBe('You have no moves left. Buy a pack or choose Sync.');
	});

	// The confirm is drawn before the submit, so a balance of nothing refuses
	// even where the seller has selected nothing yet.
	it('refuses an empty balance before anything is selected', () => {
		expect(movesReason({ available: 0 }, 3).refusal).toBe(
			'You have no moves left. Buy a pack or choose Sync.'
		);
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

	// The provider's own record is already on the page; a notice over it would
	// be a second account of one fact.
	it('is nothing for a plan that arrived through checkout, or through nothing', () => {
		expect(grantNotice(grant())).toBeNull();
		expect(grantNotice(grant({ plan: 'free', granted_by: null }))).toBeNull();
	});
});

describe('what a plan card lists', () => {
	it('reads the plan table for the subscription', () => {
		const lines = capabilityLines(caps('subscriber'));
		expect(lines).toContain('Unlimited resources');
		expect(lines).toContain('All marketplaces');
		expect(lines).toContain('Import from a spreadsheet or a marketplace');
		expect(lines).toContain('25 moves a month, saving up to 75');
		expect(lines).toContain('Sync every 6 hours');
		expect(lines).toContain('Analytics');
		expect(lines).toContain('200 AI auto-fills a month, once AI arrives');
		expect(lines).toContain('Email support, two business days');
	});

	// Only what the plan carries: a card that also listed what it withholds
	// would be a comparison table with one column.
	it('leaves out what a plan does not carry', () => {
		const lines = capabilityLines(caps('free'));
		expect(lines).not.toContain('Analytics');
		expect(lines.some((line) => line.includes('Sync every'))).toBe(false);
		expect(lines.some((line) => line.includes('a month, saving up to'))).toBe(false);
	});

	// Export is on every plan, and it is the one line that has to be there:
	// a seller who cannot get their catalogue out will not put one in.
	it('lists export on every plan', () => {
		for (const plan of PLANS) {
			expect(capabilityLines(plan.capabilities)).toContain('Spreadsheet export');
		}
	});

	it('states the editing window every plan gives a moved listing', () => {
		expect(packEditLine(caps('free'))).toBe('Moved listings can be edited for 90 days.');
		for (const plan of PLANS) {
			expect(capabilityLines(plan.capabilities)).toContain(packEditLine(plan.capabilities));
		}
	});
});

describe('what a plan hands out in moves', () => {
	// The ceiling is part of the offer: a month's moves that lapsed the moment
	// the next month landed would be a smaller promise than the one made.
	it('states the monthly run with what it saves up to', () => {
		expect(movesLimit(caps('subscriber'))).toBe('25 moves a month, saving up to 75');
	});

	it('states the free plan’s handful as a one-off rather than as a month', () => {
		expect(movesLimit(caps('free'))).toBe('5 moves to start');
	});

	it('is nothing where every move has to be bought', () => {
		expect(
			movesLimit({ ...caps('free'), moves_per_month: 0, free_moves_lifetime: 0 })
		).toBeNull();
	});
});

describe('what a level of support promises', () => {
	it('reads out each level the plan table uses', () => {
		expect(supportLabel('guides')).toBe('Guides');
		expect(supportLabel('email_2_days')).toBe('Email support, two business days');
		expect(supportLabel('email_1_day')).toBe('Email support, one business day');
	});
});
