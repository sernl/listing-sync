import { existsSync, readdirSync } from 'node:fs';
import { basename } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
	DISCLAIMER,
	EXTENSIONS,
	LISTED,
	LIVE,
	PLANNED,
	type Mark,
	wordmarkSize
} from './catalogue';

const TILES = [...PLANNED, ...LISTED, ...EXTENSIONS];
const MARKS: readonly Mark[] = [...LIVE, ...TILES].map((tile) => tile.mark);

/** The smallest a wordmark may be set and still be read at a glance beside a
 *  17px card heading. Above the function's own floor, so a word that only just
 *  fits fails this rather than shipping unreadable. */
const READABLE_PX = 9;

function staticFile(src: string): string {
	return fileURLToPath(new URL(`../../../../static${src}`, import.meta.url));
}

describe('the tile catalogue', () => {
	it('keys every prospect tile uniquely, because the grid keys on the slug', () => {
		const slugs = TILES.map((tile) => tile.slug);
		expect(new Set(slugs).size).toBe(slugs.length);
	});

	it('names every marketplace once across all three marketplace groups', () => {
		const names = [...LIVE, ...PLANNED, ...LISTED].map((tile) => tile.name);
		expect(new Set(names).size).toBe(names.length);
	});

	it('ships the file behind every image mark', () => {
		for (const mark of MARKS) {
			if (mark.kind === 'image') {
				expect(existsSync(staticFile(mark.src)), mark.src).toBe(true);
			}
		}
	});

	it('publishes nothing but images beside the marks', () => {
		// `web/static` is served unauthenticated, so anything left in this
		// directory is a public document. The provenance note that used to sit
		// here reasons about brand rules and now lives in
		// `docs/notes/design/marketplace-logo-sources.md` instead.
		const held = readdirSync(fileURLToPath(new URL('../../../../static/marketplaces', import.meta.url)));
		const strays = held.filter((name) => !/\.(png|jpg|svg)$/.test(name));
		expect(strays).toEqual([]);
	});

	it('names each mark after the tile that shows it, so no logo can drift onto the wrong marketplace', () => {
		// The mutation this closes: swapping Payhip's and Sellfy's `mark.src`
		// passed every other assertion in this file — both files exist, both are
		// under `/marketplaces/`, the names stay unique and the homepages stay
		// distinct — and a logo attached to the wrong marketplace is the single
		// failure this page's trademark position can least afford.
		for (const tile of [...PLANNED, ...LISTED]) {
			if (tile.mark.kind === 'image') {
				expect(basename(tile.mark.src).replace(/\.[a-z]+$/, ''), tile.slug).toBe(tile.slug);
			}
		}
		for (const tile of LIVE) {
			if (tile.mark.kind === 'image') {
				expect(basename(tile.mark.src), tile.name).toContain(
					tile.marketplace.toLowerCase()
				);
			}
		}
	});

	it('shows every image it ships, bar the ones deliberately fetched and not shown', () => {
		// `web/static` is public, so an image nothing renders is a mark published
		// at a guessable URL for no reason. The two below are fetched evidence
		// rather than page assets and are excluded from the landing; naming them
		// here means any *other* unreferenced file fails this instead of being
		// quietly served. It stays green once landing drops them, because it
		// asserts nothing about their existence.
		const NOT_LANDED = ['boom-learning.svg', 'tpt-wordmark.svg'];
		const shown = new Set(
			MARKS.flatMap((mark) => (mark.kind === 'image' ? [basename(mark.src)] : []))
		);
		const held = readdirSync(
			fileURLToPath(new URL('../../../../static/marketplaces', import.meta.url))
		);
		const unreferenced = held.filter(
			(name) => !shown.has(name) && !NOT_LANDED.includes(name)
		);
		expect(unreferenced).toEqual([]);
	});

	it('serves every mark from our own origin, never from the marketplace', () => {
		for (const mark of MARKS) {
			if (mark.kind === 'image') {
				expect(mark.src.startsWith('/marketplaces/')).toBe(true);
			}
		}
	});

	it('gives every tile a body that is a sentence', () => {
		for (const tile of TILES) {
			expect(tile.body.endsWith('.'), tile.slug).toBe(true);
		}
	});

	it('leads with the two connected marketplaces', () => {
		expect(LIVE.map((tile) => tile.marketplace)).toEqual(['Tes', 'Tpt']);
	});

	it('holds the transport class only for a marketplace the platform knows', () => {
		// Etsy is a member of the Rust enum and so has a recorded class; Shopify
		// is not, and a badge for it would state a fact nothing records.
		expect(PLANNED.map((tile) => tile.marketplace)).toEqual(['Etsy', undefined]);
		expect(LISTED.every((tile) => tile.marketplace === undefined)).toBe(true);
		expect(EXTENSIONS.every((tile) => tile.marketplace === undefined)).toBe(true);
	});

	it('opens every marketplace tile onto that marketplace, over https', () => {
		for (const tile of [...LIVE, ...PLANNED, ...LISTED]) {
			expect(tile.home, tile.name).toBeDefined();
			expect(tile.home?.startsWith('https://'), tile.name).toBe(true);
		}
	});

	it('sends each tile somewhere different, so no logo links to the wrong shop', () => {
		const homes = [...LIVE, ...PLANNED, ...LISTED].map((tile) => tile.home);
		expect(new Set(homes).size).toBe(homes.length);
	});

	it('links the Shopify tile to the address Shopify names as the condition', () => {
		// Shopify's brand page asks that web use of its assets "include embedded
		// hyperlinks to our homepage: www.shopify.com."
		expect(PLANNED.find((tile) => tile.name === 'Shopify')?.home).toContain(
			'www.shopify.com'
		);
	});

	it('links no browser tile anywhere, because neither is a marketplace', () => {
		expect(EXTENSIONS.every((tile) => tile.home === undefined)).toBe(true);
	});

	it('claims no relationship in the disclaimer', () => {
		expect(DISCLAIMER).toContain('not affiliated with or endorsed by them');
		expect(DISCLAIMER).toContain('belong to their respective owners');
	});

	it('shows a logo for every marketplace but the one whose rule we cannot satisfy', () => {
		// Boom Learning's guidelines make a "used with permission" line mandatory
		// wherever the mark appears, and we have no permission, so printing it
		// would be a false statement rather than the guideline risk the founder
		// accepted for the rest.
		const wordmarked = [...LIVE, ...PLANNED, ...LISTED].filter(
			(tile) => tile.mark.kind === 'wordmark'
		);
		expect(wordmarked.map((tile) => tile.name)).toEqual(['Boom Learning']);
	});

	it('tiles the three marketplaces the founder asked for by name', () => {
		const slugs = LISTED.map((tile) => tile.slug);
		expect(slugs).toContain('teachbuysell');
		expect(slugs).toContain('school-ninja');
		expect(slugs).toContain('tpd');
	});

	it('tiles twenty-one marketplaces, which the browser cards are not among', () => {
		expect(LIVE.length + PLANNED.length + LISTED.length).toBe(21);
	});
});

describe('the wordmark tile', () => {
	it('sets a short name large and a long one small', () => {
		expect(wordmarkSize('Etsy')).toBeGreaterThan(wordmarkSize('Shopify'));
	});

	it('never sets a mark larger than the tile reads as a mark', () => {
		expect(wordmarkSize('X')).toBe(15);
	});

	it('never shrinks past reading, returning the floor instead', () => {
		expect(wordmarkSize('a'.repeat(60))).toBe(8);
	});

	it('sets every wordmark this page actually draws at a readable size', () => {
		for (const mark of MARKS) {
			if (mark.kind === 'wordmark') {
				expect(wordmarkSize(mark.text), mark.text).toBeGreaterThanOrEqual(READABLE_PX);
			}
		}
	});
});
