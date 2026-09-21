// Two things the type system cannot check on its own.
//
// The union guarantees that a `name` the compiler can see resolves to a key of
// `ICONS`. It does not guarantee that the key's value is a component: a deep
// import path that happens to exist but exports no default would satisfy the
// type and render nothing. And it cannot see a name written in a file that has
// not been type-checked yet, which on a shared tree is any file a slice is
// still editing.

import { readFileSync, readdirSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { ICONS, ICON_NAMES } from './icons';

const SOURCE = new URL('../', import.meta.url);

function sourceFiles(dir: URL, prefix: string): { path: string; text: string }[] {
	const found: { path: string; text: string }[] = [];
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		if (entry.name === 'generated' || entry.name === 'node_modules') {
			continue;
		}
		const path = `${prefix}${entry.name}`;
		if (entry.isDirectory()) {
			found.push(...sourceFiles(new URL(`${entry.name}/`, dir), `${path}/`));
		} else if (entry.name.endsWith('.svelte')) {
			found.push({ path, text: readFileSync(new URL(entry.name, dir), 'utf8') });
		}
	}
	return found;
}

/** Every icon name written as a literal in a component, with where it came
 *  from.
 *
 * `name=` is matched only inside an `<Icon` tag, because a bare `name=` is an
 * ordinary HTML attribute — the search box, the bulk dialogs and the form
 * radios all carry one, and counting those made this look broken when it was
 * not. `icon=` needs no such guard, since only our own components take it. */
function literalNames(): Map<string, string[]> {
	const found = new Map<string, string[]>();
	const record = (name: string, path: string, into: Map<string, string[]>) =>
		into.set(name, [...(into.get(name) ?? []), path]);
	for (const { path, text } of sourceFiles(SOURCE, '')) {
		const sites = [
			...text.matchAll(/<Icon\b[^>]*?\bname=(?:"([a-z0-9-]+)"|\{([^}]*)\})/g),
			...text.matchAll(/\bicon=(?:"([a-z0-9-]+)"|\{([^}]*)\})/g)
		];
		for (const match of sites) {
			if (match[1] !== undefined) {
				record(match[1], path, found);
				continue;
			}
			// A ternary or a lookup: take the quoted arms and ignore the rest,
			// since a name reached through a variable is the type checker's.
			for (const arm of (match[2] ?? '').matchAll(/'([a-z0-9-]+)'/g)) {
				record(arm[1], path, found);
			}
		}
	}
	return found;
}

describe('the icon set', () => {
	it('resolves every name to a component that actually exists', () => {
		const empty = ICON_NAMES.filter((name) => ICONS[name] === undefined);
		expect(empty).toEqual([]);
	});

	it('holds no duplicate name', () => {
		expect(new Set(ICON_NAMES).size).toBe(ICON_NAMES.length);
	});

	it('reads a real component tree, so an empty sweep cannot pass silently', () => {
		expect(literalNames().size).toBeGreaterThan(20);
	});

	// The verbs, as opposed to the destinations. Every page and every nav
	// entry names its glyph in a typed field, so the type check catches a
	// missing one; an action's glyph is passed at the call site, and a slice
	// that reaches for `pencil` before the registry has it gets a type error
	// in its own file rather than an answer here. This pins the set so the
	// registry cannot lose one while the call sites are still being written.
	it('carries a glyph for every action verb the console draws', () => {
		const verbs = [
			'bold',
			'italic',
			'list',
			'trash-2',
			'pencil',
			'upload',
			'external-link',
			'filter',
			'chevron-right',
			'chevron-left',
			'clock',
			'credit-card',
			'sparkles',
			'shopping-bag',
			'gift',
			'calendar'
		];
		const absent = verbs.filter((name) => !(name in ICONS));
		expect(absent).toEqual([]);
	});

	it('carries every name a component writes as a literal', () => {
		const shipped = new Set<string>(ICON_NAMES);
		const missing = [...literalNames()]
			.filter(([name]) => !shipped.has(name))
			.map(([name, files]) => `${name} (${files[0]})`);
		expect(missing).toEqual([]);
	});
});
