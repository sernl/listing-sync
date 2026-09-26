// The console's floor is the phone's own System WebView, which can be years
// behind Chrome. `app.html` defines, before any module runs, every method the
// source uses and such an engine lacks. This holds the two lists together: a
// method the source starts using that the block does not define fails here,
// long before a seller's phone draws "That page could not be opened" for it.

import { readFileSync, readdirSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const SOURCE = new URL('../', import.meta.url);

/** Methods newer than Chrome 91 (Android 12's WebView as shipped on a Galaxy
 *  Note10+) and the regex that finds a use of each in the source. */
const FLOOR: Array<[name: string, use: RegExp, defined: RegExp]> = [
	['Object.hasOwn', /\bObject\.hasOwn\(/, /if \(!Object\.hasOwn\)/],
	['Array.prototype.at', /\.at\(-?\d+\)|\.at\([a-z]/, /Array\.prototype\.at = at/],
	['Array.prototype.findLast', /\.findLast\(/, /Array\.prototype\.findLast = /],
	['structuredClone', /\bstructuredClone\(/, /window\.structuredClone = /],
	// Chrome 93+ / not in 91: group and set helpers, `withResolvers`, `toSorted`.
	['Object.groupBy', /\bObject\.groupBy\(/, /Object\.groupBy = /],
	['Array.prototype.toSorted', /\.toSorted\(/, /Array\.prototype\.toSorted = /],
	['Promise.withResolvers', /Promise\.withResolvers\(/, /Promise\.withResolvers = /],
	['Set.prototype.union', /\.union\(/, /Set\.prototype\.union = /]
];

function sourceFiles(dir: URL, path: string): string[] {
	const found: string[] = [];
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		if (entry.name === 'generated' || entry.name === 'node_modules') continue;
		if (entry.isDirectory()) {
			found.push(...sourceFiles(new URL(`${entry.name}/`, dir), `${path}${entry.name}/`));
		} else if (/\.(ts|svelte)$/.test(entry.name) && !entry.name.endsWith('.test.ts')) {
			found.push(readFileSync(new URL(entry.name, dir), 'utf8'));
		}
	}
	return found;
}

describe('the WebView floor', () => {
	const shell = readFileSync(new URL('../app.html', import.meta.url), 'utf8');
	const sources = sourceFiles(SOURCE, '');

	for (const [name, use, defined] of FLOOR) {
		it(`defines ${name} in app.html when the source uses it`, () => {
			const used = sources.some((text) => use.test(text));
			if (used) {
				expect(defined.test(shell), `${name} is used but app.html does not define it`).toBe(true);
			}
		});
	}

	it('reads a real source tree, so an empty sweep cannot pass silently', () => {
		expect(sources.length).toBeGreaterThan(100);
		expect(sources.some((text) => /\bObject\.hasOwn\(/.test(text))).toBe(true);
	});
});
