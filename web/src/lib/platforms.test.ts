import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
	AUTHORABLE_PLATFORMS,
	CARD_ANCHOR,
	MARKETPLACE_TILES,
	MARKETPLACE_WORD,
	MARK_SRC,
	PLATFORMS,
	cardAnchor,
	marketplacesHref,
	platformTitle
} from './platforms';
import { LISTED, LIVE, PLANNED } from './pages/marketplaces/catalogue';
import { INVENTORY_IDS, MARKETPLACES } from './generated/vocab';

function staticFile(src: string): string {
	return fileURLToPath(new URL(`../../static${src}`, import.meta.url));
}

describe('how a platform is named', () => {
	it('leads with the acronym and its full name, as the founder settled it', () => {
		expect(platformTitle('Tpt')).toBe('TPT (Teachers Pay Teachers)');
	});

	it('names every inventory the vocabulary carries', () => {
		expect(Object.keys(PLATFORMS).sort()).toEqual([...INVENTORY_IDS].sort());
	});

	it('offers only the platforms an adapter exists for', () => {
		expect(AUTHORABLE_PLATFORMS).toEqual(['Tpt', 'Tes']);
	});
});

describe('the tiles the form offers', () => {
	it('draws one tile per marketplace', () => {
		expect(MARKETPLACE_TILES.map((tile) => tile.marketplace)).toEqual(['Tpt', 'Tes', 'Etsy']);
	});

	// One tile, one marketplace, one inventory: the tick is the whole answer
	// to where a listing goes, and nothing downstream has a second question.
	it('stands each tile for exactly its own inventory', () => {
		expect(MARKETPLACE_TILES.map((tile) => tile.inventory)).toEqual(['Tpt', 'Tes', 'Etsy']);
	});

	// Shown rather than hidden, so a teacher can see it is coming; the tile
	// itself is disabled because nothing can write to it.
	it('shows Etsy and says it cannot be authored', () => {
		expect(MARKETPLACE_TILES.find((tile) => tile.marketplace === 'Etsy')?.authorable).toBe(false);
	});

	it('names every tile in full, for the hover and the screen reader', () => {
		for (const tile of MARKETPLACE_TILES) {
			expect(tile.name.length, tile.marketplace).toBeGreaterThan(tile.marketplace.length);
		}
	});

	it('has a one-word name for every marketplace, for use inside a sentence', () => {
		expect(Object.keys(MARKETPLACE_WORD).sort()).toEqual([...MARKETPLACES].sort());
		for (const word of Object.values(MARKETPLACE_WORD)) {
			expect(word).not.toContain(' ');
		}
	});
});

describe('the mark each marketplace is drawn by', () => {
	it('answers for every marketplace the vocabulary carries', () => {
		expect(Object.keys(MARK_SRC).sort()).toEqual([...MARKETPLACES].sort());
	});

	it('ships the file behind each one', () => {
		for (const src of Object.values(MARK_SRC)) {
			expect(existsSync(staticFile(src)), src).toBe(true);
		}
	});

	// The drift this closes: the Marketplaces catalogue owns the same three
	// paths, and a logo moved or renamed there would leave the resource page
	// and the board's chips requesting a file that is no longer served, with
	// nothing failing until a seller looked at an empty box.
	it('names the same file the Marketplaces catalogue draws', () => {
		for (const tile of [...LIVE, ...PLANNED]) {
			if (tile.marketplace === undefined || tile.mark.kind !== 'image') {
				continue;
			}
			expect(MARK_SRC[tile.marketplace], tile.name).toBe(tile.mark.src);
		}
	});
});

describe('where a marketplace card is linked to', () => {
	it('anchors every marketplace the vocabulary carries', () => {
		expect(Object.keys(CARD_ANCHOR).sort()).toEqual([...MARKETPLACES].sort());
		expect(new Set(Object.values(CARD_ANCHOR)).size).toBe(MARKETPLACES.length);
	});

	it('sends each inventory to its own marketplace, not to the bare page', () => {
		expect(marketplacesHref('Tes')).toBe('/marketplaces#mp-tes');
		expect(marketplacesHref('Tpt')).toBe('/marketplaces#mp-tpt');
		expect(marketplacesHref('Etsy')).toBe('/marketplaces#mp-etsy');
	});

	// The card derives its own id from the name it draws, so this is what
	// binds the link to the thing linked at: a marketplace renamed on the
	// catalogue moves its anchor, and this fails rather than the link
	// silently landing at the top of the page.
	it('agrees with the id each card derives from its own name', () => {
		for (const tile of [...LIVE, ...PLANNED]) {
			if (tile.marketplace === undefined) {
				continue;
			}
			expect(cardAnchor(tile.name), tile.name).toBe(CARD_ANCHOR[tile.marketplace]);
		}
	});

	it('gives every card on the page an id of its own', () => {
		const ids = [...LIVE, ...PLANNED, ...LISTED].map((tile) => cardAnchor(tile.name));
		expect(new Set(ids).size).toBe(ids.length);
	});

	it('reduces a name with spaces and punctuation to one hyphenated id', () => {
		expect(cardAnchor('Made By Teachers')).toBe('mp-made-by-teachers');
		expect(cardAnchor('Teacha!')).toBe('mp-teacha');
	});
});
