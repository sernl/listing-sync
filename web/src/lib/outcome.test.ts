import { describe, expect, it } from 'vitest';
import { segments } from './outcome';
import type { Counts } from './api';

function counts(partial: Partial<Counts>): Counts {
	return {
		total: 0,
		queued: 0,
		in_flight: 0,
		blocked: 0,
		parked: 0,
		settled: 0,
		succeeded: 0,
		degraded: 0,
		failed: 0,
		ambiguous: 0,
		skipped: 0,
		outcome_blocked: 0,
		...partial
	};
}

describe('the outcome bar', () => {
	it('shows every present segment and no scalar verdict', () => {
		const bar = segments(counts({ total: 10, succeeded: 4, failed: 1, queued: 5 }));
		expect(bar.map((segment) => segment.label)).toEqual(['succeeded', 'failed', 'queued']);
		expect(bar.map((segment) => segment.count)).toEqual([4, 1, 5]);
	});

	it('shares sum to at most one and scale by the total', () => {
		const bar = segments(counts({ total: 4, succeeded: 1, failed: 1, queued: 2 }));
		const sum = bar.reduce((acc, segment) => acc + segment.share, 0);
		expect(sum).toBeCloseTo(1);
	});

	it('an empty job renders no segments rather than dividing by zero', () => {
		expect(segments(counts({}))).toEqual([]);
	});
});
