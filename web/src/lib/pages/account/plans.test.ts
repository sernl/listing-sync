import { describe, expect, it } from 'vitest';
import type { Pack, PlanRow } from '$lib/generated/plans';
import { AI, PACKS, PLANS } from '$lib/generated/plans';
import {
	bestValuePack,
	cardLabel,
	checkoutOutcome,
	dayLabel,
	dollars,
	expiryLine,
	invoiceStatus,
	money,
	moves,
	packsBySize,
	perMonth,
	planBullets,
	planMeaning,
	paidPlans,
	termLine,
	tierPrice
} from './plans';

// The price table is `tam-limits`' and arrives generated, so the fixtures
// here stand in for it wherever a figure would otherwise be asserted twice —
// once in this file and once in Rust, where the invariants already live. The
// two places the real table is read are the orderings, which are properties
// of the founder's own prices rather than of this rendering.

function pack(over: Partial<Pack> = {}): Pack {
	return {
		key: 'pack_20',
		moves: 20,
		price_cents: 4700,
		per_move_cents: 235,
		...over
	};
}

describe('money printed from cents', () => {
	it('drops the decimals on a whole number of dollars', () => {
		expect(dollars(2900)).toBe('$29');
	});

	it('keeps them where the price is not whole, so $1.27 is not $1', () => {
		expect(dollars(127)).toBe('$1.27');
	});

	it('states the yearly price as the month it works out at', () => {
		expect(perMonth(24000)).toBe('$20');
	});
});

describe('the packs on sale', () => {
	it('marks the pack that costs least per move', () => {
		expect(bestValuePack(PACKS)?.key).toBe('pack_500');
	});

	it('marks the cheapest even where the table is not ordered', () => {
		const shuffled = [
			pack({ key: 'pack_50', moves: 50, per_move_cents: 154 }),
			pack({ key: 'pack_500', moves: 500, per_move_cents: 79 }),
			pack({ key: 'pack_20', moves: 20, per_move_cents: 235 })
		];
		expect(bestValuePack(shuffled)?.key).toBe('pack_500');
	});

	it('answers nothing where a deployment sells no packs', () => {
		expect(bestValuePack([])).toBeNull();
	});

	it('orders by size, and a bigger pack never costs more per move', () => {
		const ordered = packsBySize(PACKS);
		expect(ordered.map((row) => row.moves)).toEqual(
			[...ordered.map((row) => row.moves)].sort((one, two) => one - two)
		);
		for (let index = 1; index < ordered.length; index += 1) {
			expect(ordered[index]!.per_move_cents).toBeLessThan(ordered[index - 1]!.per_move_cents);
		}
	});

	it('leaves the table it was handed alone', () => {
		const table = [pack({ key: 'pack_50', moves: 50 }), pack({ key: 'pack_20', moves: 20 })];
		packsBySize(table);
		expect(table.map((row) => row.moves)).toEqual([50, 20]);
	});
});

describe('a count of moves', () => {
	it('says one move in the singular', () => {
		expect(moves(1)).toBe('1 move');
	});

	it('says none in the plural, because "0 move" is not English', () => {
		expect(moves(0)).toBe('0 moves');
	});

	it('says the rest in the plural', () => {
		expect(moves(25)).toBe('25 moves');
	});
});

describe('what the billing read says in words', () => {
	it('names the day the soonest moves lapse', () => {
		expect(
			expiryLine({
				available: 12,
				expiring_soonest: Date.parse('2027-03-03T00:00:00Z')
			})
		).toBe('Moves start expiring on 3 Mar 2027.');
	});

	it('says nothing where no part of the balance lapses', () => {
		expect(expiryLine({ available: 12 })).toBeNull();
	});

	it('names the renewal day while the subscription renews', () => {
		expect(termLine({ renews_at: Date.parse('2027-03-03T00:00:00Z') })).toBe(
			'Your subscription renews on 3 Mar 2027.'
		);
	});

	it('names the end day once cancelled, and never a renewal beside it', () => {
		expect(
			termLine({
				renews_at: Date.parse('2027-03-03T00:00:00Z'),
				ends_at: Date.parse('2027-03-03T00:00:00Z')
			})
		).toBe('Your plan ends on 3 Mar 2027 and will not renew.');
	});

	it('says nothing where nothing renews or ends', () => {
		expect(termLine({})).toBeNull();
	});

	it('prints an invoice total in its own currency and minor unit', () => {
		expect(money(34783, 'nzd')).toBe('NZ$347.83');
		expect(money(2900, 'usd')).toBe('$29.00');
		expect(money(3000, 'jpy'), 'yen has no minor unit, so 3000 is ¥3,000').toBe('¥3,000');
	});

	it('calls an unpaid open invoice due, and keeps a status Stripe adds later readable', () => {
		expect(invoiceStatus('open')).toEqual({ label: 'Due', tone: 'warn' });
		expect(invoiceStatus('paid')).toEqual({ label: 'Paid', tone: 'ok' });
		expect(invoiceStatus('refunded_someday').label).toBe('refunded_someday');
	});

	it('names a card by brand and last four, and an unknown brand as a card', () => {
		expect(cardLabel({ brand: 'visa', last4: '3115' })).toBe('Visa •••• 3115');
		expect(cardLabel({ brand: 'link', last4: '0000' })).toBe('Card •••• 0000');
	});

	it('says a plan in one line from what it gives', () => {
		const caps = PLANS[0].capabilities;
		expect(
			planMeaning({
				...caps,
				moves_per_month: 25,
				moves_accrual_cap: 75,
				free_moves_lifetime: 0
			})
		).toBe('25 moves a month, saving up to 75.');
		expect(
			planMeaning({
				...caps,
				moves_per_month: 0,
				moves_accrual_cap: 0,
				free_moves_lifetime: 5
			})
		).toBe('5 moves free to try, then packs as you need them.');
	});

	it('prints a date in the server’s own zone, so a seller east of UTC reads the enforced day', () => {
		expect(dayLabel(Date.parse('2027-03-03T23:30:00Z'))).toBe('3 Mar 2027');
	});
});

describe('the tier cards', () => {
	const row = (over: Partial<PlanRow> = {}): PlanRow => ({
		...PLANS[0],
		id: 'subscriber',
		name: 'Sync',
		monthly_cents: 2900,
		yearly_cents: 24000,
		monthly_key: 'sync_monthly',
		yearly_key: 'sync_yearly',
		...over
	});

	it('are every priced plan, cheapest first, and never the free one', () => {
		const tiers = paidPlans();
		expect(tiers.map((plan) => plan.id)).not.toContain('free');
		expect(tiers.length).toBeGreaterThanOrEqual(3);
		const monthly = tiers.map((plan) => plan.monthly_cents ?? 0);
		expect([...monthly].sort((a, b) => a - b)).toEqual(monthly);
	});

	it('lead yearly with the month it works out at, the saving, and the yearly key', () => {
		expect(tierPrice(row(), 'yearly')).toEqual({
			headline: '$20',
			per: 'a month, billed yearly',
			note: '$240 a year. You save $108.',
			key: 'sync_yearly'
		});
	});

	it('sell the monthly key at the monthly price when the toggle says monthly', () => {
		expect(tierPrice(row(), 'monthly')).toEqual({
			headline: '$29',
			per: 'a month',
			note: 'Or $20 a month if you pay yearly.',
			key: 'sync_monthly'
		});
	});

	it('have no price for a plan that charges nothing', () => {
		expect(
			tierPrice(
				row({ monthly_cents: null, yearly_cents: null, monthly_key: null, yearly_key: null }),
				'yearly'
			)
		).toBeNull();
	});
});

describe('the lines on a plan card', () => {
	const caps = (id: string) => PLANS.find((plan) => plan.id === id)!.capabilities;
	const texts = (id: string) => planBullets(caps(id)).map((line) => line.text);

	it('states a resource ceiling only where the plan has one', () => {
		expect(texts('free')).toContain(`Up to ${caps('free').resources_max} resources`);
		expect(texts('subscriber').some((text) => /^Up to \d+ resources$/.test(text))).toBe(false);
	});

	it('never prints the no-limit sentinel as a count', () => {
		expect(texts('studio').some((text) => text.includes('4294967295'))).toBe(false);
	});

	it("puts what every plan does first and the free plan's trial moves last", () => {
		const free = texts('free');
		expect(free[0]).toBe('Import from wherever you sell');
		expect(free[free.length - 1]).toMatch(/onto a marketplace of your choice$/);
		expect(texts('starter').slice(0, 2)).toEqual([
			'Import from wherever you sell',
			'Edit once, sync everywhere'
		]);
	});

	it('marks AI fill as not yet built only while it is coming soon', () => {
		expect(planBullets(caps('subscriber'), AI).some((line) => line.soon)).toBe(true);
		const live = { ...AI, status: 'live' } as unknown as typeof AI;
		expect(planBullets(caps('subscriber'), live).some((line) => line.soon)).toBe(false);
	});
});

describe('what the checkout came back with', () => {
	it('reads the success return', () => {
		expect(checkoutOutcome('success')).toBe('success');
	});

	it('reads the cancelled return', () => {
		expect(checkoutOutcome('cancel')).toBe('cancel');
	});

	it('treats a hand-typed value as no return at all', () => {
		expect(checkoutOutcome('done')).toBeNull();
		expect(checkoutOutcome(null)).toBeNull();
	});
});
