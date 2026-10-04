import { describe, expect, it } from 'vitest';
import type { QuoteView, RefundRequestView } from '$lib/pages/admin/payments';
import {
	NOTE_MAX,
	askBody,
	askOffer,
	paymentRows,
	requestRows,
	type RefundablePayment
} from './refunds';

const at = (iso: string) => Date.parse(iso);

function payment(over: Partial<RefundablePayment> = {}): RefundablePayment {
	return {
		charge_id: 'ch_a',
		amount_cents: 24000,
		currency: 'usd',
		paid_at: at('2026-06-01T00:00:00Z'),
		refunded_cents: 0,
		what: 'Pro yearly plan',
		...over
	};
}

function quote(over: Partial<QuoteView> = {}): QuoteView {
	return {
		charge_id: 'ch_a',
		org_id: 'org-kiwi',
		what: 'Pro yearly plan',
		paid_cents: 24000,
		currency: 'usd',
		paid_at: at('2026-06-01T00:00:00Z'),
		remaining_cents: 24000,
		amount_cents: 14000,
		basis: 'yearly_unused_months',
		explanation: '8 months unused, less one: 7 × $240.00 ÷ 12 = $140.00.',
		ends_plan: true,
		...over
	};
}

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

describe('the payments a seller can ask about', () => {
	it('reads each as its New Zealand day, amount and what was bought', () => {
		expect(paymentRows([payment()])).toEqual([
			{
				id: 'ch_a',
				date: '1 Jun 2026',
				amount: '$240.00',
				what: 'Pro yearly plan',
				refunded: null
			}
		]);
	});

	it('calls an unrecognised payment a one-off, and says what is already refunded', () => {
		const [row] = paymentRows([payment({ what: null, amount_cents: 1200, refunded_cents: 500 })]);
		expect(row.what).toBe('One-off payment');
		expect(row.refunded).toBe('$5.00 already refunded');
	});
});

describe("the seller's requests", () => {
	it('says Waiting, Refunded or Declined, with the decline reason', () => {
		const rows = requestRows([
			request({ id: 'a' }),
			request({ id: 'b', status: 'approved' }),
			request({ id: 'c', status: 'declined', decline_reason: 'The year is over.' })
		]);
		expect(rows.map((row) => row.status.label)).toEqual(['Waiting', 'Refunded', 'Declined']);
		expect(rows.map((row) => row.declined)).toEqual([null, null, 'The year is over.']);
		expect(rows[0]).toMatchObject({ amount: '$140.00', asked: 'Asked 2 Oct 2026' });
	});
});

describe('what the ask sheet offers', () => {
	it('asks for the policy amount when there is one', () => {
		expect(askOffer(quote())).toEqual({ kind: 'ask', button: 'Ask for $140.00' });
	});

	it('offers nothing to ask for when the policy amount is zero', () => {
		expect(askOffer(quote({ amount_cents: 0, basis: 'yearly_ended' }))).toEqual({
			kind: 'nothing'
		});
	});

	it('sends the charge and the note, trimmed and no longer than the server keeps', () => {
		expect(askBody('ch_a', '  It broke  ')).toEqual({ charge_id: 'ch_a', note: 'It broke' });
		expect(askBody('ch_a', '')).toEqual({ charge_id: 'ch_a', note: '' });
		expect(askBody('ch_a', 'x'.repeat(NOTE_MAX + 5)).note).toHaveLength(NOTE_MAX);
	});
});
