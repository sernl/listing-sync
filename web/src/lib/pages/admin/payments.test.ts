import { describe, expect, it } from 'vitest';
import {
	BASIS_LABEL,
	NO_FILTERS,
	approveSentence,
	basisLabel,
	confirmSentence,
	decidedLine,
	declineBody,
	declineProblem,
	differsFromQuote,
	eventPill,
	filterEvents,
	mailButton,
	monthStats,
	nzDay,
	planLine,
	quoteDraft,
	quoteWhat,
	refundBody,
	refundOf,
	refundPolicy,
	refundProblems,
	relatedEvents,
	relatedRefunds,
	remaining,
	requestPill,
	requestsInOrder,
	requote,
	tallyAmount,
	type PaymentEventView,
	type QuoteView,
	type RefundRequestView,
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
		policy_basis: null,
		quoted_cents: null,
		override_reason: null,
		...over
	};
}

function quote(over: Partial<QuoteView> = {}): QuoteView {
	return {
		charge_id: 'ch_a',
		org_id: 'org-kiwi',
		what: 'Move Pack of 50 moves',
		paid_cents: 1200,
		currency: 'usd',
		paid_at: at('2026-10-01T00:00:00Z'),
		remaining_cents: 1200,
		amount_cents: 1200,
		basis: 'pack_unused',
		explanation: 'No moves used and bought 4 days ago, so all of it comes back.',
		ends_plan: false,
		...over
	};
}

const yearly = quote({
	what: 'Pro yearly plan',
	paid_cents: 24000,
	remaining_cents: 24000,
	amount_cents: 14000,
	basis: 'yearly_unused_months',
	explanation: '8 months unused, less one: 7 × $240.00 ÷ 12 = $140.00.',
	ends_plan: true
});

function request(
	over: Partial<RefundRequestView> & Pick<RefundRequestView, 'id'>
): RefundRequestView {
	return {
		org_id: 'org-kiwi',
		org_name: 'Kiwi Kids',
		charge_id: 'ch_a',
		quoted_cents: 14000,
		currency: 'usd',
		policy_basis: 'yearly_unused_months',
		status: 'requested',
		note: null,
		decided_by: null,
		decided_at: null,
		decline_reason: null,
		refund_id: null,
		created_at: at('2026-10-02T00:00:00Z'),
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
	it('opens with the policy amount, the auto-mail setting and a yearly plan ending', () => {
		expect(quoteDraft(quote({ amount_cents: 1250 }), true)).toEqual({
			dollars: 12.5,
			reason: '',
			note: '',
			sendEmail: true,
			overrideReason: '',
			endPlan: false
		});
		expect(quoteDraft(yearly, false)).toMatchObject({ dollars: 140, endPlan: true });
	});

	it('moves the amount back to a newer quote and keeps everything typed', () => {
		const typed = { ...quoteDraft(yearly, true), dollars: 100, note: 'asked by phone' };
		expect(requote(typed, quote({ ...yearly, amount_cents: 12000 }))).toEqual({
			...typed,
			dollars: 120
		});
	});

	it('names what was bought, or that it matched nothing', () => {
		expect(quoteWhat(yearly)).toBe('Pro yearly plan');
		expect(quoteWhat(quote({ what: null, basis: 'unmatched' }))).toBe(
			'Not matched to a plan or pack'
		);
	});

	it('accepts one cent up to what is left', () => {
		const draft = { ...quoteDraft(quote(), false), reason: 'duplicate' as const };
		const q = quote({ amount_cents: 1 });
		expect(refundProblems({ ...draft, dollars: 0.01 }, 1200, 'usd', q)).toEqual({});
		expect(refundProblems(draft, 1200, 'usd', quote())).toEqual({});
	});

	it('refuses more than is left with the sentence the server uses', () => {
		const draft = {
			...quoteDraft(quote(), false),
			reason: 'duplicate' as const,
			dollars: 12.01,
			overrideReason: 'goodwill'
		};
		expect(refundProblems(draft, 1200, 'usd', quote()).amount).toBe(
			'You can refund at most $12.00 on this payment.'
		);
	});

	it('refuses nothing, less than nothing, and fractions of a cent', () => {
		const draft = { ...quoteDraft(quote(), false), reason: 'duplicate' as const };
		for (const dollars of [null, 0, -1, Number.NaN]) {
			expect(refundProblems({ ...draft, dollars }, 1200, 'usd', quote()).amount).toBe(
				'Enter an amount greater than zero.'
			);
		}
		expect(refundProblems({ ...draft, dollars: 1.005 }, 1200, 'usd', quote()).amount).toMatch(
			/cents/
		);
	});

	it('needs a reason', () => {
		expect(refundProblems(quoteDraft(quote(), false), 1200, 'usd', quote())).toEqual({
			reason: 'Pick a reason.'
		});
	});

	it('needs a reason of its own for an amount other than the policy amount', () => {
		const draft = { ...quoteDraft(yearly, false), reason: 'requested_by_customer' as const };
		expect(differsFromQuote(draft, yearly)).toBe(false);
		expect(refundProblems(draft, 24000, 'usd', yearly)).toEqual({});

		const more = { ...draft, dollars: 160 };
		expect(differsFromQuote(more, yearly)).toBe(true);
		expect(refundProblems(more, 24000, 'usd', yearly)).toEqual({
			override: "Say why you're refunding a different amount from the policy."
		});
		expect(refundProblems({ ...more, overrideReason: '  ' }, 24000, 'usd', yearly)).toHaveProperty(
			'override'
		);
		expect(
			refundProblems({ ...more, overrideReason: 'Two weeks of downtime' }, 24000, 'usd', yearly)
		).toEqual({});
	});

	it('does not compare an amount that is not yet whole cents', () => {
		expect(differsFromQuote({ ...quoteDraft(yearly, false), dollars: 1.005 }, yearly)).toBe(false);
	});

	it('sends dollars as cents, with the request id, who issued it and the quote', () => {
		const draft = {
			...quoteDraft(quote(), true),
			dollars: 12,
			reason: 'fraudulent' as const,
			note: ' card stolen '
		};
		expect(refundBody(draft, 'req-1', 'Sam', quote())).toEqual({
			request_id: 'req-1',
			amount_cents: 1200,
			reason: 'fraudulent',
			note: 'card stolen',
			send_email: true,
			issued_by_label: 'Sam',
			policy_basis: 'pack_unused',
			quoted_cents: 1200,
			end_plan: false
		});
	});

	it('sends why the amount differs only when it does', () => {
		const draft = {
			...quoteDraft(yearly, false),
			reason: 'requested_by_customer' as const,
			overrideReason: ' Two weeks of downtime '
		};
		expect(refundBody(draft, 'r', 'Sam', yearly)).not.toHaveProperty('override_reason');
		expect(refundBody({ ...draft, dollars: 160 }, 'r', 'Sam', yearly)).toMatchObject({
			amount_cents: 16000,
			quoted_cents: 14000,
			policy_basis: 'yearly_unused_months',
			override_reason: 'Two weeks of downtime'
		});
	});

	it('ends a yearly plan unless unticked, and never a plan the quote does not end', () => {
		const draft = { ...quoteDraft(yearly, false), reason: 'duplicate' as const };
		expect(refundBody(draft, 'r', 'Sam', yearly).end_plan).toBe(true);
		expect(refundBody({ ...draft, endPlan: false }, 'r', 'Sam', yearly).end_plan).toBe(false);
		expect(refundBody({ ...draft, endPlan: true }, 'r', 'Sam', quote()).end_plan).toBe(false);
	});

	it('asks to confirm in one sentence, and says when the plan ends', () => {
		expect(confirmSentence(1200, 'usd', 'Kiwi Kids')).toBe("Refund $12.00 to Kiwi Kids's card?");
		expect(confirmSentence(1200, 'usd', null)).toBe("Refund $12.00 to the customer's card?");
		const draft = quoteDraft(yearly, false);
		expect(planLine(draft, yearly)).toBe(
			'Their yearly plan ends today and the account moves to Look.'
		);
		expect(planLine({ ...draft, endPlan: false }, yearly)).toBe('Their yearly plan keeps running.');
		expect(planLine(quoteDraft(quote(), false), quote())).toBeNull();
	});
});

describe('policy bases', () => {
	it('has a short label for every basis', () => {
		expect(BASIS_LABEL.yearly_unused_months).toBe('Yearly plan, unused months less one');
		expect(BASIS_LABEL.pack_unused).toBe('Unused Move Pack, within 14 days');
		expect(BASIS_LABEL.unmatched).toBe('Not matched to a plan or pack');
		expect(Object.keys(BASIS_LABEL)).toHaveLength(8);
	});

	it('shows a basis this console does not know as written', () => {
		expect(basisLabel('monthly_started')).toBe('Monthly plan, month started');
		expect(basisLabel('pack_gifted')).toBe('pack gifted');
	});
});

describe('a refund that carried a quote', () => {
	it('says the policy amount, and why it differs where it does', () => {
		expect(refundPolicy(refund({ id: '1', charge_id: 'ch_a' }))).toBeNull();
		expect(
			refundPolicy(refund({ id: '1', charge_id: 'ch_a', amount_cents: 1000, quoted_cents: 1000 }))
		).toEqual({ quoted: 'Policy $10.00', override: null });
		expect(
			refundPolicy(
				refund({
					id: '1',
					charge_id: 'ch_a',
					amount_cents: 1500,
					quoted_cents: 1000,
					override_reason: 'Goodwill'
				})
			)
		).toEqual({ quoted: 'Policy $10.00', override: 'Goodwill' });
	});
});

describe('refund requests', () => {
	const open = request({ id: 'open' });
	const approved = request({
		id: 'approved',
		status: 'approved',
		decided_by: 'Sam',
		decided_at: at('2026-10-03T00:00:00Z')
	});
	const declined = request({
		id: 'declined',
		status: 'declined',
		decided_by: null,
		decided_at: at('2026-10-04T00:00:00Z'),
		decline_reason: 'You have used the year.'
	});

	it('puts open requests first and keeps the rest in order', () => {
		expect(requestsInOrder([approved, open, declined]).map((r) => r.id)).toEqual([
			'open',
			'approved',
			'declined'
		]);
	});

	it('pills each status', () => {
		expect(requestPill(open)).toEqual({ tone: 'warn', label: 'Open' });
		expect(requestPill(approved)).toEqual({ tone: 'ok', label: 'Approved' });
		expect(requestPill(declined)).toEqual({ tone: 'flat', label: 'Declined' });
	});

	it('confirms approval in one sentence, with the plan ending only for a yearly plan', () => {
		expect(approveSentence(open)).toBe(
			"Refund $140.00 to Kiwi Kids's card? We'll email them, and a yearly plan ends today."
		);
		expect(
			approveSentence({ ...open, quoted_cents: 1200, policy_basis: 'pack_unused', org_name: null })
		).toBe("Refund $12.00 to the customer's card? We'll email them.");
	});

	it('says who decided and when, New Zealand day', () => {
		expect(decidedLine(open)).toBeNull();
		expect(decidedLine(approved)).toBe('Approved by Sam on 3 Oct 2026');
		expect(decidedLine(declined)).toBe('Declined on 4 Oct 2026');
	});

	it('needs a decline reason, and sends it trimmed with who decided', () => {
		expect(declineProblem('  ')).toBe('Say why, in a sentence they will read.');
		expect(declineProblem('Out of the window')).toBeNull();
		expect(declineBody(' Out of the window ', 'Sam')).toEqual({
			reason: 'Out of the window',
			decided_by_label: 'Sam'
		});
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
