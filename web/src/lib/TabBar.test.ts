import { describe, expect, it } from 'vitest';
import { labelFor, type Tab } from './TabBar.svelte';

const tab = (count: number | null): Tab => ({ id: 'templates', label: 'New resource', count });

describe('how a counted tab reads', () => {
	it('carries the figure when there is one', () => {
		expect(labelFor(tab(12))).toBe('New resource (12)');
	});

	it('carries a genuine zero, which is a fact and not an absence', () => {
		expect(labelFor(tab(0))).toBe('New resource (0)');
	});

	// The case this shape exists for: a read that failed leaves no count, and
	// zero would be indistinguishable from a seller who genuinely has none.
	it('drops the parenthesis entirely when the count is unknown', () => {
		expect(labelFor(tab(null))).toBe('New resource');
	});

	it('never renders a bracket with nothing usable in it', () => {
		for (const count of [null, 0, 1, 999]) {
			const read = labelFor(tab(count));
			expect(read).not.toMatch(/\(\s*\)|\(null\)|\(undefined\)|\(NaN\)|\(-\)/);
		}
	});
});
