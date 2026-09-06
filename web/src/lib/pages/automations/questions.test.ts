import { describe, expect, it } from 'vitest';
import { questionRows, tallyLine } from './questions';
import { queued } from './fixtures.test-support';

const DAY = 86_400_000;

describe('a question row', () => {
	it('leads with the term, which is what the seller is being asked about', () => {
		const [row] = questionRows([queued({ term: 'Key Stage 3' })], 0);
		expect(row.title).toBe('Key Stage 3');
	});

	it('names the marketplace in full and says when the question was raised', () => {
		const [row] = questionRows([queued({ inventory: 'TesGb', raised_at: 0 })], DAY);
		expect(row.meta).toContain('United Kingdom');
		expect(row.meta).toContain('raised 1 day ago');
	});

	it('carries the term’s own kind, which decides what a valid answer looks like', () => {
		const [row] = questionRows([queued({ kind: 'subject' })], 0);
		expect(row.meta).toContain('subject');
	});

	it('carries the queue item, so acting on a row need not find it again', () => {
		const item = queued({ id: 'q-7' });
		const [row] = questionRows([item], 0);
		expect(row.item).toBe(item);
	});
});

describe('the queue tally', () => {
	it('states all three figures, because the open count alone cannot say whether it drained', () => {
		expect(tallyLine(2, 40, 3)).toBe('2 to answer · 40 answered · 3 left out');
	});
});
