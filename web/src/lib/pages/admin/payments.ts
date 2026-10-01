/**
 * The admin Payments page's wire shapes and the arithmetic it does on them:
 * which rows the filters keep, this month's figures, what is left to refund
 * on a charge, which rows belong together, and what the refund form accepts
 * before it sends.
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
}

export interface PaymentsAdminView {
	/** Newest first, the last 400 days at most. */
	events: PaymentEventView[];
	/** Newest first. */
	refunds: RefundView[];
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
}

export type RefundField = 'amount' | 'reason';

/** The panel's opening state: the whole of what is left, and the auto-mail setting. */
export function refundDraft(left: number, autoMail: boolean): RefundDraft {
	return { dollars: left / 100, reason: '', note: '', sendEmail: autoMail };
}

/** Dollars as typed, in cents; null where it is not a whole number of cents. */
export function toCents(dollars: number | null): number | null {
	if (dollars === null || !Number.isFinite(dollars)) return null;
	const cents = Math.round(dollars * 100);
	return Math.abs(cents - dollars * 100) < 1e-6 ? cents : null;
}

/** What is wrong with the refund form, by field; empty when it can be sent. */
export function refundProblems(
	draft: RefundDraft,
	left: number,
	currency: string
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
	return problems;
}

/** The body the refund is sent as. Only call once `refundProblems` is empty. */
export function refundBody(draft: RefundDraft, requestId: string, issuedBy: string): RefundBody {
	return {
		request_id: requestId,
		amount_cents: toCents(draft.dollars) ?? 0,
		reason: draft.reason === '' ? 'requested_by_customer' : draft.reason,
		note: draft.note.trim(),
		send_email: draft.sendEmail,
		issued_by_label: issuedBy
	};
}

/** "Refund $12.00 to Kiwi Kids's card?" */
export function confirmSentence(cents: number, currency: string, orgName: string | null): string {
	const whose =
		orgName === null || orgName.trim() === '' ? "the customer's" : `${orgName.trim()}'s`;
	return `Refund ${money(cents, currency)} to ${whose} card?`;
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
