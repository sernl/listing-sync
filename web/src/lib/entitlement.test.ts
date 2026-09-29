import { describe, expect, it } from 'vitest';
import type { Capabilities, EntitlementUsage, Grant } from '$lib/api';
import { PLANS } from '$lib/generated/plans';
import type { Plan } from '$lib/generated/vocab';
import {
	UNLIMITED,
	dayMonth,
	featureReason,
	grantNotice,
	limitReason,
	movesLine,
	movesReason,
	sectionAllowed,
	sectionReason,
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
		storage_bytes: 0,
		previews: 0,
		ai_fills: 0,
		month_resets_at: Date.UTC(2026, 9, 1),
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
		expect(sectionReason(barren, 'automations')).toContain('Upgrade your plan to schedule');
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
			'Upgrade your plan to see how your listings are doing.'
		);
		expect(featureReason(caps('free'), 'scheduling')).toContain('Upgrade your plan to schedule when things publish');
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
		const max = caps('free').resources_max;
		expect(limitReason(caps('free'), usage({ resources: max - 1 }), 'resources')).toBeNull();
	});

	// Drawn before the write, so the question is whether one more fits — not
	// whether the last one did.
	it('refuses at the cap rather than past it', () => {
		expect(limitReason(caps('free'), usage({ resources: 100 }), 'resources')).toBe(
			'Your plan includes 100 resources. Upgrade to add more.'
		);
	});

	// Grandfathered: a seller over a ceiling that was lowered keeps what they
	// have and is refused the next one, in the same words.
	it('refuses a seller already over the ceiling', () => {
		expect(limitReason(caps('free'), usage({ resources: 340 }), 'resources')).toBe(
			'Your plan includes 100 resources. Upgrade to add more.'
		);
	});

	// A monthly allowance comes back on the first, so its sentence says the
	// seller can wait as well as upgrade, as the server's refusal does.
	it('says a monthly allowance renews', () => {
		const max = caps('free').previews_per_month;
		expect(limitReason(caps('free'), usage({ previews: max - 1 }), 'previews')).toBeNull();
		expect(limitReason(caps('free'), usage({ previews: max }), 'previews')).toBe(
			`Your plan includes ${max} watermarked previews a month. Upgrade to make more, or wait until next month.`
		);
		expect(
			limitReason({ ...caps('free'), previews_per_month: 1 }, usage({ previews: 1 }), 'previews')
		).toBe(
			'Your plan includes 1 watermarked preview a month. Upgrade to make more, or wait until next month.'
		);
	});

	it('never refuses previews on a plan with no monthly ceiling', () => {
		expect(caps('studio').previews_per_month).toBe(UNLIMITED);
		expect(limitReason(caps('studio'), usage({ previews: 10_000 }), 'previews')).toBeNull();
	});

	it('agrees its noun with the figure, and names the act', () => {
		expect(limitReason(caps('free'), usage({ templates: 1 }), 'templates')).toBe(
			'Your plan includes 1 template. Upgrade to add more.'
		);
	});

	// A plan with none of something says zero rather than implying a cap the
	// seller could reach. No shipped plan is at zero any more (Look has one
	// collection since 0.15.0), so the figure is set here.
	it('states a zero allowance as zero', () => {
		expect(
			limitReason({ ...caps('free'), collections_max: 0 }, usage(), 'collections')
		).toBe('Your plan includes 0 collections. Upgrade to add more.');
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
			'3 marketplaces, no limit'
		);
	});

	it('says a monthly count is this month’s', () => {
		expect(usageLine(3, 5, 'previews')).toBe('3 of 5 watermarked previews this month');
		expect(usageLine(40, UNLIMITED, 'previews')).toBe(
			'40 watermarked previews this month, no limit'
		);
	});

	it('lists every counted allowance, marking the full ones', () => {
		const rows = usageLines(usage({ resources: 100, labels: 2, previews: 5 }), caps('free'));
		expect(rows.map((row) => row.limit)).toEqual([
			'resources',
			'marketplaces',
			'templates',
			'collections',
			'labels',
			'devices',
			'previews'
		]);
		expect(rows.find((row) => row.limit === 'resources')?.full).toBe(true);
		expect(rows.find((row) => row.limit === 'labels')?.full).toBe(false);
		expect(rows.find((row) => row.limit === 'previews')?.full).toBe(true);
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
		expect(none.refusal).toBe('You have no moves left. Buy a pack, or choose Sync.');
	});

	// The confirm is drawn before the submit, so a balance of nothing refuses
	// even where the seller has selected nothing yet.
	it('refuses an empty balance before anything is selected', () => {
		expect(movesReason({ available: 0 }, 3).refusal).toBe(
			'You have no moves left. Buy a pack, or choose Sync.'
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
			'Teachouse gave you this plan until 1 December.'
		);
	});

	it('says so where a manual grant has no end', () => {
		expect(grantNotice(grant({ granted_by: 'operator' }))).toBe(
			'Teachouse gave you this plan, with no end date.'
		);
	});

	// The provider's own record is already on the page; a notice over it would
	// be a second account of one fact.
	it('is nothing for a plan that arrived through checkout, or through nothing', () => {
		expect(grantNotice(grant())).toBeNull();
		expect(grantNotice(grant({ plan: 'free', granted_by: null }))).toBeNull();
	});
});
