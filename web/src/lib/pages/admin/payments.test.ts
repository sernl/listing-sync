import { describe, expect, it } from 'vitest';
import {
	NO_FILTERS,
	confirmSentence,
	eventPill,
	filterEvents,
	mailButton,
	monthStats,
	nzDay,
	refundBody,
	refundDraft,
	refundOf,
	refundProblems,
	relatedEvents,
	relatedRefunds,
	remaining,
	tallyAmount,
	type PaymentEventView,
	type RefundView
} from './payments';

const at = (iso: string) => Date.parse(iso);

function event(over: Partial<PaymentEventView> & Pick<PaymentEventView, 'id'>): PaymentEventView {
	return {
		org_id: 'org-kiwi',
		org_name: 'Kiwi Kids',
		kind: 'payment_succeeded',
		amount_cents: 2900,
		currency: 'usd',
		provider_object_id: `ch_${over.id}`,
		charge_id: `ch_${over.id}`,
		payment_intent_id: null,
		invoice_id: null,
		customer_id: 'cus_1',
		status: 'succeeded',
		reason: null,
		occurred_at: at('2026-10-02T00:00:00Z'),
		...over
	};
}

function refund(over: Partial<RefundView> & Pick<RefundView, 'id' | 'charge_id'>): RefundView {
	return {
		org_id: 'org-kiwi',
		org_name: 'Kiwi Kids',
		provider_refund_id: `re_${over.id}`,
		amount_cents: 1000,
		currency: 'usd',
		status: 'succeeded',
		reason: 'requested_by_customer',
		note: null,
		issued_by: 'Sam',
		mail_requested_at: null,
		mail_sent_at: null,
		mail_error: null,
		created_at: at('2026-10-02T00:00:00Z'),
		updated_at: at('2026-10-02T00:00:00Z'),
		...over
	};
}

const paid = event({ id: 'a', org_name: 'Kiwi Kids', occurred_at: at('2026-10-05T01:00:00Z') });
const failed = event({
	id: 'b',
	kind: 'payment_failed',
	status: 'failed',
	org_id: 'org-tui',
	org_name: 'Tui Teaching',
	occurred_at: at('2026-09-10T01:00:00Z')
});
const dispute = event({
	id: 'c',
	kind: 'dispute_opened',
	provider_object_id: 'dp_1',
	charge_id: 'ch_a',
	status: 'needs_response',
	occurred_at: at('2026-10-06T01:00:00Z')
});
const invoice = event({
	id: 'd',
	kind: 'invoice_paid',
	provider_object_id: 'in_1',
	charge_id: null,
	invoice_id: 'in_1',
	status: 'paid',
	org_id: null,
	org_name: null,
	occurred_at: at('2026-08-01T01:00:00Z')
});
const all = [paid, failed, dispute, invoice];

describe('the filters', () => {
	const ids = (filters: Partial<typeof NO_FILTERS>) =>
		filterEvents(all, { ...NO_FILTERS, ...filters }).map((row) => row.id);

	it('keeps everything with no filter set', () => {
		expect(ids({})).toEqual(['a', 'b', 'c', 'd']);
	});

	it('takes a kind as a group or as one exact kind', () => {
		expect(ids({ kind: 'payments' })).toEqual(['a', 'b']);
		expect(ids({ kind: 'payment_failed' })).toEqual(['b']);
		expect(ids({ kind: 'disputes' })).toEqual(['c']);
		expect(ids({ kind: 'refunds' })).toEqual([]);
	});

	it("matches Stripe's exact status word", () => {
		expect(ids({ status: 'paid' })).toEqual(['d']);
		expect(ids({ status: 'succeed' })).toEqual([]);
	});

	it('finds an organisation by part of its name or id, in any case, and skips unlinked rows', () => {
		expect(ids({ org: 'tui' })).toEqual(['b']);
		expect(ids({ org: 'ORG-KIWI' })).toEqual(['a', 'c']);
		expect(ids({ org: '  ' })).toEqual(['a', 'b', 'c', 'd']);
	});

	it('takes the date range as inclusive New Zealand days', () => {
		// 2026-09-30T12:30Z is 1 October in Auckland.
		const lateUtc = event({ id: 'e', occurred_at: at('2026-09-30T12:30:00Z') });
		const rows = [lateUtc];
		expect(filterEvents(rows, { ...NO_FILTERS, from: '2026-10-01' })).toHaveLength(1);
		expect(filterEvents(rows, { ...NO_FILTERS, to: '2026-09-30' })).toHaveLength(0);
		expect(
			filterEvents(rows, { ...NO_FILTERS, from: '2026-10-01', to: '2026-10-01' })
		).toHaveLength(1);
	});

	it('applies every dimension together', () => {
		expect(ids({ kind: 'payments', status: 'succeeded', org: 'kiwi', from: '2026-10-01' })).toEqual(
			['a']
		);
		expect(ids({ kind: 'payments', org: 'kiwi', to: '2026-09-30' })).toEqual([]);
	});
});

describe('this month', () => {
	const now = at('2026-10-15T00:00:00Z');

	it('counts by the New Zealand month, so late 30 September UTC is October', () => {
		const edge = event({ id: 'e', amount_cents: 500, occurred_at: at('2026-09-30T12:30:00Z') });
		const before = event({ id: 'f', amount_cents: 700, occurred_at: at('2026-09-30T10:30:00Z') });
		expect(nzDay(edge.occurred_at)).toBe('2026-10-01');
		const stats = monthStats([edge, before], [], now);
		expect(stats.month).toBe('2026-10');
		expect(stats.paid).toEqual({ count: 1, cents: { usd: 500 } });
	});

	it('adds payments, refunds that gave money back, and opened chargebacks', () => {
		const refunds = [
			refund({ id: '1', charge_id: 'ch_a', amount_cents: 400, status: 'pending' }),
			refund({ id: '2', charge_id: 'ch_a', amount_cents: 300 }),
			refund({ id: '3', charge_id: 'ch_a', amount_cents: 999, status: 'failed' }),
			refund({ id: '4', charge_id: 'ch_a', created_at: at('2026-09-01T00:00:00Z') })
		];
		const stats = monthStats(all, refunds, now);
		expect(stats.paid).toEqual({ count: 1, cents: { usd: 2900 } });
		expect(stats.refunded).toEqual({ count: 2, cents: { usd: 700 } });
		expect(stats.disputed).toEqual({ count: 1, cents: { usd: 2900 } });
		expect(tallyAmount(stats.refunded)).toBe('$7.00');
		expect(tallyAmount({ count: 0, cents: {} })).toBe('$0.00');
	});
});

describe('what is left to refund', () => {
	it('takes partial refunds off what was paid', () => {
		const refunds = [
			refund({ id: '1', charge_id: 'ch_a', amount_cents: 1000 }),
			refund({ id: '2', charge_id: 'ch_a', amount_cents: 400, status: 'pending' }),
			refund({ id: '3', charge_id: 'ch_other', amount_cents: 400 })
		];
		expect(remaining('ch_a', all, refunds)).toBe(1500);
	});

	it('leaves failed and cancelled refunds out', () => {
		const refunds = [
			refund({ id: '1', charge_id: 'ch_a', amount_cents: 1000, status: 'failed' }),
			refund({ id: '2', charge_id: 'ch_a', amount_cents: 1000, status: 'canceled' })
		];
		expect(remaining('ch_a', all, refunds)).toBe(2900);
	});

	it('is never below zero, and zero for a charge with no payment', () => {
		const refunds = [refund({ id: '1', charge_id: 'ch_a', amount_cents: 5000 })];
		expect(remaining('ch_a', all, refunds)).toBe(0);
		expect(remaining('ch_missing', all, [])).toBe(0);
	});

	it('marks a payment refunded in full Refunded, and part of one Partly refunded', () => {
		expect(eventPill(paid, 0)).toEqual({ tone: 'soon', label: 'Refunded' });
		expect(eventPill(paid, 100).label).toBe('Partly refunded');
		expect(eventPill(paid, 2900).label).toBe('Paid');
		expect(eventPill(dispute, null)).toEqual({ tone: 'bad', label: 'Chargeback: needs response' });
	});
});

describe('related rows', () => {
	const intent = event({
		id: 'g',
		kind: 'invoice_paid',
		provider_object_id: 'in_2',
		charge_id: 'ch_a',
		payment_intent_id: 'pi_1',
		invoice_id: 'in_2'
	});
	const viaIntent = event({
		id: 'h',
		kind: 'payment_failed',
		provider_object_id: 'pi_1',
		charge_id: null,
		payment_intent_id: null
	});
	const refundEvent = event({
		id: 'i',
		kind: 'refund_created',
		provider_object_id: 're_1',
		charge_id: 'ch_a'
	});
	const rows = [paid, failed, dispute, invoice, intent, viaIntent, refundEvent];

	it('cross-references by charge, payment intent, invoice, or the object an id names', () => {
		expect(relatedEvents(intent, rows).map((row) => row.id)).toEqual(['a', 'c', 'h', 'i']);
		expect(relatedEvents(paid, rows).map((row) => row.id)).toEqual(['c', 'g', 'i']);
		expect(relatedEvents(failed, rows)).toEqual([]);
	});

	it('lists the refunds on the charge and links a refund event to its refund', () => {
		const refunds = [
			refund({ id: '1', charge_id: 'ch_a' }),
			refund({ id: '2', charge_id: 'ch_b' })
		];
		expect(relatedRefunds(paid, refunds).map((row) => row.id)).toEqual(['1']);
		expect(relatedRefunds(invoice, refunds)).toEqual([]);
		expect(refundOf(refundEvent, refunds)?.id).toBe('1');
		expect(refundOf(paid, refunds)).toBeNull();
	});
});

describe('the refund form', () => {
	it('opens with what is left and the auto-mail setting', () => {
		expect(refundDraft(1250, true)).toEqual({
			dollars: 12.5,
			reason: '',
			note: '',
			sendEmail: true
		});
	});

	it('accepts one cent up to what is left', () => {
		const draft = { ...refundDraft(1200, false), reason: 'duplicate' as const };
		expect(refundProblems({ ...draft, dollars: 0.01 }, 1200, 'usd')).toEqual({});
		expect(refundProblems({ ...draft, dollars: 12 }, 1200, 'usd')).toEqual({});
	});

	it('refuses more than is left with the sentence the server uses', () => {
		const draft = { ...refundDraft(1200, false), reason: 'duplicate' as const, dollars: 12.01 };
		expect(refundProblems(draft, 1200, 'usd').amount).toBe(
			'You can refund at most $12.00 on this payment.'
		);
	});

	it('refuses nothing, less than nothing, and fractions of a cent', () => {
		const draft = { ...refundDraft(1200, false), reason: 'duplicate' as const };
		for (const dollars of [null, 0, -1, Number.NaN]) {
			expect(refundProblems({ ...draft, dollars }, 1200, 'usd').amount).toBe(
				'Enter an amount greater than zero.'
			);
		}
		expect(refundProblems({ ...draft, dollars: 1.005 }, 1200, 'usd').amount).toMatch(/cents/);
	});

	it('needs a reason', () => {
		expect(refundProblems(refundDraft(1200, false), 1200, 'usd')).toEqual({
			reason: 'Pick a reason.'
		});
	});

	it('sends dollars as cents, with the request id and who issued it', () => {
		const draft = {
			dollars: 19.99,
			reason: 'fraudulent' as const,
			note: ' card stolen ',
			sendEmail: true
		};
		expect(refundBody(draft, 'req-1', 'Sam')).toEqual({
			request_id: 'req-1',
			amount_cents: 1999,
			reason: 'fraudulent',
			note: 'card stolen',
			send_email: true,
			issued_by_label: 'Sam'
		});
	});

	it('asks to confirm in one sentence', () => {
		expect(confirmSentence(1200, 'usd', 'Kiwi Kids')).toBe("Refund $12.00 to Kiwi Kids's card?");
		expect(confirmSentence(1200, 'usd', null)).toBe("Refund $12.00 to the customer's card?");
	});
});

describe('the refund email button', () => {
	const base = refund({ id: '1', charge_id: 'ch_a' });

	it('can be sent once, waits while queued, and says when it went out', () => {
		expect(mailButton(base)).toEqual({ label: 'Send email', disabled: false, failure: null });
		expect(mailButton({ ...base, mail_requested_at: at('2026-10-02T00:00:00Z') })).toMatchObject({
			label: 'Queued',
			disabled: true
		});
		expect(
			mailButton({
				...base,
				mail_requested_at: at('2026-10-02T00:00:00Z'),
				mail_sent_at: at('2026-10-02T12:00:00Z')
			})
		).toMatchObject({ label: 'Sent on 3 October 2026', disabled: true });
	});

	it('offers a failed send again, with what went wrong, until it goes out', () => {
		const failed = {
			...base,
			mail_requested_at: at('2026-10-02T00:00:00Z'),
			mail_error: 'Mailbox unavailable'
		};
		expect(mailButton(failed)).toEqual({
			label: 'Send email',
			disabled: false,
			failure: 'Mailbox unavailable'
		});
		expect(mailButton({ ...failed, mail_sent_at: at('2026-10-03T00:00:00Z') }).failure).toBeNull();
	});

	it('has nobody to email without an organisation', () => {
		expect(mailButton({ ...base, org_id: null })).toMatchObject({
			disabled: true,
			reason: "This payment isn't linked to an organisation, so there is nobody to email."
		});
	});
});
