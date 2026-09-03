import { describe, expect, it } from 'vitest';
import {
	METRIC_COLUMNS,
	capturedAgo,
	formatMetric,
	titlesByMapping
} from './analytics-view';
import type { MappingHead, ProductHead } from '$lib/api';

const HOUR = 3_600_000;
const DAY = 24 * HOUR;

function product(id: string, title: string): ProductHead {
	return { id, title, price: null, created_at: 0, updated_at: 0 };
}

function mapping(id: string, productId: string): MappingHead {
	return {
		id,
		product: productId,
		inventory: 'Tpt',
		binding_state: 'bound',
		lifecycle_state: 'live',
		updated_at: 0,
		listing_url: null
	};
}

describe('the metric columns', () => {
	it('names the three metrics a capture stores, by their stored keys', () => {
		expect(METRIC_COLUMNS.map((column) => column.key)).toEqual([
			'sales_count',
			'earnings',
			'resource_views'
		]);
	});
});

describe('a captured figure', () => {
	it('renders plainly, with no currency on the untyped earnings number', () => {
		expect(formatMetric(42.5)).toBe('42.5');
		expect(formatMetric(1234)).toBe('1,234');
	});

	it('rounds rather than spelling out a float', () => {
		expect(formatMetric(1234.567)).toBe('1,234.57');
	});

	it('shows an absent metric as absent instead of as a zero', () => {
		expect(formatMetric(undefined)).toBe('—');
		expect(formatMetric(0)).toBe('0');
	});
});

describe('the staleness wording', () => {
	it('counts in minutes, hours and days as the age grows', () => {
		const now = 10 * DAY;
		expect(capturedAgo(now - 30_000, now)).toBe('captured just now');
		expect(capturedAgo(now - 5 * 60_000, now)).toBe('captured 5 min ago');
		expect(capturedAgo(now - 3 * HOUR, now)).toBe('captured 3 h ago');
		expect(capturedAgo(now - 2 * DAY, now)).toBe('captured 2 days ago');
	});

	it('says one day rather than one days', () => {
		const now = 10 * DAY;
		expect(capturedAgo(now - DAY, now)).toBe('captured 1 day ago');
	});

	it('rounds down, so a figure is never called fresher than it is', () => {
		const now = 10 * DAY;
		expect(capturedAgo(now - (2 * HOUR - 1), now)).toBe('captured 1 h ago');
		expect(capturedAgo(now - (DAY - 1), now)).toBe('captured 23 h ago');
	});

	it('reads a clock ahead of us as just captured, not as a negative age', () => {
		expect(capturedAgo(1_000_000, 0)).toBe('captured just now');
	});
});

describe('the listing title join', () => {
	it('names a mapping by the title of the product it lists', () => {
		const titles = titlesByMapping(
			[product('p1', 'Fractions worksheet')],
			[mapping('m1', 'p1')]
		);
		expect(titles.get('m1')).toBe('Fractions worksheet');
	});

	it('leaves out a mapping whose product is not in the catalogue', () => {
		const titles = titlesByMapping([product('p1', 'Fractions worksheet')], [mapping('m9', 'p9')]);
		expect(titles.has('m9')).toBe(false);
	});

	it('titles every mapping of a product that carries more than one', () => {
		const titles = titlesByMapping(
			[product('p1', 'Fractions worksheet')],
			[mapping('m1', 'p1'), mapping('m2', 'p1')]
		);
		expect([titles.get('m1'), titles.get('m2')]).toEqual([
			'Fractions worksheet',
			'Fractions worksheet'
		]);
	});

	it('joins nothing when neither read has arrived', () => {
		expect(titlesByMapping([], []).size).toBe(0);
	});
});
