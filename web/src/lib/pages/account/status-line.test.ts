import { describe, expect, it } from 'vitest';
import type { InventoryStatus } from '$lib/api';
import { statusLine } from './status-line';

const NOW = Date.UTC(2026, 8, 5, 12);

const OPERATING: InventoryStatus = {
	// Distinct on purpose: the line renders the marketplace, and an
	// inventory that happened to share its name would not show that.
	inventory: 'TesGb',
	marketplace: 'Tes',
	halted: false
};

describe('the line under an inventory’s name', () => {
	it('is only the marketplace while it is operating', () => {
		expect(statusLine(OPERATING, NOW)).toBe('Tes');
	});

	it('carries the recorded reason and how long it has stood', () => {
		const halted: InventoryStatus = {
			...OPERATING,
			halted: true,
			reason: 'the write endpoint answered 503 four times running',
			raised_at: NOW - 3 * 3_600_000
		};
		expect(statusLine(halted, NOW)).toBe(
			'Tes · the write endpoint answered 503 four times running, since 3 h ago'
		);
	});

	// A halt with no reason is a fact about the record, not about the
	// marketplace, so it says so rather than reading as an empty sentence.
	it('says so where the halt carries no reason', () => {
		expect(statusLine({ ...OPERATING, halted: true }, NOW)).toBe(
			'Tes · no reason recorded'
		);
	});

	// The case that actually renders: `Tpt` is both an inventory id and a
	// marketplace, and the row prints the inventory above this line, so
	// repeating it read as a rendering fault.
	it('says nothing where the inventory and the marketplace are the same word', () => {
		const same: InventoryStatus = { inventory: 'Tpt', marketplace: 'Tpt', halted: false };
		expect(statusLine(same, NOW)).toBe('');
	});

	it('still carries the reason where that word is the same and it is halted', () => {
		const same: InventoryStatus = {
			inventory: 'Tpt',
			marketplace: 'Tpt',
			halted: true,
			reason: 'paused by hand'
		};
		expect(statusLine(same, NOW)).toBe('paused by hand');
	});

	it('omits the age where the halt was never stamped', () => {
		const halted: InventoryStatus = { ...OPERATING, halted: true, reason: 'paused by hand' };
		expect(statusLine(halted, NOW)).toBe('Tes · paused by hand');
	});
});
