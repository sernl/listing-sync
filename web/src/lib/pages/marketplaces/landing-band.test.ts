// The landing hero band's data, checked against the two things it is a
// transcription of: the console catalogue it copies, and the files on disk it
// names.
//
// It sits beside `catalogue.test.ts` rather than inside it because the
// assertions are about `apps/landing/`'s copy rather than about this
// catalogue's own shape, and because that file was being edited concurrently
// for an unrelated change. The scope note asked for these as an extension of
// `catalogue.test.ts`; this is the same assertions in a file of their own.
//
// The band is a marketing page showing twenty-one third-party marks, so the
// failure these guard against is not a broken image. It is a mark shipped for
// a marketplace whose owner has not permitted it.

import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { PLATFORM_NAME, PLATFORM_ORDER } from './downloads';

const REPO = new URL('../../../../../', import.meta.url).pathname;

describe('the landing hero band', () => {
	const MARKS_DIR = `${REPO}apps/landing/public/marks`;
	const source = readFileSync(`${REPO}apps/landing/src/marketplaces.js`, 'utf8');
	const entries = [...source.matchAll(/\{\s*name:[^}]*\}/g)].map((m) => m[0]);

	it('transcribes every marketplace the console catalogues', () => {
		const catalogue = readFileSync(`${REPO}web/src/lib/pages/marketplaces/catalogue.ts`, 'utf8');
		// The catalogue's marketplaces are its three marketplace groups; the
		// browser tiles below them are not marketplaces and are not transcribed.
		const marketplaces = catalogue
			.slice(0, catalogue.indexOf('EXTENSIONS'))
			.match(/^\t\tname: '(?:[^']|\\')+'/gm);
		expect(entries.length).toBe(marketplaces?.length);
	});

	it('every declared file exists, and no file is orphaned', () => {
		const declared = entries
			.map((entry) => entry.match(/file: '([^']+)'/)?.[1])
			.filter((file): file is string => file !== undefined);
		const onDisk = readdirSync(MARKS_DIR);
		expect(declared.filter((file) => !existsSync(`${MARKS_DIR}/${file}`))).toEqual([]);
		expect(onDisk.filter((file) => !declared.includes(file))).toEqual([]);
	});

	it('every entry links to the marketplace it names', () => {
		expect(entries.filter((entry) => !/home: 'https:\/\//.test(entry))).toEqual([]);
	});

	it('draws the four marks the founder\'s mockup puts in the hero row, each with a file', () => {
		// The hero row is four marks and "and beyond."; the rest of the
		// catalogue sits behind the disclosure beside it. A featured entry with
		// no file would draw its initial at the top of the page, which is the
		// one place on the site where a mark we may not draw must not appear.
		const featured = entries
			.filter((entry) => /featured: true/.test(entry))
			.map((entry) => entry.match(/name: '([^']+)'/)?.[1]);
		expect(featured).toEqual(['TPT', 'Tes', 'Classful', 'Teach Simple']);
		expect(
			entries.filter((entry) => /featured: true/.test(entry) && !/file: '/.test(entry))
		).toEqual([]);
	});

	it('ships the console\'s own bytes for every mark it copies, never a second fetch', () => {
		// Every file here is a copy of one under `web/static/marketplaces/`,
		// whose provenance row in `docs/notes/design/marketplace-logo-sources.md`
		// records where it came from. A copy that has drifted -- refetched,
		// re-exported, optimised -- is a mark with no provenance behind it, and
		// nothing else in either tree would say so.
		const console_ = `${REPO}web/static/marketplaces`;
		const drifted = readdirSync(MARKS_DIR).filter(
			(file) =>
				!existsSync(`${console_}/${file}`) ||
				!readFileSync(`${MARKS_DIR}/${file}`).equals(readFileSync(`${console_}/${file}`))
		);
		expect(drifted).toEqual([]);
	});

	it('draws only the marketplace whose rule cannot be met truthfully as an initial', () => {
		// Etsy and Shopify joined the band on 2026-09-06, when the founder
		// reversed that half of the decision at `docs/design/decisions.md` and
		// took the same accepted risk here as on the console page. Boom Learning
		// did not, and is different in kind rather than a smaller version of the
		// same thing: its guidelines make "used with permission" mandatory
		// wherever its mark appears, so drawing it means breaching the rule or
		// printing something untrue. The mutation this closes is a later copy of
		// `boom-learning.svg` into this directory on the reasoning that every
		// other exclusion was lifted.
		const fileless = entries
			.filter((entry) => !/file: '/.test(entry))
			.map((entry) => entry.match(/name: '([^']+)'/)?.[1]);
		expect(fileless).toEqual(['Boom Learning']);
		expect(readdirSync(MARKS_DIR).filter((f) => /boom/i.test(f))).toEqual([]);
	});

	it('draws every mark in its owner\'s colours, with no filter anywhere in the sheet', () => {
		// The enforceable form of the founder's "in colour all the time", and
		// the one thing a later edit made for looks would quietly undo: the band
		// was greyscale at rest and coloured on hover, which is a state a reader
		// on a phone cannot produce at all.
		const sheet = readFileSync(`${REPO}apps/landing/src/styles/site.css`, 'utf8');
		expect(sheet).not.toContain('grayscale(');
	});
});

describe('the landing brand copies', () => {
	// The landing build and the console build separately, so the landing's
	// brand files and fonts are copies rather than links, for the reason its
	// marks are. A copy that drifted would put a second drawing of the same
	// logo, or a second cut of the same face, on the same origin.
	//
	// `served-artefacts` in `flake.nix` holds the fonts and the top-level names
	// to the same rule against the built store paths, because tam-server
	// answers both tiers from the landing copy. Nothing held `public/brand/`,
	// which is a subdirectory that check does not walk.

	const pairs: [string, string][] = [
		['apps/landing/public/brand', 'web/static/brand'],
		['apps/landing/public/fonts', 'web/static/fonts']
	];

	it.each(pairs)('%s carries the console\'s own bytes', (landing, console_) => {
		const drifted = readdirSync(`${REPO}${landing}`)
			.filter((file) => file.endsWith('.svg') || file.endsWith('.woff2'))
			.filter(
				(file) =>
					!existsSync(`${REPO}${console_}/${file}`) ||
					!readFileSync(`${REPO}${landing}/${file}`).equals(
						readFileSync(`${REPO}${console_}/${file}`)
					)
			);
		expect(drifted).toEqual([]);
	});

	it('serves the same favicon as the console', () => {
		expect(
			readFileSync(`${REPO}apps/landing/public/favicon.svg`).equals(
				readFileSync(`${REPO}web/static/favicon.svg`)
			)
		).toBe(true);
	});

	it('asks only for faces it ships', () => {
		// `served-artefacts` makes this claim against the built store path. It
		// is made here too because that check runs under `nix flake check` and
		// this one runs in the web suite, so a face renamed in the sheet and
		// not in the directory fails in the lane the edit was made in.
		const sheet = readFileSync(`${REPO}apps/landing/src/styles/site.css`, 'utf8');
		const asked = [...sheet.matchAll(/url\('\/fonts\/([^']+)'\)/g)].map((m) => m[1]);
		expect(asked.length).toBeGreaterThan(0);
		expect(
			asked.filter((file) => !existsSync(`${REPO}apps/landing/public/fonts/${file}`))
		).toEqual([]);
	});
});

describe('the landing platform row', () => {
	const VENDORS_DIR = `${REPO}apps/landing/public/vendors`;
	const source = readFileSync(`${REPO}apps/landing/src/platforms.js`, 'utf8');

	it('accounts for every platform the release manifest can carry a build for', () => {
		// Both directions. A platform the app starts publishing must appear on
		// the row or be recorded as deliberately absent, so a macOS build cannot
		// ship while the public page still implies there is none; and a platform
		// dropped from the row without a reason fails here rather than reading
		// as unsupported.
		const named = [...source.matchAll(/name: '([^']+)'/g)].map((m) => m[1]);
		expect([...named].sort()).toEqual(
			PLATFORM_ORDER.map((platform) => PLATFORM_NAME[platform]).sort()
		);
	});

	it('gives a row with no mark the reason it has none', () => {
		// A null file is a permission fact, not a missing asset, and the reason
		// is the thing a later reader needs in order not to "fix" it by fetching
		// the logo.
		const rows = [...source.matchAll(/\{[^}]*name: '[^']+'[^}]*\}/g)].map((m) => m[0]);
		for (const row of rows.filter((row) => /file: null/.test(row))) {
			expect(/why: '[^']{20,}'/.test(row), row).toBe(true);
		}
	});

	it('ships every mark it declares and declares every mark it ships', () => {
		const declared = [...source.matchAll(/file: '([^']+)'/g)].map((m) => m[1]);
		expect(declared.filter((file) => !existsSync(`${VENDORS_DIR}/${file}`))).toEqual([]);
		expect(readdirSync(VENDORS_DIR).filter((file) => !declared.includes(file))).toEqual([]);
	});

	it("copies the console's own vendor file rather than fetching a second one", () => {
		// The same rule the marks directory is held to, and it matters more
		// here: the console's copy is the one whose digest the provenance note
		// records, so a second fetch of "the Android robot" is a file no row in
		// that note describes.
		const drifted = readdirSync(VENDORS_DIR).filter(
			(file) =>
				!readFileSync(`${VENDORS_DIR}/${file}`).equals(
					readFileSync(`${REPO}web/static/vendors/${file}`)
				)
		);
		expect(drifted).toEqual([]);
	});

	it("carries Google's licence line verbatim wherever the robot is drawn", () => {
		// Google's grant for the robot is conditional on this sentence appearing
		// in the creative, and this row is a second creative drawing it. A
		// paraphrase is not the condition, and a row that drew the mark without
		// the line would be outside the licence.
		expect(source).toContain(
			'The Android robot is reproduced or modified from work created and shared by Google and used according to terms described in the Creative Commons 3.0 Attribution License.'
		);
		expect(source).toContain('Android is a trademark of Google LLC.');
		expect(source).toContain('Windows is a trademark of the Microsoft group of companies.');
	});
});
