import { describe, expect, it } from 'vitest';
import { inventoryMark, marketplaceMark } from './marketplace-mark';
import { MARK_SRC, platformTitle } from './platforms';
import { INVENTORY_IDS, MARKETPLACES } from './generated/vocab';
import type { InventoryId, Marketplace } from './generated/vocab';

describe('the mark an inventory is drawn by', () => {
	it('names the platform in full and tags the region only where three sites share a mark', () => {
		expect(inventoryMark('TesGb')).toEqual({
			src: '/marketplaces/tes-mark.svg',
			region: 'GB',
			name: 'TES (Tes.com) · United Kingdom'
		});
		expect(inventoryMark('Tpt')).toEqual({
			src: '/marketplaces/tpt-mark.svg',
			region: null,
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
		expect(inventoryMark('TesAu' as InventoryId)).toEqual({
			src: null,
			region: null,
			name: 'TesAu'
		});
	});
});

describe('the mark a marketplace is drawn by', () => {
	it('carries no region, because one marketplace is one mark', () => {
		expect(marketplaceMark('Tes')).toEqual({
			src: MARK_SRC.Tes,
			region: null,
			name: 'TES (Tes.com)'
		});
	});

	it('answers for every marketplace the vocabulary carries', () => {
		for (const marketplace of MARKETPLACES) {
			expect(marketplaceMark(marketplace).src, marketplace).toBe(MARK_SRC[marketplace]);
		}
	});

	it('falls back to the identifier for a marketplace this bundle does not know', () => {
		expect(marketplaceMark('Shopify' as Marketplace)).toEqual({
			src: null,
			region: null,
			name: 'Shopify'
		});
	});
});
