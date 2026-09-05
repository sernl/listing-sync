import { describe, expect, it } from 'vitest';
import type { SubscriptionView } from '$lib/api';
import {
	PLANS,
	TRIAL_DAYS,
	checkoutLabel,
	monthsFreeOnAnnual,
	periodLabel,
	priceOf,
	sharedMonthsFree,
	standingFor,
	type BillingRead,
	type Plan,
	type PlanId
} from './plans';

function plan(id: PlanId): Plan {
	const found = PLANS.find((entry) => entry.id === id);
	if (found === undefined) {
		throw new Error(`no ${id} tier`);
	}
	return found;
}

describe('the tier table', () => {
	it('carries the four tiers in the order they are compared', () => {
		expect(PLANS.map((entry) => entry.id)).toEqual(['free', 'solo', 'studio', 'publisher']);
	});

	// The prices and allowances are a founder decision, so they are asserted
	// here rather than left to a rendering test: a typo in one figure is a
	// wrong price shown to a buyer.
	it('prices the three paid tiers as the founder approved them', () => {
		expect([plan('solo').monthly, plan('solo').annual]).toEqual([12, 120]);
		expect([plan('studio').monthly, plan('studio').annual]).toEqual([24, 240]);
		expect([plan('publisher').monthly, plan('publisher').annual]).toEqual([48, 480]);
		expect([plan('free').monthly, plan('free').annual]).toEqual([null, null]);
	});

	it('states each tier’s allowances as the founder named them', () => {
		expect(plan('free').allowances).toEqual([
			'One marketplace',
			'Twenty resources',
			'Manual sync'
		]);
		expect(plan('solo').allowances).toEqual([
			'100 resources kept in sync',
			'2 marketplaces',
			'Daily sync',
			'1 device',
			'50 resources migrated a year'
		]);
		expect(plan('studio').allowances).toEqual([
			'400 resources kept in sync',
			'All marketplaces',
			'Sync every 6 hours',
			'2 devices',
			'200 resources migrated a year'
		]);
		expect(plan('publisher').allowances).toEqual([
			'Unlimited resources kept in sync',
			'All marketplaces',
			'Hourly sync',
			'3 devices',
			'500 resources migrated a year'
		]);
	});

	it('puts the fourteen-day trial on Studio and nowhere else', () => {
		expect(plan('studio').trialDays).toBe(TRIAL_DAYS);
		expect(PLANS.filter((entry) => entry.trialDays !== undefined).map((entry) => entry.id)).toEqual(
			['studio']
		);
	});
});

describe('a tier’s price', () => {
	it('reads per month or per year', () => {
		expect(priceOf(plan('solo'), 'monthly')).toEqual({ amount: '$12', per: 'per month' });
		expect(priceOf(plan('solo'), 'annual')).toEqual({ amount: '$120', per: 'per year' });
	});

	it('reads as free, per nothing, on the tier with no price', () => {
		expect(priceOf(plan('free'), 'monthly')).toEqual({ amount: 'Free', per: '' });
		expect(priceOf(plan('free'), 'annual')).toEqual({ amount: 'Free', per: '' });
	});
});

describe('what paying yearly saves', () => {
	it('is two months on every paid tier', () => {
		expect(monthsFreeOnAnnual(plan('solo'))).toBe(2);
		expect(monthsFreeOnAnnual(plan('studio'))).toBe(2);
		expect(monthsFreeOnAnnual(plan('publisher'))).toBe(2);
	});

	it('is nothing to state on the free tier', () => {
		expect(monthsFreeOnAnnual(plan('free'))).toBeNull();
	});

	it('is withheld rather than rounded when the year is not whole months cheaper', () => {
		const odd: Plan = { ...plan('solo'), annual: 125 };
		expect(monthsFreeOnAnnual(odd)).toBeNull();
	});

	it('is withheld where the year costs at least as much as the months', () => {
		expect(monthsFreeOnAnnual({ ...plan('solo'), annual: 144 })).toBeNull();
		expect(monthsFreeOnAnnual({ ...plan('solo'), annual: 180 })).toBeNull();
	});
});

const RECORDED: SubscriptionView = {
	paddle_subscription_id: 'sub_01',
	paddle_customer_id: 'ctm_01',
	status: 'active',
	current_period_end: Date.UTC(2026, 8, 14),
	occurred_at: Date.UTC(2026, 7, 31)
};

const UNREAD: BillingRead = { state: 'unread' };
const NO_SUBSCRIPTION: BillingRead = { state: 'read', subscription: null };
const SUBSCRIBED: BillingRead = { state: 'read', subscription: RECORDED };

describe('the standing a billing record proves', () => {
	it('is the free tier where a successful read found no subscription', () => {
		const standing = standingFor(NO_SUBSCRIPTION);
		expect(standing.kind).toBe('free');
		expect(standing.plan).toBe('free');
		expect(standing.status).toBeNull();
		expect(standing.detail).toContain('never reached checkout');
	});

	// One Paddle price serves every tier today, so a running subscription
	// cannot name which tier it pays for. Guessing one would print a tier the
	// seller is not necessarily on.
	it('names no tier where a subscription is running', () => {
		const standing = standingFor(SUBSCRIBED);
		expect(standing.kind).toBe('subscribed');
		expect(standing.plan).toBeNull();
		expect(standing.status).toBe('active');
		expect(standing.headline).toContain('active');
	});

	it('passes an unrecognised Paddle status through untranslated', () => {
		const paused: BillingRead = { state: 'read', subscription: { ...RECORDED, status: 'paused' } };
		expect(standingFor(paused).status).toBe('paused');
		expect(standingFor(paused).headline).toContain('paused');
	});
});

// The defect these pin: a pending or failed read used to arrive as the same
// `null` a never-subscribed tenant does, so a paying seller was shown the free
// tier in green and offered a second subscription.
describe('a read that has not answered', () => {
	it('claims no tier at all', () => {
		const standing = standingFor(UNREAD);
		expect(standing.kind).toBe('unread');
		expect(standing.plan).toBeNull();
		expect(standing.status).toBeNull();
	});

	it('marks no card, for every tier the page renders', () => {
		const standing = standingFor(UNREAD);
		for (const plan of PLANS) {
			expect(standing.plan === plan.id).toBe(false);
		}
	});

	it('never claims the seller has not subscribed', () => {
		const standing = standingFor(UNREAD);
		expect(standing.headline).not.toContain('free tier');
		expect(standing.detail).not.toContain('never reached checkout');
	});

	it('offers no purchase', () => {
		expect(checkoutLabel(UNREAD)).toBeNull();
	});

	it('is the only read that offers none', () => {
		const reads: readonly BillingRead[] = [UNREAD, NO_SUBSCRIPTION, SUBSCRIBED];
		const offered = reads.filter((read) => checkoutLabel(read) !== null);
		expect(offered).toEqual([NO_SUBSCRIPTION, SUBSCRIBED]);
	});
});

describe('the one checkout control’s label', () => {
	it('offers to start where a successful read found nothing running', () => {
		expect(checkoutLabel(NO_SUBSCRIPTION)).toBe('Subscribe');
	});

	// Including a cancelled one: Paddle still holds the subscription, and what
	// the seller does next is change it rather than open a second.
	it('offers to change where a subscription is recorded, whatever its status', () => {
		expect(checkoutLabel(SUBSCRIBED)).toBe('Change plan');
		expect(
			checkoutLabel({ state: 'read', subscription: { ...RECORDED, status: 'canceled' } })
		).toBe('Change plan');
	});
});

describe('the saving the page states once', () => {
	it('is the figure every paid tier agrees on', () => {
		expect(sharedMonthsFree()).toBe(2);
	});

	// Withheld rather than averaged: one sentence covering three tiers has to
	// be true of all three or it is a false claim about money.
	it('is withheld where the tiers disagree', () => {
		const odd = PLANS.map((plan) =>
			plan.id === 'solo' ? { ...plan, annual: 132 } : plan
		);
		expect(sharedMonthsFree(odd)).toBeNull();
	});

	it('is withheld where any tier withholds its own', () => {
		const odd = PLANS.map((plan) => (plan.id === 'solo' ? { ...plan, annual: 125 } : plan));
		expect(sharedMonthsFree(odd)).toBeNull();
	});
});

describe('the recorded billing period', () => {
	it('says so where Paddle recorded none', () => {
		expect(periodLabel({ ...RECORDED, current_period_end: null })).toBe(
			'Paddle recorded no billing period'
		);
	});

	// Asserted against the value rather than re-derived through the production
	// path: `toLocaleDateString` follows the runner's locale, so a literal
	// oracle would pin the locale rather than the behaviour. What is pinned is
	// that a recorded period is rendered as a date carrying its year, and is
	// not the sentence the absent case uses.
	it('renders a recorded one as a date carrying its year', () => {
		const rendered = periodLabel(RECORDED);
		expect(rendered).not.toBe('Paddle recorded no billing period');
		expect(rendered).toContain('2026');
		expect(rendered).toMatch(/\d/);
	});
});
