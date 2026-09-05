import { describe, expect, it } from 'vitest';
import { DISABLED_REASON, WHEN_OPTIONS, timezoneLine, timezoneName } from './sharing';

describe('the sharing schedule', () => {
	it('offers no option that runs anything now, which D1 forbids the server to say', () => {
		expect([...WHEN_OPTIONS]).toEqual(['Every day', 'Every weekday', 'Every Monday', 'Off']);
		for (const option of WHEN_OPTIONS) {
			expect(option.toLowerCase()).not.toContain('now');
		}
	});

	it('gives every disabled control the same stated reason', () => {
		expect(DISABLED_REASON).toContain('not built yet');
	});
});

describe('the timezone the schedule means', () => {
	it('names a zone rather than leaving the clock ambiguous', () => {
		expect(timezoneName().length).toBeGreaterThan(0);
	});

	it('writes the zone into a sentence', () => {
		expect(timezoneLine('Pacific/Auckland')).toBe('Times are Pacific/Auckland.');
	});
});
