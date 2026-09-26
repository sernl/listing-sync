// The plan page's own model: how the generated price table renders, and what
// the billing read says in words.
//
// The figures are not here. `tam-limits` holds them, `GET /v1/plans` serves
// them, and `just web-typegen` mirrors them into `$lib/generated/plans`, so
// this page, the landing page and the request gate read one set. What is
// left is the rendering: dollars out of cents, a move balance in a sentence,
// and whether the founding offer is still open.

import type { EntitlementView, MoveBalance } from '$lib/api';
import { unlimited } from '$lib/entitlement';
import {
	AI,
	FOUNDING,
	PACKS,
	PLANS,
	type AiOffer,
	type Capabilities,
	type Founding,
	type Pack,
	type PlanRow
} from '$lib/generated/plans';

/** Cents as the page prints money: whole dollars where the price is whole,
 *  and cents where it is not.
 *
 *  Rounding a price to the dollar would misstate what a card is charged, and
 *  printing `$24.00` beside `$47` reads as two different kinds of figure. */
export function dollars(cents: number): string {
	return cents % 100 === 0 ? `$${cents / 100}` : `$${(cents / 100).toFixed(2)}`;
}

/** The yearly price as the month it works out at, which is the headline the
 *  yearly card leads with: a seller compares $20 against $29, not $240
 *  against $29. */
export function perMonth(yearlyCents: number): string {
	return dollars(Math.round(yearlyCents / 12));
}

/** The recurring plan this deployment sells.
 *
 *  `studio` ships priced and `sold: false`, because the founder's trigger for
 *  selling it is a measurement rather than a date, and a card for it would
 *  offer a plan no checkout can buy. */
export function syncPlan(plans: readonly PlanRow[] = PLANS): PlanRow | null {
	return plans.find((plan) => plan.id === 'subscriber' && plan.sold) ?? null;
}

/** One line on a plan card; `soon` marks a line sold before it is built. */
export interface PlanBullet {
	text: string;
	soon?: boolean;
}

/** A plan card's lines, read off `capabilities` so no figure is typed twice.
 *
 *  The wording is the landing page's pricing cards, so a seller reads the
 *  same promise on both. */
export function planBullets(caps: Capabilities, ai: AiOffer = AI): PlanBullet[] {
	const lines: PlanBullet[] = [];
	if (caps.moves_per_month > 0) lines.push({ text: `${caps.moves_per_month} moves a month` });
	if (caps.moves_accrual_cap > caps.moves_per_month)
		lines.push({ text: `Unused moves stack to ${caps.moves_accrual_cap}` });
	if (caps.free_moves_lifetime > 0)
		lines.push({ text: `${moves(caps.free_moves_lifetime)} onto a marketplace of your choice` });
	if (caps.import_spreadsheet && caps.import_marketplace)
		lines.push({ text: 'Import all your resources from wherever you sell' });
	if (!unlimited(caps.resources_max)) lines.push({ text: `Up to ${caps.resources_max} resources` });
	if (caps.sync_pull_interval_secs !== null)
		lines.push({ text: 'Edit resources in Teachouse and sync the edits across all platforms' });
	if (caps.scheduling) lines.push({ text: 'Scheduling' });
	if (caps.templates_max > 1)
		lines.push({ text: `${unlimited(caps.templates_max) ? 'Unlimited' : caps.templates_max} templates` });
	if (caps.collections_max > 0)
		lines.push({
			text: `${unlimited(caps.collections_max) ? 'Unlimited' : caps.collections_max} collections`
		});
	if (caps.analytics) lines.push({ text: 'Statistics on every shop' });
	if (caps.ai_fills_per_month > 0 && ai.status === 'coming_soon')
		lines.push({ text: `AI fill, coming soon (${caps.ai_fills_per_month} a month)`, soon: true });
	return lines;
}

/** A count of moves as a sentence says it. */
export function moves(count: number): string {
	return count === 1 ? '1 move' : `${count} moves`;
}

/** The pack that costs least per move, which is the one the grid marks.
 *
 *  Read from the table rather than named, so a pack added above the top one
 *  becomes the marked card without this file changing. */
export function bestValuePack(packs: readonly Pack[] = PACKS): Pack | null {
	return packs.reduce<Pack | null>(
		(best, pack) => (best === null || pack.per_move_cents < best.per_move_cents ? pack : best),
		null
	);
}

/** The packs smallest first, which is the order a grid of them reads in: a
 *  seller picks a size, and the per-move price falls as they go. */
export function packsBySize(packs: readonly Pack[] = PACKS): Pack[] {
	return [...packs].sort((one, two) => one.moves - two.moves);
}

/** One day as the console prints a date, in UTC.
 *
 *  UTC rather than the browser's zone: every instant on this page is the
 *  server's own — a renewal, an expiry, an offer's closing day — and a
 *  seller in Auckland reading a date a day out from the one the server
 *  enforces is a support email. */
export function dayLabel(ms: number): string {
	return new Date(ms).toLocaleDateString('en-GB', {
		day: 'numeric',
		month: 'short',
		year: 'numeric',
		timeZone: 'UTC'
	});
}

/** Whether the founding offer can still be bought.
 *
 *  The card is drawn from this rather than from a flag someone has to
 *  remember to turn off: the date is in the table, so the offer disappears on
 *  its own. The comparison is against the end of the closing day, because an
 *  offer that closes on the 31st is on sale on the 31st. */
export function foundingOpen(founding: Founding = FOUNDING, now: number = Date.now()): boolean {
	return now <= Date.parse(`${founding.closes_at}T23:59:59.999Z`);
}

/** When the founding offer closes, as its card says it. */
export function foundingClosesLabel(founding: Founding = FOUNDING): string {
	return `Closes ${dayLabel(Date.parse(`${founding.closes_at}T00:00:00Z`))}.`;
}

/** When the balance starts lapsing, or null where none of it does.
 *
 *  The balance carries one date and no count — the server answers the
 *  soonest expiry, not how much of the balance shares it — so the sentence
 *  says what it knows. */
export function expiryLine(balance: MoveBalance): string | null {
	return balance.expiring_soonest === undefined
		? null
		: `Moves start expiring on ${dayLabel(balance.expiring_soonest)}.`;
}

/** When the subscription renews, or null where nothing does. */
export function renewsLine(renewsAt: number | undefined): string | null {
	return renewsAt === undefined ? null : `Renews on ${dayLabel(renewsAt)}.`;
}

/** What Stripe sent the browser back with, where it sent one.
 *
 *  `success` and `cancel` are the only two the checkout hands back; anything
 *  else in the query is somebody's hand-typed URL and reads as no return at
 *  all. */
export type CheckoutOutcome = 'success' | 'cancel';

export function checkoutOutcome(param: string | null): CheckoutOutcome | null {
	return param === 'success' || param === 'cancel' ? param : null;
}

/**
 * What the entitlement read has told us, in the two states it has.
 *
 * A pending read and a failed read are the same state to a page: neither
 * says what the organisation holds, and a page that treated a failure as
 * `free` would tell a paying seller to buy what they already have.
 */
export type EntitlementRead =
	| { state: 'unread' }
	| { state: 'read'; entitlement: EntitlementView };
