// The rename map's other half. `nav.test.ts` proves each old path redirects;
// this proves nothing still links to one, which a compiler cannot catch in a
// Svelte href string.

import { readFileSync, readdirSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { LEGACY_REDIRECTS } from './nav';

const SOURCE = new URL('../', import.meta.url);

/** The stub routes are the one place an old path is still written down: each
 *  is the redirect that keeps a bookmark working. */
const STUBS: readonly string[] = LEGACY_REDIRECTS.map(({ from }) => `routes${from}/`);

/** The table itself, and the two tests that assert over it. */
const EXEMPT: readonly string[] = ['lib/nav.ts', 'lib/nav.test.ts', 'lib/routes.test.ts'];

function sourceFiles(dir: URL, prefix: string): { path: string; text: string }[] {
	const found: { path: string; text: string }[] = [];
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		if (entry.name === 'generated' || entry.name === 'node_modules') {
			continue;
		}
		const path = `${prefix}${entry.name}`;
		if (entry.isDirectory()) {
			found.push(...sourceFiles(new URL(`${entry.name}/`, dir), `${path}/`));
		} else if (/\.(ts|svelte|css)$/.test(entry.name)) {
			found.push({ path, text: readFileSync(new URL(entry.name, dir), 'utf8') });
		}
	}
	return found;
}

describe('the paths the rename map retired', () => {
	const files = sourceFiles(SOURCE, '');

	it('reads a source tree at all, so an empty sweep cannot pass silently', () => {
		expect(files.length).toBeGreaterThan(50);
	});

	for (const { from } of LEGACY_REDIRECTS) {
		it(`is written nowhere but the stub that redirects ${from}`, () => {
			const offenders = files
				.filter((file) => !STUBS.some((stub) => file.path.startsWith(stub)))
				.filter((file) => !EXEMPT.includes(file.path))
				.filter((file) => new RegExp(`['"\`]${from}(['"\`/?]|$)`, 'm').test(file.text))
				.map((file) => file.path);
			expect(offenders).toEqual([]);
		});
	}
});
