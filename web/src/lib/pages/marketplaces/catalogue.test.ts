import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { basename } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
	DISCLAIMER,
	EXTENSIONS,
	LISTED,
	LIVE,
	MARK_ATTRIBUTION,
	PLANNED,
	type Mark,
	wordmarkSize
} from './catalogue';
import { PLATFORM_MARK, PLATFORM_NAME } from './downloads';

const TILES = [...PLANNED, ...LISTED, ...EXTENSIONS];
const MARKS: readonly Mark[] = [...LIVE, ...TILES].map((tile) => tile.mark);

/** Every mark the page draws, the three download tiles included, because the
 *  two served directories below are shared with them and an assertion over one
 *  list would leave the other free to publish an unreferenced file. */
const ALL_MARKS: readonly Mark[] = [...MARKS, ...Object.values(PLATFORM_MARK)];

/** The two directories under `web/static` that hold a mark, and what may sit in
 *  each. They are two rather than one because a marketplace and a browser are
 *  not the same kind of owner, and because the guard that a marketplace mark is
 *  named for its marketplace is worth keeping exactly as strong as it is. */
const MARK_DIRS = ['marketplaces/', 'vendors/'] as const;

function heldIn(dir: string): string[] {
	return readdirSync(fileURLToPath(new URL(`../../../../static/${dir}`, import.meta.url)));
}

/** The twenty-one tiles that are marketplaces, which is every group but the
 *  browsers. The description guards below run over exactly this set, because
 *  the founder asked for a description of each marketplace and a browser is not
 *  one. */
const MARKETPLACES = [...LIVE, ...PLANNED, ...LISTED];

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
		for (const mark of ALL_MARKS) {
			if (mark.kind === 'image') {
				expect(existsSync(staticFile(mark.src)), mark.src).toBe(true);
			}
		}
	});

	it('publishes nothing but images beside the marks, in either mark directory', () => {
		// `web/static` is served unauthenticated, so anything left in these
		// directories is a public document. The provenance note that used to sit
		// here reasons about brand rules and now lives in
		// `docs/notes/design/marketplace-logo-sources.md` instead, and the
		// vendors' own licence texts must not follow the files they govern into a
		// public directory either.
		for (const dir of MARK_DIRS) {
			const strays = heldIn(dir).filter((name) => !/\.(png|jpg|svg)$/.test(name));
			expect(strays, dir).toEqual([]);
		}
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
		// The same rule for the browser tiles, which now hold a vendor's own file
		// rather than two words: Mozilla's lockup on the Chrome tile would be a
		// vendor logo attached to the wrong vendor, which is the one failure a
		// page carrying five owners' marks can least afford.
		for (const tile of EXTENSIONS) {
			if (tile.mark.kind === 'image') {
				expect(basename(tile.mark.src).replace(/\.[a-z]+$/, ''), tile.slug).toBe(tile.slug);
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
			ALL_MARKS.flatMap((mark) => (mark.kind === 'image' ? [basename(mark.src)] : []))
		);
		for (const dir of MARK_DIRS) {
			const unreferenced = heldIn(dir).filter(
				(name) => !shown.has(name) && !NOT_LANDED.includes(name)
			);
			expect(unreferenced, dir).toEqual([]);
		}
	});

	it('serves every mark from our own origin, out of the directory its owner belongs in', () => {
		// Two served directories rather than one, so the rule that a mark under
		// `/marketplaces/` is named for a marketplace stays as strong as it was.
		// Keyed by the list the tile came from rather than by the file name: a
		// marketplace logo moved into `vendors/` would escape that rule silently,
		// and this is what refuses it.
		for (const tile of [...LIVE, ...PLANNED, ...LISTED]) {
			if (tile.mark.kind === 'image') {
				expect(tile.mark.src.startsWith('/marketplaces/'), tile.name).toBe(true);
			}
		}
		for (const tile of EXTENSIONS) {
			if (tile.mark.kind === 'image') {
				expect(tile.mark.src.startsWith('/vendors/'), tile.name).toBe(true);
			}
		}
		for (const [platform, mark] of Object.entries(PLATFORM_MARK)) {
			if (mark.kind === 'image') {
				expect(mark.src.startsWith('/vendors/'), platform).toBe(true);
			}
		}
	});

	it('gives every tile a body that is a sentence', () => {
		for (const tile of TILES) {
			expect(tile.body.endsWith('.'), tile.slug).toBe(true);
		}
	});

	it('says what every marketplace is, in a sentence a seller can read', () => {
		for (const tile of MARKETPLACES) {
			expect(tile.about.trim(), tile.name).toBe(tile.about);
			expect(tile.about.length, tile.name).toBeGreaterThan(20);
			expect(tile.about.endsWith('.'), tile.name).toBe(true);
		}
	});

	it('keeps every description to one sentence, or two at the most', () => {
		// The page is already over seven thousand pixels tall at phone width, and
		// twenty-one descriptions are what this change adds to it. One sentence is
		// the default and two the exception; a third is a paragraph, and a
		// paragraph belongs somewhere other than a tile.
		for (const tile of MARKETPLACES) {
			const sentences = tile.about.match(/[.!?](\s|$)/g) ?? [];
			expect(sentences.length, tile.about).toBeLessThanOrEqual(2);
		}
	});

	it('describes each marketplace differently, so no card is another one copied', () => {
		// The mutation this closes: fifteen tiles already carry the identical
		// transport sentence in `body`, so a description pasted from the tile
		// above passes every other assertion here — it is a non-empty sentence
		// that ends in a full stop and sits on a tile that has one.
		const said = MARKETPLACES.map((tile) => tile.about);
		expect(new Set(said).size).toBe(said.length);
	});

	it('describes twenty-one marketplaces and neither browser', () => {
		// By group rather than by count, because the point is which tiles carry a
		// description: `EXTENSIONS` is typed `ProspectTile`, which has no `about`
		// at all, so a description added to a browser tile fails the type check
		// rather than this.
		expect(MARKETPLACES.length).toBe(21);
		for (const tile of EXTENSIONS) {
			expect(Object.hasOwn(tile, 'about'), tile.slug).toBe(false);
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

	it('links the tile drawing a vendor logo to that vendor, and links the other nowhere', () => {
		// Not a preference. Mozilla's policy permits its logo in a visual that
		// truthfully refers to or links to the program, so the link is the
		// condition the mark is shown under; the Chrome tile draws no vendor mark
		// and so carries no such condition. Dropping the Firefox link would leave
		// the logo standing with nothing discharging the permission it relies on.
		const firefox = EXTENSIONS.find((tile) => tile.slug === 'firefox');
		expect(firefox?.mark.kind).toBe('image');
		expect(firefox?.home).toBe('https://www.mozilla.org/firefox/');
		expect(EXTENSIONS.find((tile) => tile.slug === 'chrome')?.home).toBeUndefined();
	});

	it('claims no relationship in the disclaimer, and names every kind of owner under it', () => {
		expect(DISCLAIMER).toContain('not affiliated with or endorsed by them');
		expect(DISCLAIMER).toContain('belong to their respective owners');
		// The sentence became untrue the moment a browser's and a platform's mark
		// appeared under it: it spoke only of marketplaces while three of the
		// owners whose marks the page draws are not marketplaces.
		expect(DISCLAIMER).toContain('marketplace, browser and platform names and logos');
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

	it('draws a browser vendor logo only where the vendor permits one, and names the other', () => {
		// Mozilla permits its logo in writing without prior permission; Google
		// routes every Chrome product icon through a Partner Marketing Hub
		// approval nobody here holds. So the kinds differ, and asserting them is
		// what closes the direction that matters: a Chrome image lands only once
		// somebody has read an approval, and it fails here until they have.
		expect(EXTENSIONS.map((tile) => [tile.slug, tile.mark.kind])).toEqual([
			['chrome', 'wordmark'],
			['firefox', 'image']
		]);
	});

	it('opens the Firefox tile onto Mozilla on the page, not only in the catalogue', () => {
		// The defect this closes, found by reading the page rather than the
		// data: `EXTENSIONS` gained a `home` and the card that draws these two
		// tiles was still called without it, so the catalogue satisfied Mozilla's
		// link condition and the rendered page did not. Every assertion about
		// `home` above passed throughout.
		const page = readFileSync(
			fileURLToPath(new URL('./MarketplacesPage.svelte', import.meta.url)),
			'utf8'
		);
		const block = page.slice(page.indexOf('{#each EXTENSIONS'), page.indexOf('{/each}', page.indexOf('{#each EXTENSIONS')));
		expect(block).toContain('home={tile.home}');
	});

	it("writes Chrome's name the way Google requires its trademark to be written", () => {
		// "make reference to that Google product by using the text 'for', 'for use
		// with', or 'compatible with', and be sure to include the ™ symbol with the
		// Google trademark. Example: 'for Google Chrome™'", at
		// https://developer.chrome.com/docs/webstore/branding, under "Describing
		// your extension". Both halves, because the change took only the symbol
		// first time round: the heading is the reference that sentence governs and
		// so carries Google's full product name, and the mark box is artwork
		// standing in for a logo we may not draw. Dropping either is a small edit
		// that reads as a tidy-up and breaches the one condition attached to the
		// only Chrome asset we are using, the name.
		const chrome = EXTENSIONS.find((tile) => tile.slug === 'chrome');
		expect(chrome?.name).toBe('Google Chrome™');
		const mark = chrome?.mark;
		expect(mark?.kind).toBe('wordmark');
		expect(mark !== undefined && mark.kind === 'wordmark' ? mark.text : null).toBe('Chrome™');
	});

	it("heads the Firefox tile the way Mozilla's policy asks its wordmark to be used", () => {
		// "Use Mozilla wordmarks only as an adjective, never as a noun or verb...
		// Instead, use the generic term for the Mozilla product or service
		// following the trademark. For example: Firefox web browser", at
		// https://www.mozilla.org/en-US/foundation/trademarks/policy/. The tile
		// draws Mozilla's horizontal lockup, which already contains the word, so a
		// bare "Firefox" heading both repeated the wordmark and used it as the noun
		// the policy names. The generic term is what this asserts, not the exact
		// phrase: "Firefox browser" and "Firefox web browser" both satisfy it.
		const firefox = EXTENSIONS.find((tile) => tile.slug === 'firefox');
		expect(firefox?.name.startsWith('Firefox ')).toBe(true);
		expect(firefox?.name.endsWith('browser')).toBe(true);
	});
});

/** What Google and Mozilla publish, character for character, read on 2026-09-06
 *  at the addresses `docs/notes/design/marketplace-logo-sources.md` records.
 *  Held as literals here rather than imported from the module under test, so a
 *  paraphrase typed into `catalogue.ts` fails rather than travelling with it. */
const GOOGLE_CC_LINE =
	'The Android robot is reproduced or modified from work created and shared by Google and used ' +
	'according to terms described in the Creative Commons 3.0 Attribution License.';
const GOOGLE_ANDROID_TM = 'Android is a trademark of Google LLC.';

describe('the attribution under the grid', () => {
	it("carries Google's licence line exactly as Google publishes it", () => {
		// A paraphrase of a licence condition is not the condition, and this one is
		// the whole grant the robot is drawn under: reworded for house style, the
		// page keeps the mark and loses the permission, and nothing else on the
		// page would say so.
		expect(MARK_ATTRIBUTION).toContain(GOOGLE_CC_LINE);
		expect(MARK_ATTRIBUTION).toContain(GOOGLE_ANDROID_TM);
	});

	it('prints a sentence exactly for the marks that are actually drawn', () => {
		// Both directions for all five, so the block follows the page rather than a
		// memory of it: removing the robot must remove its two lines, dropping the
		// Windows download card must drop the Windows line, and adding a vendor's
		// mark or name without its attribution fails here. Two rows were hard-coded
		// `true` before, which is the memory this is meant not to be: the page
		// could lose every Android reference or the whole Windows card and still be
		// required to carry their sentences, with nothing saying why.
		//
		// A name obliges as surely as a logo does. Google requires its trademark
		// line wherever the Android name appears and its Chrome line wherever
		// Chrome is named, and Microsoft's wordmark permission is what the Windows
		// card's name rests on, so the three name rows key on the names this page
		// actually prints -- the tiles' and the download cards' together, since
		// `MarketplacesPage.svelte` renders both under one attribution block.
		const drawn = ALL_MARKS.flatMap((mark) => (mark.kind === 'image' ? [mark.src] : []));
		const named = [...LIVE, ...TILES].map((tile) => tile.name).concat(Object.values(PLATFORM_NAME));
		const names = (product: string) => named.some((name) => name.includes(product));
		const owed = new Map<string, boolean>([
			[GOOGLE_CC_LINE, drawn.includes('/vendors/android-robot.svg')],
			[GOOGLE_ANDROID_TM, names('Android') || drawn.includes('/vendors/android-robot.svg')],
			[
				'Firefox is a trademark of the Mozilla Foundation in the US and other countries.',
				drawn.includes('/vendors/firefox.svg') || names('Firefox')
			],
			['Google Chrome is a trademark of Google LLC.', names('Chrome')],
			['Windows is a trademark of the Microsoft group of companies.', names('Windows')]
		]);
		for (const [sentence, required] of owed) {
			expect(MARK_ATTRIBUTION.includes(sentence), sentence).toBe(required);
		}
		expect(MARK_ATTRIBUTION.length).toBe([...owed.values()].filter(Boolean).length);
	});

	it('publishes no provenance on the page, only the sentences the licences ask for', () => {
		// The addresses, the retrieval dates and the digests are in a note under
		// `docs/notes/`, deliberately, because a document reasoning about brand
		// risk is not something to serve at a URL. The mutation this closes is a
		// well-meant one: pasting a source URL beside its sentence so a reader can
		// check it, which publishes the evidence trail on the marketing-facing
		// half of a logged-in page.
		for (const sentence of [DISCLAIMER, ...MARK_ATTRIBUTION]) {
			expect(/https?:\/\//.test(sentence), sentence).toBe(false);
			expect(/\d{4}-\d{2}-\d{2}/.test(sentence), sentence).toBe(false);
			expect(/\b[0-9a-f]{64}\b/.test(sentence), sentence).toBe(false);
		}
	});
});

describe('the wordmark tile', () => {
	it('sets a short name large and a long one small', () => {
		// Past the ceiling rather than either side of it: the tile is wide enough
		// that every word of seven characters or fewer now clamps to the same
		// size, so a pair that short would compare two ceilings and pass whatever
		// the taper did.
		expect(wordmarkSize('Etsy')).toBeGreaterThan(wordmarkSize('Lemon Squeezy'));
	});

	it('never sets a mark larger than the tile reads as a mark', () => {
		expect(wordmarkSize('X')).toBe(22);
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
