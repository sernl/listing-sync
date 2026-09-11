import { describe, expect, it } from 'vitest';
import { inventoryMark, marketplaceMark } from './marketplace-mark';
import { MARK_SRC, platformTitle } from './platforms';
import { INVENTORY_IDS, MARKETPLACES } from './generated/vocab';
import type { InventoryId, Marketplace } from './generated/vocab';

describe('the mark an inventory is drawn by', () => {
	it('names the platform in full beside its own mark', () => {
		expect(inventoryMark('Tes')).toEqual({
			src: '/marketplaces/tes-mark.svg',
			name: 'TES (Tes.com)'
		});
		expect(inventoryMark('Tpt')).toEqual({
			src: '/marketplaces/tpt-mark.svg',
			name: 'TPT (Teachers Pay Teachers)'
		});
	});

	it('answers for every inventory the vocabulary carries', () => {
		for (const inventory of INVENTORY_IDS) {
			const mark = inventoryMark(inventory);
			expect(mark.src, inventory).not.toBeNull();
			expect(mark.name, inventory).toBe(platformTitle(inventory));
		}
	});

	// A row's inventory is read straight off the wire, and a server ahead of
	// this bundle can name one the union does not hold; the site still has to
	// draw something with a name rather than an empty box.
	it('falls back to the identifier in words for an inventory this bundle does not know', () => {
		expect(inventoryMark('TesAu' as InventoryId)).toEqual({ src: null, name: 'TesAu' });
	});
});

describe('the mark a marketplace is drawn by', () => {
	it('names the marketplace in full beside its own mark', () => {
		expect(marketplaceMark('Tes')).toEqual({ src: MARK_SRC.Tes, name: 'TES (Tes.com)' });
	});

	it('answers for every marketplace the vocabulary carries', () => {
		for (const marketplace of MARKETPLACES) {
			expect(marketplaceMark(marketplace).src, marketplace).toBe(MARK_SRC[marketplace]);
		}
	});

	it('falls back to the identifier for a marketplace this bundle does not know', () => {
		expect(marketplaceMark('Shopify' as Marketplace)).toEqual({
			src: null,
			name: 'Shopify'
		});
	});
});
