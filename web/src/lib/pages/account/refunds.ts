/**
 * The billing page's "Ask for a refund": which payments a seller can ask
 * about, how their requests read, what the ask sheet offers once the policy
 * has quoted a payment, and the body it sends.
 *
 * The server quotes and checks every request; this decides what to show.
 */

import type { Tone } from '$lib/StatusPill.svelte';
import { money } from '$lib/pages/account/plans';
import {
	nzShortDate,
	type QuoteView,
	type RefundRequestStatus,
	type RefundRequestView
} from '$lib/pages/admin/payments';

/** One payment from the last year the seller can ask about. */
export interface RefundablePayment {
	charge_id: string;
	amount_cents: number;
	currency: string;
	paid_at: number;
	refunded_cents: number;
	/** "Pro yearly plan"; null for a payment the policy does not recognise. */
	what: string | null;
}

export interface BillingRefundsView {
	payments: RefundablePayment[];
	requests: RefundRequestView[];
}

export interface AskBody {
	charge_id: string;
	note: string;
}

/** The longest note the server keeps. */
export const NOTE_MAX = 1000;

export interface PaymentRow {
	id: string;
	date: string;
	amount: string;
	what: string;
	/** "$10.00 already refunded", where some of it has been. */
	refunded: string | null;
}

/** The payments as the sheet's radio list reads them. */
export function paymentRows(payments: readonly RefundablePayment[]): PaymentRow[] {
	return payments.map((payment) => ({
		id: payment.charge_id,
		date: nzShortDate(payment.paid_at),
		amount: money(payment.amount_cents, payment.currency),
		what: payment.what ?? 'One-off payment',
		refunded:
			payment.refunded_cents > 0
				? `${money(payment.refunded_cents, payment.currency)} already refunded`
				: null
	}));
}

const STATUS: Record<RefundRequestStatus, { label: string; tone: Tone }> = {
	requested: { label: 'Waiting', tone: 'run' },
	approved: { label: 'Refunded', tone: 'ok' },
	declined: { label: 'Declined', tone: 'flat' }
};

export interface RequestRow {
	id: string;
	amount: string;
	asked: string;
	status: { label: string; tone: Tone };
	/** Why it was declined, as we emailed it. */
	declined: string | null;
}

/** The seller's requests, as the billing page lists them under the button. */
export function requestRows(requests: readonly RefundRequestView[]): RequestRow[] {
	return requests.map((request) => ({
		id: request.id,
		amount: money(request.quoted_cents, request.currency),
		asked: `Asked ${nzShortDate(request.created_at)}`,
		status: STATUS[request.status],
		declined: request.status === 'declined' ? request.decline_reason : null
	}));
}

/** What the sheet offers once a payment is quoted: a button asking for the
 *  policy's amount, or nothing to ask for. */
export type AskOffer = { kind: 'ask'; button: string } | { kind: 'nothing' };

export function askOffer(quote: QuoteView): AskOffer {
	return quote.amount_cents > 0
		? { kind: 'ask', button: `Ask for ${money(quote.amount_cents, quote.currency)}` }
		: { kind: 'nothing' };
}

export function askBody(chargeId: string, note: string): AskBody {
	return { charge_id: chargeId, note: note.trim().slice(0, NOTE_MAX) };
}
