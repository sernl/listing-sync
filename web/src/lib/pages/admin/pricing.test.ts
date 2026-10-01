import { describe, expect, it } from 'vitest';
import {
	codeRows,
	discountFigure,
	durationLabel,
	formPreview,
	formProblems,
	termsBody,
	windowLabel,
	type DiscountView,
	type PricingForm,
	type TermsDraft
} from './pricing';

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

function form(over: Partial<PricingForm> = {}, terms: Partial<TermsDraft> = {}): PricingForm {
	return {
		kind: 'sale',
		terms: { ...halloween, ...terms },
		banner: 'Halloween sale',
		bannerHref: '',
		theme: 'halloween',
		code: '',
		limit: null,
		...over
	};
}

describe('the pricing form', () => {
	it('accepts the Halloween sale a week before it opens', () => {
		expect(formProblems(form(), '2026-09-27')).toEqual({});
	});

	it('accepts a one-day window and a window that is already running', () => {
		expect(formProblems(form({}, { until: '2026-10-01' }), '2026-09-27')).toEqual({});
		expect(formProblems(form(), '2026-10-31')).toEqual({});
	});

	it('puts a bad window on the last day, and says which way it is wrong', () => {
		expect(formProblems(form({}, { until: '2026-09-30' }), '2026-09-27').until).toMatch(/last day/);
		expect(formProblems(form(), '2026-11-01').until).toMatch(/already closed/);
		expect(formProblems(form({}, { from: '', until: '' }), '2026-09-27')).toMatchObject({
			from: expect.any(String),
			until: expect.any(String)
		});
	});

	it('refuses percentages outside 1 to 100 and months outside 1 to 36', () => {
		for (const percent of [0, 101, 12.5, null]) {
			expect(formProblems(form({}, { percent }), '2026-09-27')).toHaveProperty('percent');
		}
		expect(
			formProblems(form({}, { duration: 'repeating', months: 37 }), '2026-09-27')
		).toHaveProperty('months');
	});

	it('names every wrong field at once, so each can say so beneath itself', () => {
		const problems = formProblems(
			form({ kind: 'code', code: 'no', limit: 0 }, { name: '', by: 'amount', dollars: 0 }),
			'2026-09-27'
		);
		expect(Object.keys(problems).sort()).toEqual(['code', 'dollars', 'limit', 'name']);
	});

	it('checks only what the chosen kind shows', () => {
		// A sale has no prices to pick and a code has no banner.
		expect(formProblems(form({}, { keys: [] }), '2026-09-27')).not.toHaveProperty('keys');
		expect(
			formProblems(form({ kind: 'code', code: 'TEACHER10', banner: '' }), '2026-09-27')
		).toEqual({});
		expect(formProblems(form({ kind: 'one_off' }), '2026-09-27')).toHaveProperty('keys');
		expect(formProblems(form({ banner: ' ' }), '2026-09-27')).toHaveProperty('banner');
	});

	it('sends dollars as cents and months only when repeating', () => {
		const body = termsBody({
			...halloween,
			by: 'amount',
			dollars: 9.99,
			duration: 'repeating',
			months: 3
		});
		expect(body).toMatchObject({
			amount_off_cents: 999,
			duration: 'repeating',
			duration_months: 3
		});
		expect(body).not.toHaveProperty('percent_off');
		expect(termsBody(halloween)).not.toHaveProperty('duration_months');
	});
});

describe('the preview line', () => {
	it('says a sale as it will run', () => {
		expect(formPreview(form())).toBe('25% off every plan, 1–31 Oct, banner “Halloween sale”');
	});

	it('shows what is still missing as a gap rather than a zero', () => {
		expect(formPreview(form({ banner: '' }, { percent: null, from: '', until: '' }))).toBe(
			'…% off every plan, no dates yet, no banner yet'
		);
	});

	it('leads a code with the code and ends it with its use limit', () => {
		expect(
			formPreview(
				form(
					{ kind: 'code', code: 'TEACHER10', limit: 50 },
					{ by: 'amount', dollars: 10, keys: ['pro_monthly'], duration: 'repeating', months: 3 }
				)
			)
		).toBe('TEACHER10: $10 off pro_monthly, first 3 months, 1–31 Oct, up to 50 uses');
	});

	it('says a window across months and years in full', () => {
		expect(windowLabel('2026-10-28', '2026-11-03')).toBe('28 Oct – 3 Nov');
		expect(windowLabel('2026-12-28', '2027-01-03')).toBe('28 Dec 2026 – 3 Jan 2027');
		expect(windowLabel('2026-10-01', '2026-10-01')).toBe('1 Oct');
	});
});

function discount(over: Partial<DiscountView> = {}): DiscountView {
	return {
		id: 'd1',
		kind: 'code',
		name: 'Teachers',
		percent_off: 10,
		amount_off_cents: null,
		currency: 'usd',
		duration: 'once',
		duration_months: null,
		price_keys: [],
		from: '2026-10-01',
		until: '2026-10-31',
		state: 'open',
		stripe_coupon_id: 'c1',
		in_stripe: true,
		banner: null,
		banner_href: null,
		theme: null,
		codes: [],
		...over
	};
}

describe('the codes table', () => {
	it('has a row per typed code, live ones first, and ends the discount behind it', () => {
		const rows = codeRows([
			discount({ id: 'old', state: 'over', codes: [code('a', 'OLD10')] }),
			discount({ kind: 'sale', id: 'sale' }),
			discount({
				id: 'live',
				codes: [code('b', 'TEACHER10', 50), code('c', 'GONE', null, true)]
			})
		]);
		expect(rows.map((row) => [row.code, row.state, row.live, row.discountId])).toEqual([
			['TEACHER10', 'open', true, 'live'],
			['OLD10', 'over', false, 'old'],
			['GONE', 'ended', false, 'live']
		]);
		expect(rows[0]).toMatchObject({
			off: '10% off, first payment',
			window: '1–31 Oct',
			uses: 'Up to 50'
		});
		expect(rows[2].uses).toBe('No limit');
	});
});

function code(id: string, typed: string, max: number | null = null, ended = false) {
	return { id, code: typed, max_redemptions: max, stripe_promotion_code_id: `p-${id}`, ended };
}

describe('a discount row', () => {
	it('leads with its figure and says which payments it reaches', () => {
		expect(discountFigure({ percent_off: 25, amount_off_cents: null })).toBe('25% off');
		expect(discountFigure({ percent_off: null, amount_off_cents: 1050 })).toBe('$10.50 off');
		expect(durationLabel({ duration: 'once', duration_months: null })).toBe('first payment');
		expect(durationLabel({ duration: 'repeating', duration_months: 3 })).toBe('first 3 months');
	});
});
