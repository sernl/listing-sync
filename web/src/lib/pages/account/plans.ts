// The subscription page's own model: the four tiers as the founder approved
// them, the two payment cadences, and what the recorded billing state can
// honestly be said to prove.
//
// The tier table lives on the client because no server surface carries one:
// `/v1/billing` serves Paddle's status and nothing else, and the module doc
// for it is explicit that nothing there gates a feature, meters a quota or
// decides an entitlement. So these figures are what the seller is being
// offered, not what the platform enforces, and the page says so.

import type { SubscriptionView } from '$lib/api';

export type PlanId = 'free' | 'solo' | 'studio' | 'publisher';

export type Cadence = 'monthly' | 'annual';

export interface Plan {
	id: PlanId;
	name: string;
	/** What the tier is for, in one line. */
	blurb: string;
	/** Whole US dollars. Null on the free tier, which has no price at all
	 *  rather than a price of zero to render. */
	monthly: number | null;
	annual: number | null;
	/** The allowances, in the order every card lists them. The free tier
	 *  carries three because the founder named three; a fourth invented to
	 *  square the columns would be a figure nobody decided. */
	allowances: readonly string[];
	/** The free trial this tier carries, where it carries one. */
	trialDays?: number;
}

/** The trial, named once. */
export const TRIAL_DAYS = 14;

export const PLANS: readonly Plan[] = [
	{
		id: 'free',
		name: 'Free',
		blurb: 'Enough to see whether this works for your shop.',
		monthly: null,
		annual: null,
		allowances: ['One marketplace', 'Twenty resources', 'Manual sync']
	},
	{
		id: 'solo',
		name: 'Solo',
		blurb: 'One teacher selling on a couple of marketplaces.',
		monthly: 12,
		annual: 120,
		allowances: [
			'100 resources kept in sync',
			'2 marketplaces',
			'Daily sync',
			'1 device',
			'50 resources migrated a year'
		]
	},
	{
		id: 'studio',
		name: 'Studio',
		blurb: 'A working shop across every marketplace we support.',
		monthly: 24,
		annual: 240,
		allowances: [
			'400 resources kept in sync',
			'All marketplaces',
			'Sync every 6 hours',
			'2 devices',
			'200 resources migrated a year'
		],
		trialDays: TRIAL_DAYS
	},
	{
		id: 'publisher',
		name: 'Publisher',
		blurb: 'A catalogue large enough that the sync has to keep up with it.',
		monthly: 48,
		annual: 480,
		allowances: [
			'Unlimited resources kept in sync',
			'All marketplaces',
			'Hourly sync',
			'3 devices',
			'500 resources migrated a year'
		]
	}
];

export interface Price {
	/** What the figure reads as, already carrying its currency mark. */
	amount: string;
	/** What the figure is per, or the empty string where it is per nothing. */
	per: string;
}

/** A tier's price at one cadence. */
export function priceOf(plan: Plan, cadence: Cadence): Price {
	const dollars = cadence === 'annual' ? plan.annual : plan.monthly;
	if (dollars === null) {
		return { amount: 'Free', per: '' };
	}
	return { amount: `$${dollars}`, per: cadence === 'annual' ? 'per year' : 'per month' };
}

/**
 * How many months of the yearly price are not charged, against paying monthly.
 *
 * Null where either figure is absent, and null where the yearly price is not a
 * whole number of months cheaper: a saving stated as "two months free" has to
 * be exactly that or it is a rounded claim about money.
 */
export function monthsFreeOnAnnual(plan: Plan): number | null {
	if (plan.monthly === null || plan.annual === null || plan.monthly === 0) {
		return null;
	}
	const saved = plan.monthly * 12 - plan.annual;
	if (saved <= 0 || saved % plan.monthly !== 0) {
		return null;
	}
	return saved / plan.monthly;
}

/**
 * What the billing read has told us, in the three states it actually has.
 *
 * A pending read and a failed read are both `unread`. Collapsing either into
 * "no subscription" is what let a paying seller be shown the free tier in
 * green and offered a second subscription, so the absence of an answer is a
 * value here rather than a `null` that means two things.
 */
export type BillingRead =
	| { state: 'unread' }
	| { state: 'read'; subscription: SubscriptionView | null };

export type StandingKind = 'unread' | 'free' | 'subscribed';

export interface Standing {
	kind: StandingKind;
	/** The tier the record proves this organisation is on, or null where it
	 *  cannot say which — which includes every unread state. */
	plan: PlanId | null;
	/** Paddle's own word for the subscription, passed through untranslated,
	 *  or null where there is no subscription to have a status. */
	status: string | null;
	headline: string;
	detail: string;
}

/**
 * What the billing record says the organisation is on.
 *
 * The deployment sells through exactly one Paddle price, so a recorded
 * subscription proves that money is arriving and cannot prove which tier it is
 * for. Naming a tier from that would be a guess printed as a fact, so the
 * standing says which of the three situations this is and stops there. An
 * unread read names no tier at all: the page marks no card and offers no
 * purchase until the read answers.
 */
export function standingFor(read: BillingRead): Standing {
	if (read.state === 'unread') {
		return {
			kind: 'unread',
			plan: null,
			status: null,
			headline: 'The billing record has not answered.',
			detail:
				'Until it does, nothing here can say which plan you are on, so no plan below is marked and nothing can be bought.'
		};
	}
	if (read.subscription === null) {
		return {
			kind: 'free',
			plan: 'free',
			status: null,
			headline: 'You are on the free tier.',
			detail:
				'This organisation has never reached checkout, which is a different fact from a cancelled subscription — that one would be shown here with its status.'
		};
	}
	return {
		kind: 'subscribed',
		plan: null,
		status: read.subscription.status,
		headline: `Paddle records this subscription as ${read.subscription.status}.`,
		detail:
			'There is one price on this deployment, so the record says that a subscription is running and not which plan below it pays for.'
	};
}

/** When the recorded billing period ends, in the browser's own locale. */
export function periodLabel(subscription: SubscriptionView): string {
	return subscription.current_period_end === null
		? 'Paddle recorded no billing period'
		: new Date(subscription.current_period_end).toLocaleDateString();
}

/** What stands where the checkout control would, on a build that was given no
 *  Paddle token and price.
 *
 *  A sentence rather than a disabled button: there is nothing to press, and a
 *  greyed control invites the press anyway and then refuses it. */
export const CHECKOUT_DORMANT = 'Billing opens soon. Nothing on this page can be bought yet.';

/**
 * The label the one checkout control carries, or null for no control at all.
 *
 * Null on an unread read, so a control cannot be rendered without a successful
 * one: the guard is the return type rather than a sibling `{#if}` a later
 * edit could drop. The deployment sells a single Paddle price, so this control
 * buys that price and cannot buy a tier; a per-tier button appears only once a
 * per-tier price exists.
 */
export function checkoutLabel(read: BillingRead): string | null {
	if (read.state === 'unread') {
		return null;
	}
	return read.subscription === null ? 'Subscribe' : 'Change plan';
}

/**
 * The months a year costs less than twelve monthly payments, where every paid
 * tier agrees on the figure.
 *
 * Null where they disagree or where any of them withholds its saving, so the
 * page's one sentence about money is derived from the table rather than typed
 * beside it and left to drift.
 */
export function sharedMonthsFree(plans: readonly Plan[] = PLANS): number | null {
	const savings = plans
		.filter((plan) => plan.monthly !== null)
		.map((plan) => monthsFreeOnAnnual(plan));
	if (savings.length === 0 || savings.some((months) => months === null)) {
		return null;
	}
	const first = savings[0];
	return savings.every((months) => months === first) ? first : null;
}
