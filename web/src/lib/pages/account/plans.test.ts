import { describe, expect, it } from 'vitest';
import type { Capabilities, EntitlementView, PlanRow } from '$lib/api';
import {
	checkoutLabel,
	dollars,
	monthsFreeOnAnnual,
	periodLabel,
	priceOf,
	rungLabel,
	soldPlans,
	standingFor,
	type EntitlementRead
} from './plans';

// The price table is `tam-limits`' and arrives generated, so these fixtures
// stand in for it: what is under test here is the rendering, and a test that
// read the real table would be asserting the founder's prices twice — once
// here and once in Rust, where the invariants already live.
const CAPS: Capabilities = {
	resources_max: 400,
	marketplaces_max: 4294967295,
	storage_bytes_max: 21474836480,
	import_spreadsheet: true,
	import_marketplace: true,
	duplicate_review: true,
	publish_marketplaces_max: 4294967295,
	edit_days_after_purchase: null,
	migrations_per_month: 20,
	scheduling: true,
	sync_pull_interval_secs: 21600,
	auto_publish_rules: true,
	templates_max: 20,
	collections_max: 20,
	labels_max: 20,
	analytics: true,
	export: true,
	devices_max: 2,
	ai_fills_per_month: 200,
	support: 'email_2_days'
};

function row(over: Partial<PlanRow> = {}): PlanRow {
	return {
		id: 'subscriber',
		name: 'Teachouse Subscription',
		monthly_cents: 2400,
		yearly_cents: 24000,
		trial_days: 14,
		sold: true,
		capabilities: CAPS,
		...over
	};
}

const TABLE: PlanRow[] = [
	row({ id: 'free', name: 'Free', monthly_cents: null, yearly_cents: null, trial_days: 0 }),
	row(),
	row({ id: 'migration_only', name: 'Catalogue Import', monthly_cents: null, yearly_cents: null }),
	row({
		id: 'studio',
		name: 'Studio',
		monthly_cents: 4400,
		yearly_cents: 44000,
		trial_days: 0,
		sold: false
	})
];

function read(over: Partial<EntitlementView> = {}): EntitlementRead {
	return {
		state: 'read',
		entitlement: {
			plan: 'subscriber',
			rung: null,
			granted_by: 'paddle',
			granted_at: 1_757_000_000_000,
			expires_at: null,
			capabilities: CAPS,
			usage: {
				resources: 12,
				marketplaces: 2,
				migrations_this_month: 0,
				migrations_reset_at: 1_759_276_800_000,
				templates: 1,
				labels: 3,
				devices: 1
			},
			...over
		}
	};
}

describe('money printed from cents', () => {
	it('drops the cents where a price is whole dollars', () => {
		expect(dollars(2400)).toBe('$24');
		expect(dollars(4700)).toBe('$47');
	});

	// A card is charged what the table says, so a price with cents in it is
	// printed with them rather than rounded to the nearest dollar.
	it('keeps them where it is not', () => {
		expect(dollars(2499)).toBe('$24.99');
		expect(dollars(50)).toBe('$0.50');
	});
});

describe('a plan’s price', () => {
	it('reads per month or per year', () => {
		expect(priceOf(row(), 'monthly')).toEqual({ amount: '$24', per: 'per month' });
		expect(priceOf(row(), 'annual')).toEqual({ amount: '$240', per: 'per year' });
	});

	it('reads as free, per nothing, on a plan with no price', () => {
		const free = row({ monthly_cents: null, yearly_cents: null });
		expect(priceOf(free, 'monthly')).toEqual({ amount: 'Free', per: '' });
		expect(priceOf(free, 'annual')).toEqual({ amount: 'Free', per: '' });
	});
});

describe('what paying yearly saves', () => {
	it('is two months at the founder’s prices', () => {
		expect(monthsFreeOnAnnual(row())).toBe(2);
	});

	it('is nothing to state on a plan with no price', () => {
		expect(monthsFreeOnAnnual(row({ monthly_cents: null, yearly_cents: null }))).toBeNull();
	});

	// A saving stated as "two months free" has to be exactly that, or it is a
	// rounded claim about money.
	it('is withheld rather than rounded when the year is not whole months cheaper', () => {
		expect(monthsFreeOnAnnual(row({ monthly_cents: 2400, yearly_cents: 25000 }))).toBeNull();
	});

	it('is withheld where the year costs at least as much as the months', () => {
		expect(monthsFreeOnAnnual(row({ monthly_cents: 2400, yearly_cents: 28800 }))).toBeNull();
		expect(monthsFreeOnAnnual(row({ monthly_cents: 2400, yearly_cents: 33600 }))).toBeNull();
	});
});

describe('the plans a deployment offers', () => {
	// Studio ships priced and unsold because the founder's trigger for selling
	// it is a measurement. A card for it would offer a plan no checkout buys.
	it('leaves out a plan that is priced but not sold', () => {
		expect(soldPlans(TABLE).map((plan) => plan.id)).toEqual([
			'free',
			'subscriber',
			'migration_only'
		]);
	});
});

describe('the standing an entitlement proves', () => {
	// This is the whole point of phase 1 on this page: the grant names the
	// plan, so the page states which one instead of saying only that a
	// subscription is running.
	it('names the plan the grant names', () => {
		const standing = standingFor(read(), TABLE);
		expect(standing.kind).toBe('read');
		expect(standing.plan).toBe('subscriber');
		expect(standing.name).toBe('Teachouse Subscription');
		expect(standing.headline).toBe('You are on Teachouse Subscription.');
	});

	it('says a one-off purchase runs out, and a free plan does not', () => {
		expect(standingFor(read({ plan: 'migration_only', rung: 50 }), TABLE).detail).toContain(
			'one-off Catalogue Import'
		);
		expect(standingFor(read({ plan: 'free', granted_by: null }), TABLE).detail).toContain(
			'does not run out'
		);
	});

	it('says so when a person set the plan rather than a card', () => {
		expect(standingFor(read({ granted_by: 'operator' }), TABLE).detail).toContain(
			'Teachouse set this plan'
		);
	});

	// A plan the served table does not carry still names a plan, and printing
	// nothing there would hide which one the tenant holds.
	it('falls back to the wire word for a plan the table does not carry', () => {
		expect(standingFor(read(), []).headline).toBe('You are on subscriber.');
	});

	it('claims no plan at all where the read has not answered', () => {
		const standing = standingFor({ state: 'unread' }, TABLE);
		expect(standing.kind).toBe('unread');
		expect(standing.plan).toBeNull();
		expect(standing.name).toBeNull();
	});
});

describe('the subscription control’s label', () => {
	it('offers to start where the plan is not a recurring one', () => {
		expect(checkoutLabel(read({ plan: 'free' }))).toBe('Subscribe');
		expect(checkoutLabel(read({ plan: 'migration_only' }))).toBe('Subscribe');
	});

	it('offers to manage where one is already running', () => {
		expect(checkoutLabel(read())).toBe('Manage');
		expect(checkoutLabel(read({ plan: 'studio' }))).toBe('Manage');
	});

	// The guard is the return type rather than a sibling `{#if}` a later edit
	// could drop: no control can be rendered over a standing that does not
	// know what it would be changing.
	it('is nothing at all where the read has not answered', () => {
		expect(checkoutLabel({ state: 'unread' })).toBeNull();
	});
});

describe('a ladder rung', () => {
	it('states its price and the volume it covers', () => {
		expect(rungLabel({ up_to: 50, price_cents: 7700 })).toBe('$77 · up to 50 resources');
	});
});

describe('the recorded billing period', () => {
	it('says so where Paddle recorded none', () => {
		expect(periodLabel({ current_period_end: null })).toBe('Paddle recorded no billing period');
	});

	it('renders a recorded one as a date carrying its year', () => {
		const at = Date.UTC(2026, 8, 30, 12, 0, 0);
		expect(periodLabel({ current_period_end: at })).toContain('2026');
	});
});
