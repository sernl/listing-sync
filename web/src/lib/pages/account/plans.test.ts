import { describe, expect, it } from 'vitest';
import type { Founding, Pack } from '$lib/generated/plans';
import { AI, PACKS, PLANS } from '$lib/generated/plans';
import {
	bestValuePack,
	checkoutOutcome,
	dayLabel,
	dollars,
	expiryLine,
	foundingClosesLabel,
	foundingOpen,
	moves,
	packsBySize,
	perMonth,
	planBullets,
	renewsLine,
	syncPlan
} from './plans';

// The price table is `tam-limits`' and arrives generated, so the fixtures
// here stand in for it wherever a figure would otherwise be asserted twice —
// once in this file and once in Rust, where the invariants already live. The
// two places the real table is read are the orderings, which are properties
// of the founder's own prices rather than of this rendering.

function pack(over: Partial<Pack> = {}): Pack {
	return { key: 'pack_20', moves: 20, price_cents: 4700, per_move_cents: 235, ...over };
}

function founding(over: Partial<Founding> = {}): Founding {
	return {
		discount_year_one_pct: 25,
		discount_ongoing_pct: 20,
		ongoing_years: 3,
		year_one_cents: 18000,
		ongoing_cents: 19200,
		closes_at: '2026-12-31',
		annual_only: true,
		extra_moves: 20,
		places: 100,
		...over
	};
}

const CLOSING_DAY = Date.parse('2026-12-31T12:00:00Z');

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
		expect(ordered.map((row) => row.moves)).toEqual([...ordered.map((row) => row.moves)].sort((one, two) => one - two));
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

describe('the founding offer', () => {
	it('is on sale on its closing day, because an offer closing on the 31st is sold on the 31st', () => {
		expect(foundingOpen(founding(), CLOSING_DAY)).toBe(true);
	});

	it('is gone the day after', () => {
		expect(foundingOpen(founding(), Date.parse('2027-01-01T00:00:00Z'))).toBe(false);
	});

	it('is on sale well before', () => {
		expect(foundingOpen(founding(), Date.parse('2026-01-05T00:00:00Z'))).toBe(true);
	});

	it('states the day it closes', () => {
		expect(foundingClosesLabel(founding())).toBe('Closes 31 Dec 2026.');
	});
});

describe('what the billing read says in words', () => {
	it('names the day the soonest moves lapse', () => {
		expect(expiryLine({ available: 12, expiring_soonest: Date.parse('2027-03-03T00:00:00Z') })).toBe(
			'Moves start expiring on 3 Mar 2027.'
		);
	});

	it('says nothing where no part of the balance lapses', () => {
		expect(expiryLine({ available: 12 })).toBeNull();
	});

	it('names the renewal day', () => {
		expect(renewsLine(Date.parse('2027-03-03T00:00:00Z'))).toBe('Renews on 3 Mar 2027.');
	});

	it('says nothing where nothing renews', () => {
		expect(renewsLine(undefined)).toBeNull();
	});

	it('prints a date in the server’s own zone, so a seller east of UTC reads the enforced day', () => {
		expect(dayLabel(Date.parse('2027-03-03T23:30:00Z'))).toBe('3 Mar 2027');
	});
});

describe('the plan a deployment sells', () => {
	it('is Sync, which is the one recurring plan on sale', () => {
		const plan = syncPlan();
		expect(plan?.id).toBe('subscriber');
		expect(plan?.name).toBe('Sync');
	});

	it('is never Studio, which ships priced and unsold', () => {
		expect(syncPlan()?.id).not.toBe('studio');
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
