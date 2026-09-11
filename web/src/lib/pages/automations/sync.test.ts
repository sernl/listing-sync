import { describe, expect, it } from 'vitest';
import { openQuestionsLabel, runLog, runRows } from './sync';
import { job } from './fixtures.test-support';

const HOUR = 3_600_000;

describe('a run row', () => {
	it('leads with where the run was sent rather than with its identifier', () => {
		const [row] = runRows([job({ inventory: 'Tes' })], 0);
		expect(row.inventory).toBe('Tes');
	});

	it('keeps the identifier and the age on the meta line', () => {
		const [row] = runRows([job({ job: 'abcdef1234567890', created_at: 0 })], HOUR);
		expect(row.meta).toContain('abcdef12');
		expect(row.meta).toContain('1 h ago');
	});

	it('opens the run', () => {
		const [row] = runRows([job({ job: 'j-7' })], 0);
		expect(row.href).toBe('/sync/j-7');
	});

	it('hands the raw instant on, so the template renders it in the seller’s own locale', () => {
		const [row] = runRows([job({ created_at: 1234 })], 0);
		expect(row.at).toBe(1234);
	});
});

describe('the activity log', () => {
	it('names the marketplace short enough for one line', () => {
		const [entry] = runLog([job({ inventory: 'Tes' })], 0);
		expect(entry.what).toContain('TES');
	});
});

describe('the open-questions control', () => {
	it('carries the figure where there is one', () => {
		expect(openQuestionsLabel(3)).toBe('Open questions (3)');
		expect(openQuestionsLabel(0)).toBe('Open questions (0)');
	});

	it('drops the figure rather than claiming zero where it could not be read', () => {
		expect(openQuestionsLabel(null)).toBe('Open questions');
	});
});
