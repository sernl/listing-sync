import { describe, expect, it } from 'vitest';
import { agoLabel } from './elapsed';

const HOUR = 3_600_000;
const DAY = 24 * HOUR;
const now = 1_756_000_000_000;

describe('the age of an instant', () => {
	it('is written in the largest unit that fits', () => {
		expect(agoLabel(now - 30_000, now)).toBe('just now');
		expect(agoLabel(now - 5 * 60_000, now)).toBe('5 min ago');
		expect(agoLabel(now - 3 * HOUR, now)).toBe('3 h ago');
		expect(agoLabel(now - 2 * DAY, now)).toBe('2 days ago');
	});

	it('says one day rather than one days', () => {
		expect(agoLabel(now - DAY, now)).toBe('1 day ago');
	});

	it('rounds down, so nothing reads fresher than it is', () => {
		expect(agoLabel(now - (2 * HOUR - 1), now)).toBe('1 h ago');
		expect(agoLabel(now - (DAY - 1), now)).toBe('23 h ago');
	});

	it('reads a clock ahead of us as just now, not as a negative age', () => {
		expect(agoLabel(1_000_000, 0)).toBe('just now');
	});
});
