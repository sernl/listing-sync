import { describe, expect, it } from 'vitest';
import { discountFigure, durationLabel, termsBody, termsRefusal, type TermsDraft } from './pricing';

const halloween: TermsDraft = {
	name: 'Halloween 2026',
	by: 'percent',
	percent: 25,
	dollars: null,
	duration: 'once',
	months: null,
	keys: [],
	from: '2026-10-01',
	until: '2026-10-31'
};

describe('the pricing form', () => {
	it('accepts the Halloween sale a week before it opens', () => {
		expect(termsRefusal(halloween, '2026-09-27')).toBeNull();
	});

	it('accepts a one-day window and a window that is already running', () => {
		expect(termsRefusal({ ...halloween, until: '2026-10-01' }, '2026-09-27')).toBeNull();
		expect(termsRefusal(halloween, '2026-10-31')).toBeNull();
	});

	it('refuses a window that ends before it starts or has already closed', () => {
		expect(termsRefusal({ ...halloween, until: '2026-09-30' }, '2026-09-27')).toMatch(/last day/);
		expect(termsRefusal(halloween, '2026-11-01')).toMatch(/already closed/);
	});

	it('refuses percentages outside 1 to 100 and months outside 1 to 36', () => {
		expect(termsRefusal({ ...halloween, percent: 0 }, '2026-09-27')).not.toBeNull();
		expect(termsRefusal({ ...halloween, percent: 101 }, '2026-09-27')).not.toBeNull();
		expect(termsRefusal({ ...halloween, percent: 12.5 }, '2026-09-27')).not.toBeNull();
		expect(
			termsRefusal({ ...halloween, duration: 'repeating', months: 37 }, '2026-09-27')
		).not.toBeNull();
	});

	it('sends dollars as cents and months only when repeating', () => {
		const body = termsBody({
			...halloween,
			by: 'amount',
			dollars: 9.99,
			duration: 'repeating',
			months: 3
		});
		expect(body).toMatchObject({ amount_off_cents: 999, duration: 'repeating', duration_months: 3 });
		expect(body).not.toHaveProperty('percent_off');
		expect(termsBody(halloween)).not.toHaveProperty('duration_months');
	});
});

describe('a discount row', () => {
	it('leads with its figure and says which payments it reaches', () => {
		expect(discountFigure({ percent_off: 25, amount_off_cents: null })).toBe('25% off');
		expect(discountFigure({ percent_off: null, amount_off_cents: 1050 })).toBe('$10.50 off');
		expect(durationLabel({ duration: 'once', duration_months: null })).toBe('first payment');
		expect(durationLabel({ duration: 'repeating', duration_months: 3 })).toBe('first 3 months');
	});
});
