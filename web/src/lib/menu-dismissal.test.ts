import { describe, expect, it } from 'vitest';
import { closes, type MenuEvent } from './menu-dismissal';

describe('what closes an open menu', () => {
	it('closes on Escape', () => {
		expect(closes({ kind: 'escape' })).toBe(true);
	});

	it('closes on a press outside its own box', () => {
		expect(closes({ kind: 'press', inside: false })).toBe(true);
	});

	it('stays open on a press inside its own box', () => {
		expect(closes({ kind: 'press', inside: true })).toBe(false);
	});

	// The one the Labels board was missing: the caller renders the items, so
	// only the caller can say a choice happened, and a menu left standing over
	// the thing it just acted on is the bug this closes.
	it('closes when one of its items is chosen', () => {
		expect(closes({ kind: 'choice' })).toBe(true);
	});

	it('answers for every reason a menu can close', () => {
		const every: MenuEvent[] = [
			{ kind: 'escape' },
			{ kind: 'press', inside: true },
			{ kind: 'press', inside: false },
			{ kind: 'choice' }
		];
		for (const event of every) {
			expect(typeof closes(event)).toBe('boolean');
		}
	});
});
