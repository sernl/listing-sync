// The Template Manager renders the create form's own bands, and the one thing
// that has to differ is what they demand: the panel says "leave the rest empty
// and we will ask as usual", so a Required chip or an `aria-required` here
// would tell the seller the opposite of what the panel and the write both do.
//
// Read from the source, as `pages/resources/form-shape.test.ts` is and for the
// same reason: this is a fact about the files that the type checker cannot
// state and a pure test of the model cannot reach.

import { readFileSync, readdirSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const HERE = new URL('.', import.meta.url).pathname;
const PANELS = new URL('../resources/panels/', import.meta.url).pathname;

const TAB = readFileSync(`${HERE}ResourceTab.svelte`, 'utf8');
const panelFiles = readdirSync(PANELS).filter((name) => name.endsWith('.svelte'));

/** Every panel component the Template Manager's form renders, by tag. */
const rendered = [...TAB.matchAll(/<([A-Z]\w+Panel)\b([^>]*)>/gs)].map((match) => ({
	tag: match[1],
	attributes: match[2]
}));

describe('a template asks for everything and requires nothing', () => {
	it('renders the create form’s bands rather than a second set of controls', () => {
		expect(rendered.length).toBeGreaterThanOrEqual(6);
		for (const panel of rendered) {
			expect(panelFiles).toContain(`${panel.tag}.svelte`);
		}
	});

	it('carries no Required chip and no aria-required anywhere in the form', () => {
		// Every band takes `optional`, and every band that says Required at all
		// gates it on that prop. The two together are what makes the rendered
		// form carry none: a band passed the prop but ignoring it, or a band
		// honouring it but never passed it, fails one half or the other.
		const demanding = ['DescriptionPanel', 'PricePanel', 'CategoriesPanel', 'MarketplacePanel'];
		for (const tag of demanding) {
			const panel = rendered.find((one) => one.tag === tag);
			expect(panel, `${tag} is not rendered by the Template Manager`).toBeDefined();
			expect(panel?.attributes).toMatch(/(^|\s)optional(\s|$|=)/);
		}
		for (const file of panelFiles) {
			const source = readFileSync(`${PANELS}${file}`, 'utf8');
			// A bare `required` attribute or a literal `aria-required="true"` is
			// one the template form cannot switch off.
			expect([...source.matchAll(/\srequired(?=[\s/>])/g)].map(() => file)).toEqual([]);
			expect([...source.matchAll(/aria-required="true"/g)].map(() => file)).toEqual([]);
		}
	});

	it('leaves the grade grid’s Required to its caller rather than hard-coding it', () => {
		const grid = readFileSync(new URL('../../GradeGrid.svelte', import.meta.url).pathname, 'utf8');
		expect(grid).toMatch(/\{#if required\}<span class="req">Required<\/span>\{\/if\}/);
		expect(grid).toMatch(/required = true/);
	});
});
