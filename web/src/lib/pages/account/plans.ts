// The plan page's own model: how the generated price table renders, and what
// the billing read says in words.
//
// The figures are not here. `tam-limits` holds them, `GET /v1/plans` serves
// them, and `just web-typegen` mirrors them into `$lib/generated/plans`, so
// this page, the landing page and the request gate read one set. What is
// left is the rendering: dollars out of cents, a move balance in a sentence,
// and what a tier card says at the cadence the page is showing.

import type { BillingView, CardView, EntitlementView, InvoiceView, MoveBalance } from '$lib/api';
import { unlimited } from '$lib/entitlement';
import { afterPercentOff, saleLine, salePrice } from '$lib/sale';
import type { Tone } from '$lib/StatusPill.svelte';
import {
	AI,
	PACKS,
	PLANS,
	type AiOffer,
	type Capabilities,
	type Pack,
	type PlanRow,
	type SaleView
} from '$lib/generated/plans';
import type { Plan, PriceKey } from '$lib/generated/vocab';

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

/** The paid plans, cheapest first: every row that names a price. Free is
 *  not a tier card beside them; it is where a seller who buys nothing is. */
export function paidPlans(plans: readonly PlanRow[] = PLANS): PlanRow[] {
	return plans.filter((plan) => plan.monthly_key !== null && plan.yearly_key !== null);
}

/** Which of a plan's two prices the tier cards show and buy. */
export type Cadence = 'monthly' | 'yearly';

/** What one tier card says at the cadence the toggle is on. */
export interface TierPrice {
	/** The headline figure: the monthly price, or the yearly one over twelve. */
	headline: string;
	per: string;
	/** The small print under the figure: what the other cadence costs. */
	note: string;
	/** The key the card's button opens a checkout for. */
	key: PriceKey;
}

/** A tier card's price at one cadence, or null for a plan with no price.
 *
 *  The yearly card leads with the month it works out at and says the saving
 *  in dollars, because "$20 a month" is what a seller compares against $29
 *  and "you save $108" is the reason to pay a year up front. */
export function tierPrice(plan: PlanRow, cadence: Cadence): TierPrice | null {
	if (
		plan.monthly_cents === null ||
		plan.yearly_cents === null ||
		plan.monthly_key === null ||
		plan.yearly_key === null
	)
		return null;
	if (cadence === 'yearly')
		return {
			headline: perMonth(plan.yearly_cents),
			per: 'a month, billed yearly',
			note: `${dollars(plan.yearly_cents)} a year. You save ${dollars(plan.monthly_cents * 12 - plan.yearly_cents)}.`,
			key: plan.yearly_key
		};
	return {
		headline: dollars(plan.monthly_cents),
		per: 'a month',
		note: `Or ${perMonth(plan.yearly_cents)} a month if you pay yearly.`,
		key: plan.monthly_key
	};
}

/** One line on a plan card; `soon` marks a line sold before it is built. */
export interface PlanBullet {
	text: string;
	soon?: boolean;
}

/** A plan card's lines, read off `capabilities` so no figure is typed twice.
 *
 *  The wording and order are the landing page's pricing cards
 *  (`apps/landing/src/pricing.js`), so a seller reads the same promise on
 *  both. */
export function planBullets(caps: Capabilities, ai: AiOffer = AI): PlanBullet[] {
	const count = (n: number, noun: string) => `${unlimited(n) ? 'Unlimited' : n} ${noun}`;
	const lines: PlanBullet[] = [];
	if (caps.import_spreadsheet && caps.import_marketplace)
		lines.push({ text: 'Import from wherever you sell' });
	if (caps.sync_pull_interval_secs !== null) lines.push({ text: 'Edit once, sync everywhere' });
	if (caps.moves_per_month > 0) lines.push({ text: `${caps.moves_per_month} moves a month` });
	if (caps.moves_accrual_cap > caps.moves_per_month)
		lines.push({ text: `Unused moves stack to ${caps.moves_accrual_cap}` });
	if (!unlimited(caps.resources_max)) lines.push({ text: `Up to ${caps.resources_max} resources` });
	lines.push({ text: 'Add a watermarked preview of your file' });
	if (caps.scheduling) lines.push({ text: 'Scheduling' });
	if (caps.templates_max > 1) lines.push({ text: count(caps.templates_max, 'templates') });
	if (caps.collections_max > 0) lines.push({ text: count(caps.collections_max, 'collections') });
	if (caps.analytics) lines.push({ text: 'Statistics on every shop' });
	if (caps.auto_publish_rules) lines.push({ text: 'Automatic publishing rules' });
	if (caps.support === 'email_1_day') lines.push({ text: 'Priority support' });
	if (caps.ai_fills_per_month > 0 && ai.status === 'coming_soon')
		lines.push({ text: 'AI description fill, coming soon', soon: true });
	if (caps.free_moves_lifetime > 0)
		lines.push({
			text: `${moves(caps.free_moves_lifetime)} onto a marketplace of your choice`
		});
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

/** What the current-plan card says about the subscription's term: when it
 *  renews, or, once cancelled, when it ends. Null where neither is known —
 *  no subscription, or one whose end has passed. */
export function termLine(view: Pick<BillingView, 'renews_at' | 'ends_at'>): string | null {
	if (view.ends_at !== undefined) {
		return `Your plan ends on ${dayLabel(view.ends_at)} and will not renew.`;
	}
	if (view.renews_at !== undefined) {
		return `Your subscription renews on ${dayLabel(view.renews_at)}.`;
	}
	return null;
}

/** A plan in one line, read off its capabilities so the sentence moves when
 *  the plan does. */
export function planMeaning(caps: Capabilities): string {
	if (caps.moves_per_month > 0) {
		return caps.moves_accrual_cap > caps.moves_per_month
			? `${moves(caps.moves_per_month)} a month, saving up to ${caps.moves_accrual_cap}.`
			: `${moves(caps.moves_per_month)} a month.`;
	}
	return caps.free_moves_lifetime > 0
		? `${moves(caps.free_moves_lifetime)} free to try, then packs as you need them.`
		: 'Moves come from packs as you need them.';
}

/** An invoice total in its own currency. Stripe states totals in the
 *  currency's minor unit, and the number of those in a whole unit is the
 *  currency's own — two for dollars, none for yen — so the divisor is read
 *  from `Intl` rather than assumed. `en-US` so a New Zealand invoice reads
 *  `NZ$` rather than a bare `$` that could be anyone's dollar. */
export function money(total: number, currency: string): string {
	const format = new Intl.NumberFormat('en-US', {
		style: 'currency',
		currency: currency.toUpperCase()
	});
	const digits = format.resolvedOptions().maximumFractionDigits ?? 2;
	return format.format(total / 10 ** digits);
}

/** An invoice status as the table says it, with the pill tone it takes.
 *  `open` is a bill not yet paid, which a seller calls due. */
export function invoiceStatus(status: string): { label: string; tone: Tone } {
	switch (status) {
		case 'paid':
			return { label: 'Paid', tone: 'ok' };
		case 'open':
			return { label: 'Due', tone: 'warn' };
		case 'uncollectible':
			return { label: 'Unpaid', tone: 'bad' };
		case 'void':
			return { label: 'Void', tone: 'soon' };
		default:
			return { label: status, tone: 'soon' };
	}
}

const CARD_BRANDS: Record<string, string> = {
	amex: 'American Express',
	diners: 'Diners Club',
	discover: 'Discover',
	eftpos_au: 'EFTPOS',
	jcb: 'JCB',
	mastercard: 'Mastercard',
	unionpay: 'UnionPay',
	visa: 'Visa'
};

/** A card as the Payment row names it: the brand and the last four digits,
 *  which is what the seller's own bank statement shows. */
export function cardLabel(card: CardView): string {
	const brand = CARD_BRANDS[card.brand] ?? 'Card';
	return `${brand} •••• ${card.last4}`;
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
export type EntitlementRead = { state: 'unread' } | { state: 'read'; entitlement: EntitlementView };

// --------------------------------------------------------- the page's views

/** The two fields the plan table carries from 0.15 on — which paid plan the
 *  page recommends, and each plan in one sentence. Read as optional so the
 *  page renders the same table before and after the generated type has them. */
type PlanExtras = { readonly recommended?: boolean; readonly tagline?: string };

/** A plan in one sentence: its own tagline where the table has one, and
 *  otherwise what it gives, in moves. */
export function planTagline(plan: PlanRow): string {
	const tagline = (plan as PlanRow & PlanExtras).tagline;
	return tagline !== undefined && tagline.trim() !== '' ? tagline : planMeaning(plan.capabilities);
}

/** What a plan card's button does: open a checkout for one price, send a
 *  seller who already pays to Stripe's portal to switch, or nothing. */
export type PlanCta =
	{ kind: 'buy'; key: PriceKey; label: string } | { kind: 'switch'; label: string };

/** One card in the Plans grid, everything it prints already worked out. */
export interface PlanCard {
	id: Plan;
	name: string;
	/** "trial" for the free plan, "subscription" for the rest. */
	kind: 'trial' | 'subscription';
	tagline: string;
	headline: string;
	per: string;
	/** The list price struck through while a sale takes some off. */
	was: string | null;
	/** The sale in words, while one applies to this card. */
	saleNote: string | null;
	/** The small print under the price. */
	note: string | null;
	bullets: PlanBullet[];
	current: boolean;
	recommended: boolean;
	cta: PlanCta | null;
}

export interface PlanCardsInput {
	/** The plan the seller holds; undefined while the billing read is out. */
	held: Plan | undefined;
	cadence: Cadence;
	sale: SaleView | null;
	plans?: readonly PlanRow[];
	ai?: AiOffer;
}

/** The Plans grid, free plan first and then every priced plan, from the
 *  generated table alone.
 *
 *  A seller who already pays changes plan in Stripe's portal rather than
 *  through a second checkout, so their cards switch instead of buy, and no
 *  sale is shown to them: a sale reaches a new checkout, not a running
 *  subscription. */
export function planCards({
	held,
	cadence,
	sale,
	plans = PLANS,
	ai = AI
}: PlanCardsInput): PlanCard[] {
	const paid = paidPlans(plans);
	const subscribed = paid.some((plan) => plan.id === held);
	const cards: PlanCard[] = [];
	for (const plan of plans) {
		const recommended = (plan as PlanRow & PlanExtras).recommended === true;
		const base = {
			id: plan.id,
			name: plan.name,
			tagline: planTagline(plan),
			bullets: planBullets(plan.capabilities, ai),
			current: plan.id === held,
			recommended
		};
		if (!paid.includes(plan)) {
			if (plan.monthly_cents !== null || plan.yearly_cents !== null) continue;
			cards.push({
				...base,
				kind: 'trial',
				headline: 'Free',
				per: 'for as long as you like',
				was: null,
				saleNote: null,
				note: null,
				cta: null
			});
			continue;
		}
		const price = tierPrice(plan, cadence);
		if (price === null) continue;
		const onSale = subscribed
			? null
			: cadence === 'yearly'
				? salePrice(sale, plan.yearly_cents, 12)
				: salePrice(sale, plan.monthly_cents);
		const yearlyOnSale = onSale !== null && sale !== null && cadence === 'yearly';
		cards.push({
			...base,
			kind: 'subscription',
			headline: onSale === null ? price.headline : dollars(onSale.saleCents),
			per: price.per,
			was: onSale === null ? null : dollars(onSale.listCents),
			saleNote: onSale !== null && sale !== null ? saleLine(sale) : null,
			note:
				yearlyOnSale && plan.yearly_cents !== null
					? `${dollars(afterPercentOff(plan.yearly_cents, sale.percent_off))} for the year (usually ${dollars(plan.yearly_cents)}).`
					: price.note,
			cta: subscribed
				? base.current
					? null
					: { kind: 'switch', label: `Switch to ${plan.name}` }
				: { kind: 'buy', key: price.key, label: `Choose ${plan.name}` }
		});
	}
	return cards;
}

/** The price the current-plan card leads with: the held plan at the cadence
 *  it is billed at. Null for a paid plan with no cadence behind it — a plan
 *  an operator granted is not charged, and a price beside it would say it is. */
export function heldPrice(
	plan: PlanRow,
	cadence: Cadence | undefined
): { headline: string; per: string } | null {
	if (plan.monthly_cents === null && plan.yearly_cents === null) {
		return { headline: 'Free', per: 'for as long as you like' };
	}
	if (cadence === undefined) return null;
	const price = tierPrice(plan, cadence);
	if (price === null) return null;
	return cadence === 'yearly' && plan.yearly_cents !== null
		? { headline: dollars(plan.yearly_cents), per: 'a year' }
		: { headline: price.headline, per: price.per };
}

/** One row of the Invoices table. */
export interface InvoiceRow {
	id: string;
	date: string;
	description: string;
	amount: string;
	status: { label: string; tone: Tone };
	/** Where the receipt opens: Stripe's hosted page, or its PDF. */
	receipt: string | null;
}

/** The Invoices table's rows, newest first.
 *
 *  Stripe leaves a subscription invoice's description empty; the row then
 *  says what every invoice here is, a payment to Teachouse. */
export function invoiceRows(invoices: readonly InvoiceView[]): InvoiceRow[] {
	return [...invoices]
		.sort((one, two) => two.created_at - one.created_at)
		.map((invoice) => ({
			id: invoice.id,
			date: dayLabel(invoice.created_at),
			description:
				invoice.description === undefined || invoice.description.trim() === ''
					? 'Payment to Teachouse'
					: invoice.description,
			amount: money(invoice.total, invoice.currency),
			status: invoiceStatus(invoice.status),
			receipt: invoice.hosted_url ?? invoice.pdf_url ?? null
		}));
}

const BRAND_MARKS: Record<string, string> = {
	amex: 'AMEX',
	diners: 'DINERS',
	discover: 'DISC',
	eftpos_au: 'EFTPOS',
	jcb: 'JCB',
	mastercard: 'MC',
	unionpay: 'UPI',
	visa: 'VISA'
};

/** The short mark the Payment card draws for a card's brand. */
export function cardMark(card: CardView): string {
	return BRAND_MARKS[card.brand] ?? 'CARD';
}

/** "Expires 04/2028", or null where Stripe did not say. */
export function cardExpiry(card: CardView): string | null {
	if (card.exp_month === undefined || card.exp_year === undefined) return null;
	return `Expires ${String(card.exp_month).padStart(2, '0')}/${card.exp_year}`;
}
