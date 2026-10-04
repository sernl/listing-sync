/**
 * The admin Payments page's wire shapes and the arithmetic it does on them:
 * which rows the filters keep, this month's figures, what is left to refund
 * on a charge, which rows belong together, what the refund policy quotes,
 * what the refund form accepts before it sends, and how sellers' refund
 * requests read.
 *
 * The server checks every refund rule again; these exist so the panel can say
 * what is wrong before Stripe is asked. Months and days are New Zealand's
 * (`Pacific/Auckland`), because that is the company's calendar.
 */

import type { Tone } from '$lib/StatusPill.svelte';
import { money } from '$lib/pages/account/plans';

export type PaymentKind =
	| 'payment_succeeded'
	| 'payment_failed'
	| 'refund_created'
	| 'refund_updated'
	| 'dispute_opened'
	| 'dispute_closed'
	| 'invoice_paid'
	| 'invoice_payment_failed'
	| 'subscription_created'
	| 'subscription_canceled';

export interface PaymentEventView {
	id: string;
	org_id: string | null;
	org_name: string | null;
	kind: PaymentKind;
	/** Null for subscription events with no amount. */
	amount_cents: number | null;
	currency: string | null;
	/** The Stripe object: `ch_`, `pi_`, `re_`, `dp_`, `in_` or `sub_`. */
	provider_object_id: string;
	/** The charge this event is about, where one is known. */
	charge_id: string | null;
	payment_intent_id: string | null;
	invoice_id: string | null;
	customer_id: string | null;
	/** Stripe's own status word. */
	status: string | null;
	reason: string | null;
	occurred_at: number;
}

export type RefundStatus = 'pending' | 'succeeded' | 'failed' | 'canceled';
export type RefundReason = 'duplicate' | 'fraudulent' | 'requested_by_customer';

export interface RefundView {
	/** Also the `request_id` the refund panel minted. */
	id: string;
	org_id: string | null;
	org_name: string | null;
	provider_refund_id: string;
	charge_id: string;
	amount_cents: number;
	currency: string;
	status: RefundStatus;
	reason: string | null;
	/** The operator's internal note. */
	note: string | null;
	/** Who issued it; null when it was made in Stripe's dashboard. */
	issued_by: string | null;
	mail_requested_at: number | null;
	mail_sent_at: number | null;
	mail_error: string | null;
	created_at: number;
	updated_at: number;
	/** The policy's basis when the refund panel sent its quote; null for older
	 *  refunds and ones made in Stripe's dashboard. */
	policy_basis: string | null;
	/** What the policy said, in cents, when the refund was issued. */
	quoted_cents: number | null;
	/** Why the amount differs from `quoted_cents`, where it does. */
	override_reason: string | null;
}

/** Why the policy came to its amount. */
export type RefundBasis =
	| 'pack_unused'
	| 'pack_used'
	| 'pack_window_closed'
	| 'monthly_started'
	| 'monthly_not_started'
	| 'yearly_unused_months'
	| 'yearly_ended'
	| 'unmatched';

/** What the Terms' refund policy says one charge is owed, as the server
 *  works it out. */
export interface QuoteView {
	charge_id: string;
	org_id: string | null;
	/** "Pro yearly plan", "Move Pack of 50 moves"; null when unmatched. */
	what: string | null;
	paid_cents: number;
	currency: string;
	paid_at: number;
	/** What Stripe says is left on the charge. */
	remaining_cents: number;
	/** The policy amount, already capped at `remaining_cents`. */
	amount_cents: number;
	basis: RefundBasis;
	/** The policy sentence; shown verbatim. */
	explanation: string;
	/** A yearly plan: refunding ends the plan today. */
	ends_plan: boolean;
}

export type RefundRequestStatus = 'requested' | 'approved' | 'declined';

/** A seller asking for the policy's refund on one of their payments. */
export interface RefundRequestView {
	id: string;
	org_id: string;
	org_name: string | null;
	charge_id: string;
	quoted_cents: number;
	currency: string;
	policy_basis: string;
	status: RefundRequestStatus;
	/** The seller's own note. */
	note: string | null;
	decided_by: string | null;
	decided_at: number | null;
	decline_reason: string | null;
	refund_id: string | null;
	created_at: number;
}

export interface PaymentsAdminView {
	/** Newest first, the last 400 days at most. */
	events: PaymentEventView[];
	/** Newest first. */
	refunds: RefundView[];
	/** Sellers' refund requests, open ones first. */
	requests: RefundRequestView[];
	auto_refund_mail: boolean;
	stripe_configured: boolean;
}

export interface RefundBody {
	/** Minted once per opening of the refund panel, so a double press refunds once. */
	request_id: string;
	amount_cents: number;
	reason: RefundReason;
	note: string;
	send_email: boolean;
	/** The operator's name; the server says "an admin" without it. */
	issued_by_label?: string;
	/** The quote the panel opened with, so the server can tell it changed. */
	policy_basis?: RefundBasis;
	quoted_cents?: number;
	/** Required by the server when `amount_cents` differs from `quoted_cents`. */
	override_reason?: string;
	/** Only meaningful when the quote ends a yearly plan. */
	end_plan?: boolean;
}

/** What Approve and Decline send on a refund request. */
export interface DecisionBody {
	/** Why it was declined; the seller is emailed it. Ignored by Approve. */
	reason?: string;
	/** The operator's name, as the refund panel sends `issued_by_label`. */
	decided_by_label: string;
}

export interface SyncView {
	charges: number;
	refunds: number;
	disputes: number;
	invoices: number;
	/** Ledger rows newly written or refreshed. */
	recorded: number;
}

export interface PaymentSettingsBody {
	auto_refund_mail: boolean;
}

// ------------------------------------------------------------- vocabulary

export type KindGroup = 'payments' | 'refunds' | 'disputes' | 'invoices' | 'subscriptions';

export const KIND_GROUPS: Record<KindGroup, readonly PaymentKind[]> = {
	payments: ['payment_succeeded', 'payment_failed'],
	refunds: ['refund_created', 'refund_updated'],
	disputes: ['dispute_opened', 'dispute_closed'],
	invoices: ['invoice_paid', 'invoice_payment_failed'],
	subscriptions: ['subscription_created', 'subscription_canceled']
};

export const GROUP_LABEL: Record<KindGroup, string> = {
	payments: 'Payments',
	refunds: 'Refunds',
	disputes: 'Chargebacks',
	invoices: 'Invoices',
	subscriptions: 'Subscriptions'
};

export const KIND_LABEL: Record<PaymentKind, string> = {
	payment_succeeded: 'Payment',
	payment_failed: 'Failed payment',
	refund_created: 'Refund',
	refund_updated: 'Refund update',
	dispute_opened: 'Chargeback opened',
	dispute_closed: 'Chargeback closed',
	invoice_paid: 'Invoice paid',
	invoice_payment_failed: 'Invoice not paid',
	subscription_created: 'Subscription started',
	subscription_canceled: 'Subscription cancelled'
};

export const REASON_LABEL: Record<RefundReason, string> = {
	duplicate: 'Duplicate',
	fraudulent: 'Fraudulent',
	requested_by_customer: 'Requested by the customer'
};

export const REASONS: readonly RefundReason[] = [
	'requested_by_customer',
	'duplicate',
	'fraudulent'
];

/** Why the policy came to its amount, as the operator reads it. */
export const BASIS_LABEL: Record<RefundBasis, string> = {
	pack_unused: 'Unused Move Pack, within 14 days',
	pack_used: 'Move Pack with moves used',
	pack_window_closed: 'Move Pack, after 14 days',
	monthly_started: 'Monthly plan, month started',
	monthly_not_started: 'Monthly plan, month not started',
	yearly_unused_months: 'Yearly plan, unused months less one',
	yearly_ended: 'Yearly plan, year over',
	unmatched: 'Not matched to a plan or pack'
};

/** A basis as the wire carries it, which may be a word this console does
 *  not know yet: that one is shown as written. */
export function basisLabel(basis: string): string {
	return Object.hasOwn(BASIS_LABEL, basis) ? BASIS_LABEL[basis as RefundBasis] : statusWords(basis);
}

// ------------------------------------------------------------ NZ calendar

const ZONE = 'Pacific/Auckland';

const DAY_PARTS = new Intl.DateTimeFormat('en-GB', {
	timeZone: ZONE,
	year: 'numeric',
	month: '2-digit',
	day: '2-digit'
});

/** The New Zealand calendar day an instant falls on, as `YYYY-MM-DD`. */
export function nzDay(ms: number): string {
	const parts = DAY_PARTS.formatToParts(new Date(ms));
	const part = (type: Intl.DateTimeFormatPartTypes) =>
		parts.find((entry) => entry.type === type)?.value ?? '';
	return `${part('year')}-${part('month')}-${part('day')}`;
}

/** The New Zealand calendar month an instant falls in, as `YYYY-MM`. */
export function nzMonth(ms: number): string {
	return nzDay(ms).slice(0, 7);
}

const LONG_DATE = new Intl.DateTimeFormat('en-GB', {
	timeZone: ZONE,
	day: 'numeric',
	month: 'long',
	year: 'numeric'
});

/** "3 October 2026", in New Zealand. */
export function nzLongDate(ms: number): string {
	return LONG_DATE.format(new Date(ms));
}

const SHORT_DATE = new Intl.DateTimeFormat('en-GB', {
	timeZone: ZONE,
	day: 'numeric',
	month: 'short',
	year: 'numeric'
});

const TIME = new Intl.DateTimeFormat('en-NZ', {
	timeZone: ZONE,
	hour: 'numeric',
	minute: '2-digit'
});

/** "3 Oct 2026", in New Zealand. */
export function nzShortDate(ms: number): string {
	return SHORT_DATE.format(new Date(ms));
}

/** "1:30 pm", in New Zealand. */
export function nzTime(ms: number): string {
	return TIME.format(new Date(ms));
}

/** "This month" as the stats header names it: "October 2026". */
export function nzMonthLabel(ms: number): string {
	return new Intl.DateTimeFormat('en-GB', {
		timeZone: ZONE,
		month: 'long',
		year: 'numeric'
	}).format(new Date(ms));
}

// ---------------------------------------------------------------- filters

/** `all`, a group of kinds, or one exact kind. */
export type KindFilter = 'all' | KindGroup | PaymentKind;

export interface PaymentFilters {
	kind: KindFilter;
	/** Stripe's exact status word, or `all`. */
	status: string;
	/** Part of an organisation's name or id; empty for every one. */
	org: string;
	/** Inclusive New Zealand days, `YYYY-MM-DD`; empty for no bound. */
	from: string;
	to: string;
}

export const NO_FILTERS: PaymentFilters = { kind: 'all', status: 'all', org: '', from: '', to: '' };

function kindMatches(kind: PaymentKind, filter: KindFilter): boolean {
	if (filter === 'all') return true;
	if (filter in KIND_GROUPS) return KIND_GROUPS[filter as KindGroup].includes(kind);
	return kind === filter;
}

function orgMatches(event: PaymentEventView, search: string): boolean {
	const needle = search.trim().toLowerCase();
	if (needle === '') return true;
	return [event.org_name, event.org_id].some(
		(value) => value !== null && value.toLowerCase().includes(needle)
	);
}

/** The events every filter keeps, in the order given. */
export function filterEvents(
	events: readonly PaymentEventView[],
	filters: PaymentFilters
): PaymentEventView[] {
	return events.filter((event) => {
		if (!kindMatches(event.kind, filters.kind)) return false;
		if (filters.status !== 'all' && event.status !== filters.status) return false;
		if (!orgMatches(event, filters.org)) return false;
		const day = nzDay(event.occurred_at);
		if (filters.from !== '' && day < filters.from) return false;
		if (filters.to !== '' && day > filters.to) return false;
		return true;
	});
}

/** Every status word the events carry, for the status filter. */
export function statusOptions(events: readonly PaymentEventView[]): string[] {
	const words = new Set<string>();
	for (const event of events) {
		if (event.status !== null) words.add(event.status);
	}
	return [...words].sort();
}

export function filtersActive(filters: PaymentFilters): boolean {
	return (
		filters.kind !== 'all' ||
		filters.status !== 'all' ||
		filters.org.trim() !== '' ||
		filters.from !== '' ||
		filters.to !== ''
	);
}

// ------------------------------------------------------------------ stats

/** A count and its total, per currency. */
export interface Tally {
	count: number;
	cents: Record<string, number>;
}

export interface MonthStats {
	/** `YYYY-MM`, the New Zealand month of now. */
	month: string;
	paid: Tally;
	refunded: Tally;
	disputed: Tally;
}

function add(tally: Tally, cents: number | null, currency: string | null): void {
	tally.count += 1;
	if (cents === null) return;
	const key = currency ?? 'usd';
	tally.cents[key] = (tally.cents[key] ?? 0) + cents;
}

/** Which refunds gave money back, or are about to: a failed or cancelled one did not. */
export const REFUND_COUNTS: Record<RefundStatus, boolean> = {
	pending: true,
	succeeded: true,
	failed: false,
	canceled: false
};

/** Paid, refunded and disputed in the New Zealand calendar month of `now`. */
export function monthStats(
	events: readonly PaymentEventView[],
	refunds: readonly RefundView[],
	now: number
): MonthStats {
	const month = nzMonth(now);
	const stats: MonthStats = {
		month,
		paid: { count: 0, cents: {} },
		refunded: { count: 0, cents: {} },
		disputed: { count: 0, cents: {} }
	};
	for (const event of events) {
		if (nzMonth(event.occurred_at) !== month) continue;
		if (event.kind === 'payment_succeeded') add(stats.paid, event.amount_cents, event.currency);
		if (event.kind === 'dispute_opened') add(stats.disputed, event.amount_cents, event.currency);
	}
	for (const refund of refunds) {
		if (REFUND_COUNTS[refund.status] && nzMonth(refund.created_at) === month) {
			add(stats.refunded, refund.amount_cents, refund.currency);
		}
	}
	return stats;
}

/** A tally's total, the largest currency first: "$120.00", or "$120.00 + NZ$15.00". */
export function tallyAmount(tally: Tally): string {
	const entries = Object.entries(tally.cents).sort((one, two) => two[1] - one[1]);
	if (entries.length === 0) return money(0, 'usd');
	return entries.map(([currency, cents]) => money(cents, currency)).join(' + ');
}

// -------------------------------------------------------- one charge's sums

/** The successful payment for a charge, if the ledger holds it. */
export function paymentFor(
	chargeId: string,
	events: readonly PaymentEventView[]
): PaymentEventView | null {
	return (
		events.find(
			(event) =>
				event.kind === 'payment_succeeded' &&
				(event.charge_id ?? event.provider_object_id) === chargeId
		) ?? null
	);
}

/** What can still be refunded on a charge: what was paid less every refund
 *  that is pending or went through. Failed and cancelled refunds gave nothing
 *  back. Never below zero; zero for a charge the ledger has no payment for. */
export function remaining(
	chargeId: string,
	events: readonly PaymentEventView[],
	refunds: readonly RefundView[]
): number {
	const paid = paymentFor(chargeId, events)?.amount_cents ?? 0;
	const back = refunds
		.filter((refund) => refund.charge_id === chargeId && REFUND_COUNTS[refund.status])
		.reduce((sum, refund) => sum + refund.amount_cents, 0);
	return Math.max(0, paid - back);
}

/** The ids an event names, which the rows about the same money share. */
function idsOf(event: PaymentEventView): string[] {
	return [event.charge_id, event.payment_intent_id, event.invoice_id].filter(
		(id): id is string => id !== null
	);
}

/** Every other event about the same money: sharing a charge, payment intent
 *  or invoice, or being the very object one of those ids names. */
export function relatedEvents(
	event: PaymentEventView,
	events: readonly PaymentEventView[]
): PaymentEventView[] {
	const ids = new Set(idsOf(event));
	if (ids.size === 0) return [];
	return events.filter(
		(other) =>
			other.id !== event.id &&
			(ids.has(other.provider_object_id) || idsOf(other).some((id) => ids.has(id)))
	);
}

/** The refunds on the charge an event is about. */
export function relatedRefunds(
	event: PaymentEventView,
	refunds: readonly RefundView[]
): RefundView[] {
	if (event.charge_id === null) return [];
	return refunds.filter((refund) => refund.charge_id === event.charge_id);
}

/** The refund a refund event is the Stripe record of. */
export function refundOf(
	event: PaymentEventView,
	refunds: readonly RefundView[]
): RefundView | null {
	if (!KIND_GROUPS.refunds.includes(event.kind)) return null;
	return refunds.find((refund) => refund.provider_refund_id === event.provider_object_id) ?? null;
}

// ------------------------------------------------------------ presentation

export interface Pill {
	tone: Tone;
	label: string;
}

/** Stripe's status word as a person writes it: `needs_response` → "needs response". */
export function statusWords(status: string): string {
	return status.replaceAll('_', ' ');
}

const STATUS_TONE: Record<string, Tone> = {
	succeeded: 'ok',
	paid: 'ok',
	active: 'ok',
	won: 'ok',
	pending: 'run',
	processing: 'run',
	needs_response: 'bad',
	warning_needs_response: 'bad',
	under_review: 'warn',
	warning_under_review: 'warn',
	lost: 'bad',
	failed: 'bad',
	canceled: 'soon',
	charge_refunded: 'soon'
};

/** The pill a row carries. A payment refunded in full says Refunded; a
 *  chargeback says so, with where Stripe has it. */
export function eventPill(event: PaymentEventView, left: number | null): Pill {
	if (event.kind === 'dispute_opened' || event.kind === 'dispute_closed') {
		const tone = event.status === null ? 'bad' : (STATUS_TONE[event.status] ?? 'warn');
		return {
			tone,
			label: event.status === null ? 'Chargeback' : `Chargeback: ${statusWords(event.status)}`
		};
	}
	if (event.kind === 'payment_succeeded' && left !== null && event.amount_cents !== null) {
		if (left === 0) return { tone: 'soon', label: 'Refunded' };
		if (left < event.amount_cents) return { tone: 'warn', label: 'Partly refunded' };
		return { tone: 'ok', label: 'Paid' };
	}
	if (event.kind === 'payment_failed' || event.kind === 'invoice_payment_failed') {
		return { tone: 'bad', label: 'Failed' };
	}
	if (event.status === null) return { tone: 'flat', label: '—' };
	const words = statusWords(event.status);
	return {
		tone: STATUS_TONE[event.status] ?? 'flat',
		label: words.charAt(0).toUpperCase() + words.slice(1)
	};
}

const REFUND_TONE: Record<RefundStatus, Tone> = {
	pending: 'run',
	succeeded: 'ok',
	failed: 'bad',
	canceled: 'soon'
};

const REFUND_LABEL: Record<RefundStatus, string> = {
	pending: 'Pending',
	succeeded: 'Refunded',
	failed: 'Failed',
	canceled: 'Cancelled'
};

export function refundPill(refund: RefundView): Pill {
	return { tone: REFUND_TONE[refund.status], label: REFUND_LABEL[refund.status] };
}

/** The pill a ledger row carries: a refund event shows its refund's own
 *  status, a payment shows how much of it is left. */
export function rowPill(
	event: PaymentEventView,
	events: readonly PaymentEventView[],
	refunds: readonly RefundView[]
): Pill {
	const refund = refundOf(event, refunds);
	if (refund !== null) return refundPill(refund);
	const left =
		event.kind === 'payment_succeeded'
			? remaining(event.charge_id ?? event.provider_object_id, events, refunds)
			: null;
	return eventPill(event, left);
}

/** An amount with its currency, or a dash for the events that carry none. */
export function eventAmount(event: Pick<PaymentEventView, 'amount_cents' | 'currency'>): string {
	return event.amount_cents === null ? '—' : money(event.amount_cents, event.currency ?? 'usd');
}

// ------------------------------------------------------------- refund form

/** What the refund panel holds while it is filled in. */
export interface RefundDraft {
	/** Dollars, as typed. */
	dollars: number | null;
	reason: RefundReason | '';
	note: string;
	sendEmail: boolean;
	/** Why the amount differs from the policy's; sent only when it does. */
	overrideReason: string;
	/** End a yearly plan today; only read when the quote ends one. */
	endPlan: boolean;
}

export type RefundField = 'amount' | 'reason' | 'override';

/** The panel's opening state once the quote is in: the policy's amount, the
 *  auto-mail setting, and a yearly plan ending as the Terms say it does. */
export function quoteDraft(quote: QuoteView, autoMail: boolean): RefundDraft {
	return {
		dollars: quote.amount_cents / 100,
		reason: '',
		note: '',
		sendEmail: autoMail,
		overrideReason: '',
		endPlan: quote.ends_plan
	};
}

/** The draft against a newer quote (the old one went stale): the amount
 *  starts again from the policy's, everything else typed stays. */
export function requote(draft: RefundDraft, quote: QuoteView): RefundDraft {
	return { ...draft, dollars: quote.amount_cents / 100 };
}

/** What the policy covers, or that the charge matched nothing it covers. */
export function quoteWhat(quote: QuoteView): string {
	return quote.what ?? 'Not matched to a plan or pack';
}

/** Dollars as typed, in cents; null where it is not a whole number of cents. */
export function toCents(dollars: number | null): number | null {
	if (dollars === null || !Number.isFinite(dollars)) return null;
	const cents = Math.round(dollars * 100);
	return Math.abs(cents - dollars * 100) < 1e-6 ? cents : null;
}

/** Whether the typed amount is not the policy's, which needs a reason. An
 *  amount that is not yet a whole number of cents is not compared. */
export function differsFromQuote(draft: RefundDraft, quote: QuoteView): boolean {
	const cents = toCents(draft.dollars);
	return cents !== null && cents !== quote.amount_cents;
}

/** What is wrong with the refund form, by field; empty when it can be sent. */
export function refundProblems(
	draft: RefundDraft,
	left: number,
	currency: string,
	quote: QuoteView
): Partial<Record<RefundField, string>> {
	const problems: Partial<Record<RefundField, string>> = {};
	const cents = toCents(draft.dollars);
	if (draft.dollars === null || !Number.isFinite(draft.dollars) || draft.dollars <= 0) {
		problems.amount = 'Enter an amount greater than zero.';
	} else if (cents === null) {
		problems.amount = 'Enter dollars and cents, like 12.50.';
	} else if (cents < 1) {
		problems.amount = 'Enter an amount greater than zero.';
	} else if (cents > left) {
		problems.amount = `You can refund at most ${money(left, currency)} on this payment.`;
	}
	if (draft.reason === '') {
		problems.reason = 'Pick a reason.';
	}
	if (differsFromQuote(draft, quote) && draft.overrideReason.trim() === '') {
		problems.override = "Say why you're refunding a different amount from the policy.";
	}
	return problems;
}

/** The body the refund is sent as, carrying the quote the panel opened with.
 *  Only call once `refundProblems` is empty. */
export function refundBody(
	draft: RefundDraft,
	requestId: string,
	issuedBy: string,
	quote: QuoteView
): RefundBody {
	return {
		request_id: requestId,
		amount_cents: toCents(draft.dollars) ?? 0,
		reason: draft.reason === '' ? 'requested_by_customer' : draft.reason,
		note: draft.note.trim(),
		send_email: draft.sendEmail,
		issued_by_label: issuedBy,
		policy_basis: quote.basis,
		quoted_cents: quote.amount_cents,
		...(differsFromQuote(draft, quote) ? { override_reason: draft.overrideReason.trim() } : {}),
		end_plan: quote.ends_plan && draft.endPlan
	};
}

/** Whose card, as a refund sentence names it. */
function cardOf(orgName: string | null): string {
	return orgName === null || orgName.trim() === '' ? "the customer's" : `${orgName.trim()}'s`;
}

/** "Refund $12.00 to Kiwi Kids's card?" */
export function confirmSentence(cents: number, currency: string, orgName: string | null): string {
	return `Refund ${money(cents, currency)} to ${cardOf(orgName)} card?`;
}

/** The confirm step's word on the plan, where this refund ends one. */
export function planLine(draft: RefundDraft, quote: QuoteView): string | null {
	if (!quote.ends_plan) return null;
	return draft.endPlan
		? 'Their yearly plan ends today and the account moves to Look.'
		: 'Their yearly plan keeps running.';
}

/** What the policy said beside a refund that carried a quote, and why the
 *  amount differs where it does. Null for refunds without one. */
export function refundPolicy(
	refund: RefundView
): { quoted: string; override: string | null } | null {
	if (refund.quoted_cents === null) return null;
	const differs = refund.quoted_cents !== refund.amount_cents;
	return {
		quoted: `Policy ${money(refund.quoted_cents, refund.currency)}`,
		override: differs && refund.override_reason ? refund.override_reason : null
	};
}

// ---------------------------------------------------------- refund requests

/** Open requests first, otherwise in the order the server sent them. */
export function requestsInOrder(requests: readonly RefundRequestView[]): RefundRequestView[] {
	const open = requests.filter((request) => request.status === 'requested');
	return [...open, ...requests.filter((request) => request.status !== 'requested')];
}

const REQUEST_PILL: Record<RefundRequestStatus, Pill> = {
	requested: { tone: 'warn', label: 'Open' },
	approved: { tone: 'ok', label: 'Approved' },
	declined: { tone: 'flat', label: 'Declined' }
};

export function requestPill(request: RefundRequestView): Pill {
	return REQUEST_PILL[request.status];
}

/** "Refund $140.00 to Kiwi Kids's card? We'll email them, and a yearly plan
 *  ends today." The plan clause only where the quote was a yearly plan's. */
export function approveSentence(request: RefundRequestView): string {
	const refund = `Refund ${money(request.quoted_cents, request.currency)} to ${cardOf(request.org_name)} card?`;
	return request.policy_basis === 'yearly_unused_months'
		? `${refund} We'll email them, and a yearly plan ends today.`
		: `${refund} We'll email them.`;
}

/** "Approved by Sam on 3 Oct 2026"; null while the request is open. */
export function decidedLine(request: RefundRequestView): string | null {
	if (request.status === 'requested') return null;
	const verb = request.status === 'approved' ? 'Approved' : 'Declined';
	const by = request.decided_by ? ` by ${request.decided_by}` : '';
	const on = request.decided_at === null ? '' : ` on ${nzShortDate(request.decided_at)}`;
	return `${verb}${by}${on}`;
}

/** What is wrong with a decline reason; null when it can be sent. */
export function declineProblem(reason: string): string | null {
	return reason.trim() === '' ? 'Say why, in a sentence they will read.' : null;
}

/** The body Decline sends. Only call once `declineProblem` is null. */
export function declineBody(reason: string, decidedBy: string): DecisionBody {
	return { reason: reason.trim(), decided_by_label: decidedBy };
}

// ------------------------------------------------------------- refund mail

export interface MailButton {
	label: string;
	disabled: boolean;
	/** Why it cannot be pressed, where it cannot. */
	reason?: string;
	/** The last send failure, where the email has not gone out; the button
	 *  then sends it again. */
	failure: string | null;
}

const NO_ORG = "This payment isn't linked to an organisation, so there is nobody to email.";

/** The "Email the customer" control on one refund: Send email, Queued, or
 *  Sent on its day. A send that failed says Not sent yet and can be retried. */
export function mailButton(refund: RefundView): MailButton {
	if (refund.mail_sent_at !== null) {
		return {
			label: `Sent on ${nzLongDate(refund.mail_sent_at)}`,
			disabled: true,
			reason: 'This email has already gone out.',
			failure: null
		};
	}
	if (refund.mail_error !== null) {
		return refund.org_id === null
			? { label: 'Send email', disabled: true, reason: NO_ORG, failure: refund.mail_error }
			: { label: 'Send email', disabled: false, failure: refund.mail_error };
	}
	if (refund.mail_requested_at !== null) {
		return {
			label: 'Queued',
			disabled: true,
			reason: 'This email is waiting to go out.',
			failure: null
		};
	}
	if (refund.org_id === null) {
		return { label: 'Send email', disabled: true, reason: NO_ORG, failure: null };
	}
	return { label: 'Send email', disabled: false, failure: null };
}
