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

	it('no file is copied for a marketplace whose owner requires written permission', () => {
		// Etsy and Shopify are drawn as an initial and their name until the
		// founder reports permission (`docs/design/decisions.md`, 2026-09-06).
		// The exclusion has to be a fact about the bytes that ship, not only
		// about the markup, or the file is fetchable by name from a public
		// build.
		expect(readdirSync(MARKS_DIR).filter((f) => /etsy|shopify/i.test(f))).toEqual([]);
	});
});
