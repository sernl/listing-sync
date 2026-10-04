import { describe, expect, it } from 'vitest';
import { ApiFailure } from '$lib/http';
import {
	bars,
	cityLabel,
	conversionRate,
	dayLabel,
	failureOf,
	formatPercent,
	lineGeometry,
	niceMax,
	pageLabel,
	parseRange,
	referrerLabel,
	rowLabel,
	windowLabel,
	type DayPoint,
	type SiteRow
} from './site-analytics';

const row = (label: string | null, visitors: number, detail: string | null = null): SiteRow => ({
	label,
	detail,
	visitors,
	pageviews: visitors * 2
});

describe('the range', () => {
	it('reads the three offered ranges and defaults everything else to a week', () => {
		expect(parseRange('30d')).toBe('30d');
		expect(parseRange('90d')).toBe('90d');
		expect(parseRange('365d')).toBe('7d');
		expect(parseRange(null)).toBe('7d');
	});
});

describe('why there are no figures', () => {
	it('tells an unconfigured server from PostHog not answering', () => {
		expect(failureOf(new ApiFailure(503, null))).toBe('unconfigured');
		expect(failureOf(new ApiFailure(502, null))).toBe('upstream');
		expect(failureOf(new ApiFailure(401, null))).toBe('other');
		expect(failureOf(new TypeError('offline'))).toBe('other');
	});
});

describe('conversion', () => {
	it('is signups per visitor, and nothing with no visitors', () => {
		expect(conversionRate({ visitors: 200, pageviews: 500, signups: 5, cta_clicks: 9 })).toBe(
			0.025
		);
		expect(conversionRate({ visitors: 0, pageviews: 0, signups: 0, cta_clicks: 0 })).toBeNull();
	});

	it('shows one decimal under ten percent and whole numbers above', () => {
		expect(formatPercent(0.025)).toBe('2.5%');
		expect(formatPercent(0.1234)).toBe('12%');
		expect(formatPercent(0)).toBe('0.0%');
		expect(formatPercent(null)).toBe('—');
	});
});

describe('labels', () => {
	it('says Unknown where PostHog recorded nothing', () => {
		expect(rowLabel(null)).toBe('Unknown');
		expect(pageLabel(null)).toBe('Unknown');
		expect(referrerLabel(null)).toBe('Unknown');
	});

	it('names the home page and direct visits in plain words', () => {
		expect(pageLabel('/')).toBe('Home page (/)');
		expect(pageLabel('/pricing/')).toBe('/pricing/');
		expect(referrerLabel('$direct')).toBe('Direct or bookmarked');
		expect(referrerLabel('www.google.com')).toBe('www.google.com');
	});

	it('puts the country beside a city when there is one', () => {
		expect(cityLabel(row('Auckland', 3, 'New Zealand'))).toBe('Auckland, New Zealand');
		expect(cityLabel(row('Auckland', 3))).toBe('Auckland');
		expect(cityLabel(row(null, 3, 'New Zealand'))).toBe('Unknown, New Zealand');
	});

	it('dates the window as the New Zealand days it names', () => {
		expect(dayLabel('2026-10-04')).toMatch(/Sun.*4.*Oct/);
		expect(windowLabel({ from: '2026-09-28', to: '2026-10-04' })).toMatch(/28 Sept?.*–.*4 Oct/);
	});
});

describe('bars', () => {
	it('measures each row against the most visitors', () => {
		const drawn = bars([row('Mobile', 40), row('Desktop', 10), row(null, 0)], (r) =>
			rowLabel(r.label)
		);
		expect(drawn.map((bar) => bar.share)).toEqual([1, 0.25, 0]);
		expect(drawn[2].label).toBe('Unknown');
		expect(drawn[0].title).toBe('Mobile: 40 visitors, 80 page views');
	});

	it('draws nothing full when every row is zero', () => {
		expect(bars([row('Mobile', 0)], (r) => rowLabel(r.label))[0].share).toBe(0);
	});
});

describe('the visitors line', () => {
	const frame = { width: 336, height: 220, left: 36, right: 0, top: 20, bottom: 0 };
	const week: DayPoint[] = [
		{ day: '2026-09-28', visitors: 0, pageviews: 0 },
		{ day: '2026-09-29', visitors: 3, pageviews: 5 },
		{ day: '2026-09-30', visitors: 6, pageviews: 9 },
		{ day: '2026-10-01', visitors: 2, pageviews: 2 },
		{ day: '2026-10-02', visitors: 1, pageviews: 1 },
		{ day: '2026-10-03', visitors: 4, pageviews: 6 },
		{ day: '2026-10-04', visitors: 5, pageviews: 7 }
	];

	it('ends the axis on a round number whose half is whole', () => {
		expect(niceMax(0)).toBe(4);
		expect(niceMax(5)).toBe(6);
		expect(niceMax(9)).toBe(10);
		expect(niceMax(45)).toBe(60);
		expect(niceMax(120)).toBe(200);
		expect(niceMax(1000)).toBe(1000);
	});

	it('spreads the days across the plot and scales visitors to the axis', () => {
		const geometry = lineGeometry(week, frame);
		expect(geometry.points.map((point) => point.x)).toEqual([36, 86, 136, 186, 236, 286, 336]);
		// The axis ends at 6, so 6 visitors touch the top and 0 sits on the baseline.
		expect(geometry.points[2].y).toBe(20);
		expect(geometry.points[0].y).toBe(220);
		expect(geometry.points[1].y).toBe(120);
		expect(geometry.line.startsWith('M36 220 L86 120 L136 20')).toBe(true);
		expect(geometry.area.endsWith('L336 220 L36 220 Z')).toBe(true);
		expect(geometry.ticks).toEqual([
			{ value: 0, y: 220 },
			{ value: 3, y: 120 },
			{ value: 6, y: 20 }
		]);
	});

	it('names about five days, the last always among them', () => {
		expect(lineGeometry(week, frame).labels.map((label) => label.day)).toEqual([
			'2026-09-28',
			'2026-09-30',
			'2026-10-02',
			'2026-10-04'
		]);
		const quarter = Array.from({ length: 90 }, (_, index) => ({
			day: `day-${index}`,
			visitors: index,
			pageviews: index
		}));
		const named = lineGeometry(quarter, frame).labels;
		expect(named.length).toBeLessThanOrEqual(6);
		expect(named[named.length - 1].day).toBe('day-89');
	});

	it('draws one day in the middle and no days as nothing', () => {
		const one = lineGeometry([week[1]], frame);
		expect(one.points[0].x).toBe(186);
		expect(one.labels).toHaveLength(1);
		const none = lineGeometry([], frame);
		expect(none.line).toBe('');
		expect(none.area).toBe('');
		expect(none.labels).toEqual([]);
	});
});
