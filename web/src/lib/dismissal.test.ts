// Three things the type system cannot check on its own, in the manner
// `icons.test.ts` already establishes.
//
// The union guarantees that a key the compiler can see is one this module
// declares. It cannot see a key written in a file that has not been type
// checked yet, which on a shared tree is any file another slice is still
// editing; it cannot say whether two keys quietly share a storage string; and
// it says nothing at all about the size of the control that writes them, which
// with no rendering lane is otherwise held by eye alone.

import { readFileSync, readdirSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
	DISMISS_KEYS,
	DISMISS_NAMES,
	type DismissalStore,
	remember,
	remembered
} from './dismissal';

const SOURCE = new URL('../', import.meta.url);

function fake(): DismissalStore {
	const held = new Map<string, string>();
	return {
		getItem: (key) => held.get(key) ?? null,
		setItem: (key, value) => void held.set(key, value)
	};
}

const refusing: DismissalStore = {
	getItem() {
		throw new Error('The operation is insecure.');
	},
	setItem() {
		throw new Error('The operation is insecure.');
	}
};

describe('a remembered dismissal', () => {
	it('reads back what it wrote', () => {
		const store = fake();
		expect(remembered('templates.licence-is-yours', store)).toBe(false);
		remember('templates.licence-is-yours', store);
		expect(remembered('templates.licence-is-yours', store)).toBe(true);
	});

	it('does not answer for a key nobody dismissed', () => {
		const store = fake();
		remember('templates.licence-is-yours', store);
		expect(remembered('analytics.tpt-reports-only', store)).toBe(false);
	});

	it('reads false rather than throwing when storage refuses to be read', () => {
		expect(remembered('templates.licence-is-yours', refusing)).toBe(false);
	});

	it('writes nothing and throws nothing when storage refuses the write', () => {
		// The caller's own visit-scoped dismissal is what stands in this case,
		// which is what every one of these pages had before it remembered
		// anything.
		expect(() => remember('templates.licence-is-yours', refusing)).not.toThrow();
	});
});

describe('the declared keys', () => {
	it('give every name a storage string', () => {
		const blank = DISMISS_NAMES.filter((name) => !DISMISS_KEYS[name]);
		expect(blank).toEqual([]);
	});

	it('give no two names the same storage string', () => {
		const written = DISMISS_NAMES.map((name) => DISMISS_KEYS[name]);
		expect(new Set(written).size).toBe(written.length);
	});
});

function sourceFiles(dir: URL, prefix: string, suffix: string): { path: string; text: string }[] {
	const found: { path: string; text: string }[] = [];
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		if (entry.name === 'generated' || entry.name === 'node_modules') {
			continue;
		}
		const path = `${prefix}${entry.name}`;
		if (entry.isDirectory()) {
			found.push(...sourceFiles(new URL(`${entry.name}/`, dir), `${path}/`, suffix));
		} else if (entry.name.endsWith(suffix)) {
			found.push({ path, text: readFileSync(new URL(entry.name, dir), 'utf8') });
		}
	}
	return found;
}

/** Every key a component passes to this module as a literal, with where it came
 *  from. A key reached through a variable is the type checker's. */
function literalKeys(): Map<string, string[]> {
	const found = new Map<string, string[]>();
	for (const { path, text } of sourceFiles(SOURCE, '', '.svelte')) {
		for (const match of text.matchAll(/\bremember(?:ed)?\(\s*'([^']*)'/g)) {
			found.set(match[1], [...(found.get(match[1]) ?? []), path]);
		}
	}
	return found;
}

describe('the keys components actually write', () => {
	it('reads a real component tree, so an empty sweep cannot pass silently', () => {
		expect(literalKeys().size).toBeGreaterThanOrEqual(DISMISS_NAMES.length);
	});

	it('are all declared, so a page cannot invent one no test knows about', () => {
		const declared = new Set<string>(DISMISS_NAMES);
		const undeclared = [...literalKeys()]
			.filter(([key]) => !declared.has(key))
			.map(([key, files]) => `${key} (${files[0]})`);
		expect(undeclared).toEqual([]);
	});
});

/** Every declaration block written for a selector, so a rule added later
 *  inside a media query cannot displace the one carrying the metric. */
function blocksFor(sheet: string, selector: string): string[] {
	const found: string[] = [];
	let from = 0;
	for (;;) {
		const start = sheet.indexOf(`${selector} {`, from);
		if (start === -1) {
			return found;
		}
		const end = sheet.indexOf('}', start);
		found.push(sheet.slice(start, end));
		from = end;
	}
}

describe('the close controls', () => {
	// Crude on purpose. `svelte-check` cannot see a stylesheet and no rendering
	// lane gates a merge here, so a grep over the two sheets is the only
	// automated hold on the metric these controls were raised to meet.
	const SHEETS: [string, string][] = [
		['lib/styles/components.css', '.banner-close'],
		['app.css', '.toast-close']
	];

	for (const [file, selector] of SHEETS) {
		it(`gives ${selector} the full control height and width`, () => {
			const sheet = readFileSync(new URL(file, SOURCE), 'utf8');
			const blocks = blocksFor(sheet, selector);
			expect(blocks.length).toBeGreaterThan(0);
			const sized = blocks.filter(
				(block) =>
					/min-width:\s*var\(--control-h\)/.test(block) &&
					/min-height:\s*var\(--control-h\)/.test(block)
			);
			expect(sized).toHaveLength(1);
		});
	}
});

describe('closing a notification with the keyboard', () => {
	// Crude for the same reason as the block above: what holds Escape on these
	// two surfaces is where the handler is attached, and no lane here renders
	// them. The pairing below is the whole of the claim -- the key is answered
	// on the notification's own element, it runs the call the click runs, and
	// no listener is fitted to the window or the document, which is what would
	// take Escape away from a dialog open over the page.
	//
	// The picker's needle is the statement run rather than the verb. It closes
	// from four places, so a window wide enough to hold the Escape path also
	// reaches one of the others, and `closes(` contains the verb as well;
	// either would let an inlined `open = false` pass. The run opens with the
	// default prevented: Escape in a search box holding text clears it, and
	// the `input` that clearing dispatches would reopen the picker.
	const SURFACES: [string, string, string][] = [
		['lib/Banner.svelte', 'banner', 'close'],
		['routes/+layout.svelte', 'toast', 'closeToast'],
		[
			'lib/FacetPicker.svelte',
			'field fp',
			'event.preventDefault();\n\t\tevent.stopPropagation();\n\t\tvoid close();'
		]
	];

	for (const [file, marker, dismisses] of SURFACES) {
		const source = readFileSync(new URL(file, SOURCE), 'utf8');

		it(`answers Escape on the ${marker}'s own element`, () => {
			// `[^<]*` cannot cross into another tag, so this is the handler on
			// that element rather than one anywhere in the file.
			expect(new RegExp(`class="${marker}[^<]*onkeydown=`).test(source)).toBe(true);
		});

		it(`closes the ${marker} on Escape by the call its click makes`, () => {
			const at = source.indexOf("event.key !== 'Escape'");
			expect(at).toBeGreaterThan(-1);
			expect(source.slice(at, at + 200)).toContain(dismisses);
		});

		it(`leaves every other Escape to whatever is over the ${marker}`, () => {
			// The tag rather than the adjacency: a surface that fits a second
			// window handler puts the key one attribute away from the first. The
			// scan runs to the tag's own `/>` rather than to the first `>`, which
			// an arrow function in an earlier attribute supplies early.
			expect(/<svelte:window(?:(?!\/>)[\s\S])*onkeydown/.test(source)).toBe(false);
			expect(source).not.toContain("addEventListener('keydown'");
		});
	}
});
