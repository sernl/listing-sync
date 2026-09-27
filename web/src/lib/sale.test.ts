import { describe, expect, it } from 'vitest';
import {
	afterPercentOff as landingAfterPercentOff,
	saleLine as landingSaleLine
} from '../../../apps/landing/src/sale.js';
import type { SaleView } from '$lib/generated/plans';
import { afterPercentOff, saleLine, salePrice } from './sale';

const halloween: SaleView = {
	percent_off: 25,
	until: '2026-10-31',
	banner: 'Halloween sale: 25% off every plan until 31 October',
	banner_href: null
};

describe('a sale price', () => {
	it('strikes the list price and shows what Stripe will charge', () => {
		expect(salePrice(halloween, 2900)).toEqual({ listCents: 2900, saleCents: 2175 });
		expect(salePrice(halloween, 24000)).toEqual({ listCents: 24000, saleCents: 18000 });
	});

	it('takes the sale off a yearly price before quoting it a month', () => {
		// $96 a year is $8 a month; 25% off the year is $72, $6 a month.
		expect(salePrice(halloween, 9600, 12)).toEqual({ listCents: 800, saleCents: 600 });
		// $480 a year: $40 a month, and $30 a month in the sale.
		expect(salePrice(halloween, 48000, 12)).toEqual({ listCents: 4000, saleCents: 3000 });
	});

	it('rounds the amount off to the cent, the way Stripe does', () => {
		// 15% of $9.99 is 149.85 cents; Stripe takes 150 off.
		expect(afterPercentOff(999, 15)).toBe(849);
		// 33% of $19 is 627 cents exactly.
		expect(afterPercentOff(1900, 33)).toBe(1273);
	});

	it('strikes nothing without a sale, on a free plan, or when nothing comes off', () => {
		expect(salePrice(null, 2900)).toBeNull();
		expect(salePrice(halloween, null)).toBeNull();
		expect(salePrice(halloween, 0)).toBeNull();
		expect(salePrice({ ...halloween, percent_off: 1 }, 1)).toBeNull();
	});

	it('never goes below nothing', () => {
		expect(afterPercentOff(2900, 100)).toBe(0);
	});

	it('names the last day in words', () => {
		expect(saleLine(halloween)).toBe('25% off until 31 October');
		expect(saleLine({ ...halloween, percent_off: 30, until: '2026-12-01' })).toBe(
			'30% off until 1 December'
		);
	});
});

describe("the landing's copy of the same sums", () => {
	it('gives the same sale price for every figure the console does', () => {
		for (const cents of [1, 99, 999, 1900, 2900, 4400, 24000, 44000]) {
			for (const percent of [1, 10, 15, 25, 33, 50, 100]) {
				expect(landingAfterPercentOff(cents, percent)).toBe(afterPercentOff(cents, percent));
			}
		}
	});

	it('says the same line', () => {
		expect(landingSaleLine(halloween)).toBe(saleLine(halloween));
	});
});
