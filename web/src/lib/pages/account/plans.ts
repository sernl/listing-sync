// The subscription page's own model: how the served price table renders, and
// what the entitlement read can honestly be said to prove.
//
// The table itself is no longer here. `tam-limits` holds it, `GET /v1/plans`
// serves it, and `just web-typegen` mirrors it into `$lib/generated/plans`,
// so the pricing page, this page and the request gate read one set of
// figures. What is left is the rendering: dollars out of cents, the saving a
// year states, and the three states an entitlement read has.

import type { EntitlementView, PlanRow } from '$lib/api';
import { PLANS } from '$lib/generated/plans';
import type { Plan } from '$lib/generated/vocab';

export type Cadence = 'monthly' | 'annual';

export interface Price {
	/** What the figure reads as, already carrying its currency mark. */
	amount: string;
	/** What the figure is per, or the empty string where it is per nothing. */
	per: string;
}

/** Cents as the page prints money: whole dollars where the price is whole,
 *  and cents where it is not.
 *
 *  Rounding a price to the dollar would misstate what a card is charged, and
 *  printing `$24.00` beside `$47` reads as two different kinds of figure. */
export function dollars(cents: number): string {
	return cents % 100 === 0 ? `$${cents / 100}` : `$${(cents / 100).toFixed(2)}`;
}

/** A plan's price at one cadence. */
export function priceOf(plan: PlanRow, cadence: Cadence): Price {
	const cents = cadence === 'annual' ? plan.yearly_cents : plan.monthly_cents;
	if (cents === null) {
		return { amount: 'Free', per: '' };
	}
	return { amount: dollars(cents), per: cadence === 'annual' ? 'per year' : 'per month' };
}

/**
 * How many months of the yearly price are not charged, against paying
 * monthly.
 *
 * Null where either figure is absent, and null where the yearly price is not
 * a whole number of months cheaper: a saving stated as "two months free" has
 * to be exactly that or it is a rounded claim about money.
 */
export function monthsFreeOnAnnual(plan: PlanRow): number | null {
	const { monthly_cents: monthly, yearly_cents: yearly } = plan;
	if (monthly === null || yearly === null || monthly === 0) {
		return null;
	}
	const saved = monthly * 12 - yearly;
	if (saved <= 0 || saved % monthly !== 0) {
		return null;
	}
	return saved / monthly;
}

/** The plans this deployment actually sells, in table order.
 *
 *  `studio` ships in the code with a price and `sold: false`, because the
 *  founder's trigger for selling it is a measurement rather than a date. A
 *  card for it would offer a plan no checkout can buy. */
export function soldPlans(plans: readonly PlanRow[] = PLANS): PlanRow[] {
	return plans.filter((plan) => plan.sold);
}

/**
 * What the entitlement read has told us, in the three states it has.
 *
 * A pending read and a failed read are both `unread`. Collapsing either into
 * "no plan" is what let a paying seller be shown the free plan in green and
 * offered a second subscription, so the absence of an answer is a value here
 * rather than a `null` that means two things.
 */
export type EntitlementRead =
	| { state: 'unread' }
	| { state: 'read'; entitlement: EntitlementView };

export type StandingKind = 'unread' | 'read';

export interface Standing {
	kind: StandingKind;
	/** The plan the server says this organisation holds, or null where the
	 *  read has not answered. Never a guess: the grant names the plan. */
	plan: Plan | null;
	/** The plan's own name out of the served table, where the table carries
	 *  the plan the grant names. */
	name: string | null;
	headline: string;
	detail: string;
}

/**
 * What the entitlement says the organisation is on.
 *
 * This used to name no plan at all: one Paddle price could not prove which
 * tier money was arriving for, so a running subscription was rendered as "a
 * subscription is running" and nothing else. The grant names the plan, so the
 * page can now say which — and an unread read still names none, because a
 * plan printed before the server answered is a guess printed as a fact.
 */
export function standingFor(read: EntitlementRead, plans: readonly PlanRow[] = PLANS): Standing {
	if (read.state === 'unread') {
		return {
			kind: 'unread',
			plan: null,
			name: null,
			headline: 'Your plan has not been read yet.',
			detail:
				'Until it answers, we cannot say which plan you are on, so nothing below is marked and nothing can be bought.'
		};
	}
	const held = read.entitlement.plan;
	const row = plans.find((plan) => plan.id === held);
	const name = row?.name ?? null;
	return {
		kind: 'read',
		plan: held,
		name,
		headline: name === null ? `You are on ${held}.` : `You are on ${name}.`,
		detail: standingDetail(read.entitlement)
	};
}

/** The sentence under the headline: what the plan is doing next, where it is
 *  doing anything. */
function standingDetail(entitlement: EntitlementView): string {
	if (entitlement.granted_by === 'operator') {
		return 'Teachouse set this plan for you. Ask us before changing it here.';
	}
	if (entitlement.plan === 'migration_only') {
		return 'You bought a one-off Catalogue Import. Subscribe below to keep publishing after it runs out.';
	}
	if (entitlement.plan === 'free') {
		return 'The free plan does not run out. Subscribe below when you need more of it.';
	}
	return 'Your subscription renews on its own. Manage it below.';
}

/** When the recorded billing period ends, in the browser's own locale. */
export function periodLabel(subscription: { current_period_end: number | null }): string {
	return subscription.current_period_end === null
		? 'Paddle recorded no billing period'
		: new Date(subscription.current_period_end).toLocaleDateString();
}

/** What stands where the checkout control would, on a build that was given no
 *  Paddle token and prices.
 *
 *  A sentence rather than a disabled button: there is nothing to press, and a
 *  greyed control invites the press anyway and then refuses it. */
export const CHECKOUT_DORMANT = 'Billing opens soon. Nothing on this page can be bought yet.';

/**
 * The label the subscription control carries, or null for no control at all.
 *
 * Null on an unread read, so a control cannot be rendered over a standing
 * that does not know what it would be changing: the guard is the return type
 * rather than a sibling `{#if}` a later edit could drop. A seller already on
 * a recurring plan manages it rather than buying it again.
 */
export function checkoutLabel(read: EntitlementRead): string | null {
	if (read.state === 'unread') {
		return null;
	}
	const held = read.entitlement.plan;
	return held === 'subscriber' || held === 'studio' ? 'Manage' : 'Subscribe';
}

/** One rung of the import ladder, as its button reads: "$47 · up to 20
 *  resources". */
export function rungLabel(rung: { up_to: number; price_cents: number }): string {
	return `${dollars(rung.price_cents)} · up to ${rung.up_to} resources`;
}
