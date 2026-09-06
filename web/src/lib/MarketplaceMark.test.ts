// Where the console draws a marketplace by its mark rather than by its name,
// read from the source rather than from a render.
//
// Vitest runs in `node`, so there is no component to mount. What can be held
// here is the founder's 2026-09-07 instruction as a list: every label and tab
// that used to print a marketplace's name now draws `MarketplaceMark`, and a
// bare name survives in a label position only where a control's own label
// needs the words. The sweep below is what stops a later edit quietly typing
// a name back out beside the marks.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const SRC = fileURLToPath(new URL('..', import.meta.url));

function svelteFiles(dir: string): string[] {
	const found: string[] = [];
	for (const entry of readdirSync(dir)) {
		if (entry === 'node_modules') {
			continue;
		}
		const path = join(dir, entry);
		if (statSync(path).isDirectory()) {
			found.push(...svelteFiles(path));
		} else if (entry.endsWith('.svelte')) {
			found.push(path);
		}
	}
	return found;
}

function read(path: string): string {
	return readFileSync(join(SRC, path), 'utf8');
}

/** The sites converted on 2026-09-07, each with the bare name it printed
 *  before, so the conversion cannot be undone one site at a time. */
const CONVERTED: Record<string, readonly string[]> = {
	'routes/sync/+page.svelte': ['{run.title}'],
	'routes/automations/migration/+page.svelte': ['{row.title}'],
	'lib/pages/automations/MarketplaceList.svelte': ['{row.name}'],
	'lib/pages/import/ImportPage.svelte': ['<h2>{card.name}</h2>', '{row.title}'],
	'lib/pages/analytics/Analytics.svelte': ['{row.platform}'],
	'lib/TabBar.svelte': [],
	'lib/pages/marketplaces/Machines.svelte': ['MARKETPLACE_NAME'],
	'routes/status/+page.svelte': ['platformTitle('],
	'lib/pages/templates/MappingTab.svelte': ['<h3>{platformTitle(group.inventory)}</h3>'],
	'lib/pages/resources/ResourceDetail.svelte': ['{platformTitle(run.inventory)}']
};

/** Where a marketplace is still written as a name in a label position: each
 *  is the label of a checkbox that publishes to or deletes from that
 *  marketplace, and a choice of that weight is made against words. Selects
 *  keep the name too, and are not counted here because an `option` is never
 *  a label position. The new-resource picker is a tile drawn by its mark with
 *  the name as its visually hidden label, so it is no longer counted. */
const KEPT: Record<string, number> = {
	'lib/CrossListDialog.svelte': 1,
	'lib/DeleteDialog.svelte': 1
};

/** An element whose whole content is one marketplace name. */
const BARE =
	/<(span|div|h[1-6]|td|th|b|i|p|strong|li|label)\b[^>]*>\s*\{(platformTitle\([^}]*\)|SHORT_NAME\[[^\]]*\]|MARKETPLACE_NAME\[[^\]]*\])\}\s*<\/\1>/g;

describe('where a marketplace is drawn by its mark', () => {
	it('draws the mark at every converted site and no longer prints the name there', () => {
		for (const [site, before] of Object.entries(CONVERTED)) {
			const source = read(site);
			expect(source, site).toContain("import MarketplaceMark from '$lib/MarketplaceMark.svelte';");
			expect(source, site).toContain('<MarketplaceMark ');
			for (const bare of before) {
				expect(source, `${site} still prints ${bare}`).not.toContain(bare);
			}
		}
	});

	it('prints a bare name in a label position only where a control is labelled by it', () => {
		const offenders: Record<string, number> = {};
		for (const path of svelteFiles(SRC)) {
			const count = [...readFileSync(path, 'utf8').matchAll(BARE)].length;
			if (count > 0) {
				offenders[relative(SRC, path).split(sep).join('/')] = count;
			}
		}
		expect(offenders).toEqual(KEPT);
	});

	it('names the mark once, in words, and the image not at all', () => {
		const source = read('lib/MarketplaceMark.svelte');
		expect(source).toContain('role="img"');
		expect(source).toContain('aria-label={shown.name}');
		expect(source).toContain('title={shown.name}');
		for (const image of source.matchAll(/<img\b[^>]*>/g)) {
			expect(image[0]).toMatch(/alt=""/);
		}
	});

	it('keeps the words as a marked tab’s accessible name', () => {
		const source = read('lib/TabBar.svelte');
		expect(source).toContain('aria-label={tab.mark === undefined ? undefined : labelFor(tab)}');
	});
});
