import { describe, expect, it } from 'vitest';
import { firstSelection, heldSelection, marketplaceRows } from './marketplace-list';
import { connection } from './fixtures.test-support';

describe('the marketplace column', () => {
	it('lists one row per marketplace the seller has connected, in column order', () => {
		const rows = marketplaceRows([connection('Etsy'), connection('Tes'), connection('Tpt')]);
		expect(rows.map((row) => row.marketplace)).toEqual(['Tpt', 'Tes', 'Etsy']);
	});

	it('omits a marketplace the seller has not connected rather than showing it empty', () => {
		expect(marketplaceRows([connection('Tes')]).map((row) => row.marketplace)).toEqual(['Tes']);
	});

	it('renders the status in the words the server sent rather than re-spelling them', () => {
		const [row] = marketplaceRows([connection('Tes', { status: 'unstable' })]);
		expect(row.status).toBe('unstable');
		expect(row.tone).toBe('warn');
	});

	it('draws a linked-but-unverified connection in grey, which is nothing to act on', () => {
		const [row] = marketplaceRows([connection('Tes', { status: 'checking' })]);
		expect(row.tone).toBe('soon');
	});

	it('carries the page figure where the page has one, and null where it has none', () => {
		const rows = marketplaceRows([connection('Tes'), connection('Tpt')], { Tes: 2 });
		expect(rows.map((row) => row.count)).toEqual([null, 2]);
	});

	it('shows a zero the page counted rather than dropping it', () => {
		const [row] = marketplaceRows([connection('Tes')], { Tes: 0 });
		expect(row.count).toBe(0);
	});

	it('keeps the newest connection where a marketplace has more than one', () => {
		// The newest is given first as well as last, because with only the
		// ascending order a rule that always overwrote would pass too.
		const older = connection('Tes', { id: 'old', status: 'disconnected', updated_at: 1 });
		const newer = connection('Tes', { id: 'new', status: 'connected', updated_at: 2 });
		expect(marketplaceRows([newer, older])[0].status).toBe('connected');
		expect(marketplaceRows([older, newer])[0].status).toBe('connected');
		expect(marketplaceRows([newer, older])).toHaveLength(1);
	});
});

describe('which row the page opens on', () => {
	it('opens on the first row', () => {
		const rows = marketplaceRows([connection('Tes'), connection('Tpt')]);
		expect(firstSelection(rows)).toBe('Tpt');
	});

	it('answers nothing where the seller has connected nothing', () => {
		expect(firstSelection([])).toBeNull();
	});

	it('holds the seller’s own choice across a refetch', () => {
		const rows = marketplaceRows([connection('Tes'), connection('Tpt')]);
		expect(heldSelection(rows, 'Tes')).toBe('Tes');
	});

	it('falls back only when the marketplace it named has left the list', () => {
		const rows = marketplaceRows([connection('Tpt')]);
		expect(heldSelection(rows, 'Tes')).toBe('Tpt');
		expect(heldSelection([], 'Tes')).toBeNull();
	});
});
