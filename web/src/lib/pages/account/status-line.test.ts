import { describe, expect, it } from 'vitest';
import type { InventoryStatus } from '$lib/api';
import { statusLine } from './status-line';

const NOW = Date.UTC(2026, 8, 5, 12);

const OPERATING: InventoryStatus = {
	// Distinct on purpose: the line used to prefix the marketplace whenever it
	// differed from the inventory, so this shape is the one that would show a
	// prefix if one ever came back.
	inventory: 'TesGb',
	marketplace: 'Tes',
	halted: false
};

describe('the line under an inventory’s name', () => {
	it('says nothing while it is operating', () => {
		expect(statusLine(OPERATING, NOW)).toBe('');
	});

	it('carries the recorded reason and how long it has stood', () => {
		const halted: InventoryStatus = {
			...OPERATING,
			halted: true,
			reason: 'the write endpoint answered 503 four times running',
			raised_at: NOW - 3 * 3_600_000
		};
		expect(statusLine(halted, NOW)).toBe(
			'the write endpoint answered 503 four times running, since 3 h ago'
		);
	});

	// A halt with no reason is a fact about the record, not about the
	// marketplace, so it says so rather than reading as an empty sentence.
	it('says so where the halt carries no reason', () => {
		expect(statusLine({ ...OPERATING, halted: true }, NOW)).toBe('no reason recorded');
	});

	// The property the prefix was removed to establish, and the one a later edit
	// could quietly undo: the row's heading is `platformTitle`, which names the
	// marketplace already, so this line never does. Pinned over both shapes and
	// both states, because the prefix it replaced was conditional on exactly the
	// difference between the shapes.
	it('never names the marketplace, whatever it is called', () => {
		const sameWord: InventoryStatus = { inventory: 'Tpt', marketplace: 'Tpt', halted: false };
		const halted = { halted: true, reason: 'paused by hand' } as const;
		expect(statusLine(OPERATING, NOW)).not.toContain('Tes');
		expect(statusLine({ ...OPERATING, ...halted }, NOW)).not.toContain('Tes');
		expect(statusLine(sameWord, NOW)).not.toContain('Tpt');
		expect(statusLine({ ...sameWord, ...halted }, NOW)).not.toContain('Tpt');
	});

	it('omits the age where the halt was never stamped', () => {
		const halted: InventoryStatus = { ...OPERATING, halted: true, reason: 'paused by hand' };
		expect(statusLine(halted, NOW)).toBe('paused by hand');
	});
});
