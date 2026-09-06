import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
	AUTHORABLE_PLATFORMS,
	CARD_ANCHOR,
	MARK_SRC,
	PLATFORMS,
	REGION_TAG,
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

	it('separates the three Tes sites, which are one marketplace under three inventories', () => {
		const titles = (['TesGb', 'TesUs', 'TesNz'] as const).map(platformTitle);
		expect(new Set(titles).size).toBe(3);
		for (const title of titles) {
			expect(title.startsWith('TES (Tes.com)')).toBe(true);
		}
	});

	it('offers only the platforms an adapter exists for', () => {
		expect(AUTHORABLE_PLATFORMS).toEqual(['Tpt', 'TesGb', 'TesUs', 'TesNz']);
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

describe('the region tag beside a mark', () => {
	it('answers for every inventory the vocabulary carries', () => {
		expect(Object.keys(REGION_TAG).sort()).toEqual([...INVENTORY_IDS].sort());
	});

	// A tag exists to tell one inventory of a marketplace from another, which
	// is exactly where `PLATFORMS` carries a region. Tying the two together
	// stops a fourth Tes site arriving with a mark and no way to tell it apart.
	it('carries a tag exactly where the platform name carries a region', () => {
		for (const inventory of INVENTORY_IDS) {
			expect(REGION_TAG[inventory] === null, inventory).toBe(
				PLATFORMS[inventory].region === null
			);
		}
	});
});

describe('where a marketplace card is linked to', () => {
	it('anchors every marketplace the vocabulary carries', () => {
		expect(Object.keys(CARD_ANCHOR).sort()).toEqual([...MARKETPLACES].sort());
		expect(new Set(Object.values(CARD_ANCHOR)).size).toBe(MARKETPLACES.length);
	});

	it('sends each inventory to its own marketplace, not to the bare page', () => {
		expect(marketplacesHref('TesGb')).toBe('/marketplaces#mp-tes');
		expect(marketplacesHref('TesUs')).toBe('/marketplaces#mp-tes');
		expect(marketplacesHref('TesNz')).toBe('/marketplaces#mp-tes');
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
