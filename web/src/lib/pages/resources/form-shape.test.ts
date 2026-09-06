// The shape of the resource form's surface, read from the source rather than
// from a render.
//
// A restyle is invisible to the type checker: `svelte-check` passes on a form
// that has silently regained the nine cards it was taken out of, on a `res-`
// class whose rule nobody wrote, and on a control whose label says Required to
// an eye and nothing to a screen reader. These are the facts about the files
// that the render proofs cannot state and the type checker will not.

import { readFileSync, readdirSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { ICON_NAMES } from '$lib/icons';

const HERE = new URL('.', import.meta.url).pathname;
const LIB = new URL('../../', import.meta.url).pathname;

const css = readFileSync(`${HERE}resources.css`, 'utf8');
const read = (file: string) => readFileSync(`${HERE}${file}`, 'utf8');

const FORM_SECTION = readFileSync(`${LIB}FormSection.svelte`, 'utf8');
const RESOURCE_FORM = read('ResourceForm.svelte');
const MARKETPLACE_PICKER = read('MarketplacePicker.svelte');

/** Every class beginning `res-` a file writes, whether in a literal `class`
 *  attribute, inside a string in an expression one, or as `class:name`. */
function classesIn(source: string): Set<string> {
	const found = new Set<string>();
	for (const match of source.matchAll(/class=(?:"([^"]*)"|\{([^}]*)\})/g)) {
		const written = match[1] ?? match[2] ?? '';
		const literals = [...written.matchAll(/'([^']*)'/g)].map((m) => m[1]);
		for (const chunk of literals.length > 0 ? literals : [written]) {
			for (const word of chunk.split(/\s+/)) {
				if (word.startsWith('res-')) {
					found.add(word);
				}
			}
		}
	}
	for (const match of source.matchAll(/class:(res-[a-z0-9-]+)/g)) {
		found.add(match[1]);
	}
	return found;
}

/** The stylesheet split into rules, each as its selector and its body. */
function rulesIn(source: string): { selector: string; body: string }[] {
	const stripped = source.replace(/\/\*[\s\S]*?\*\//g, '');
	return [...stripped.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((match) => ({
		selector: match[1].trim(),
		body: match[2]
	}));
}

describe('the resource form is one surface of bands', () => {
	it('cannot silently regain the card per section', () => {
		// Nine cards abutting inside one form draw no ground between them, so
		// the shape they read as is one white slab seamed by a shadow. The band
		// is what replaced it, and `panel` is what would bring it back.
		expect(FORM_SECTION).not.toMatch(/class="[^"]*\bpanel\b/);
		expect(FORM_SECTION).toContain('class="res-sec"');
	});

	it('passes every band an icon the console already has', () => {
		// Eleven bands on the canonical tab. The two per-marketplace panels are
		// headed by the marketplace's own mark instead, which is what says
		// whose options they hold.
		const passed = [
			...RESOURCE_FORM.matchAll(/<FormSection\b[^>]*?\bicon="([^"]+)"/gs)
		].map((match) => match[1]);
		expect(passed.length).toBe(11);
		expect(passed.filter((name) => !ICON_NAMES.includes(name as never))).toEqual([]);
	});

	it('declares every res- class its pages write, and writes every one it declares', () => {
		const written = new Set<string>();
		for (const file of readdirSync(HERE).filter((name) => name.endsWith('.svelte'))) {
			for (const name of classesIn(read(file))) {
				written.add(name);
			}
		}
		for (const name of classesIn(FORM_SECTION)) {
			written.add(name);
		}
		const declared = new Set([...css.matchAll(/\.(res-[a-z0-9-]+)/g)].map((match) => match[1]));
		expect([...written].filter((name) => !declared.has(name)).sort()).toEqual([]);
		expect([...declared].filter((name) => !written.has(name)).sort()).toEqual([]);
	});

	it('sets no control height in the form except through the shared token', () => {
		// Scoped to the classes this form owns rather than to the whole sheet:
		// the board's own tiles and two off-screen file inputs carry heights of
		// their own that predate the form and are not what this guards.
		// `(?![\w-])` rather than `\b`, so `res-sec-mark` — a logo, which has a
		// size of its own — is not read as the `res-sec` band.
		const OWNED = /\.res-(card|sec|sec-h|sec-ico|sec-help|actbar|row|picks|pick)(?![\w-])/;
		const offending = rulesIn(css)
			.filter((rule) => OWNED.test(rule.selector))
			.filter((rule) =>
				[...rule.body.matchAll(/(?:^|;)\s*(?:min-|max-)?height\s*:\s*([^;]+)/g)].some(
					(match) => !/var\(--control-h(-sm)?\)/.test(match[1])
				)
			);
		expect(offending.map((rule) => rule.selector)).toEqual([]);
	});
});

describe('what the form says is required, it says out loud', () => {
	it('renders the word rather than an asterisk a screen reader reads as punctuation', () => {
		for (const file of ['Field.svelte', 'FacetPicker.svelte', 'GradeGrid.svelte']) {
			const source = readFileSync(`${LIB}${file}`, 'utf8');
			expect(source).toContain('<span class="req">Required</span>');
			expect(source).not.toMatch(/class="req"[^>]*aria-hidden/);
		}
	});

	it('marks the control itself, not only its label', () => {
		// A `Field` that says Required to a reader and carries no `required`
		// nor `aria-required` on the control announces nothing to a screen
		// reader, which is the state five of these six controls were in.
		const required = [...`${RESOURCE_FORM}${MARKETPLACE_PICKER}`.matchAll(
			/<Field\b[^>]*?\bid="([^"]+)"[^>]*?\brequired\b/gs
		)].map((match) => match[1]);
		expect(required.length).toBeGreaterThan(0);
		const unmarked = required.filter((id) => {
			const control = new RegExp(`<(?:input|select|textarea)\\b[^>]*?id="${id}"[^>]*?>`, 's');
			const tag = `${RESOURCE_FORM}${MARKETPLACE_PICKER}`.match(control)?.[0] ?? '';
			return !/\brequired\b/.test(tag) && !/\baria-required="true"/.test(tag);
		});
		expect(unmarked).toEqual([]);
	});
});

describe('a read in flight is drawn as pending', () => {
	it('gives every band that gates its whole content on the vocabulary an else branch', () => {
		// A band whose entire content sits behind `{#if form}` renders a
		// heading over nothing while the read is in flight, and one surface of
		// bands makes that more conspicuous than a dozen cards did. A gate
		// around one counter is not that case, so the shape is what is
		// measured rather than the count.
		const bands = [...RESOURCE_FORM.matchAll(/<FormSection\b[\s\S]*?<\/FormSection>/g)].map(
			(match) => match[0]
		);
		expect(bands.length).toBeGreaterThanOrEqual(11);
		const whole = bands.filter((band) => /<\/FormSection>/.test(band) && /\{#if form\}/.test(band));
		const silent = whole.filter((band) => {
			const opened = band.indexOf('{#if form}');
			const rest = band.slice(opened);
			// The gate wraps the band's own content where nothing but
			// whitespace precedes it after the opening tag.
			const head = band.slice(band.indexOf('>') + 1, opened);
			return head.trim().length === 0 && !rest.includes('{:else}');
		});
		expect(silent).toEqual([]);
	});
});
