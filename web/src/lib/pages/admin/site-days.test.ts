import { describe, expect, it } from 'vitest';
import { siteToday } from './site-days';

describe('siteToday', () => {
	it('turns at midnight in New Zealand, not at midnight UTC', () => {
		expect(siteToday(new Date('2026-09-30T10:59:59Z'))).toBe('2026-09-30');
		expect(siteToday(new Date('2026-09-30T11:00:00Z'))).toBe('2026-10-01');
		expect(siteToday(new Date('2026-06-30T12:00:00Z'))).toBe('2026-07-01');
	});
});
