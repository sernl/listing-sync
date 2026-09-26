// The palette choice, which is three answers rather than two and has to
// survive a store that refuses to hold it.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

import {
	DARK_QUERY,
	DEFAULT_CHOICE,
	THEME_KEY,
	commitThemeChoice,
	isThemeChoice,
	resolvedTheme,
	setThemeChoice,
	themeChoice,
	type ThemeChoice,
} from './theme';

/** A `Storage` that holds what it is given, and one that will not. */
function store(seed?: Record<string, string>): Storage {
	const held = new Map(Object.entries(seed ?? {}));
	return {
		get length() {
			return held.size;
		},
		clear: () => held.clear(),
		getItem: (key: string) => held.get(key) ?? null,
		key: (index: number) => [...held.keys()][index] ?? null,
		removeItem: (key: string) => void held.delete(key),
		setItem: (key: string, value: string) => void held.set(key, value),
	};
}

function refusing(): Storage {
	const thrower = () => {
		throw new DOMException('The operation is insecure.', 'SecurityError');
	};
	return { ...store(), getItem: thrower, setItem: thrower };
}

describe('the stored choice', () => {
	it('is light where nothing was ever chosen', () => {
		expect(DEFAULT_CHOICE).toBe('light');
		expect(themeChoice(store())).toBe('light');
	});

	it.each(['system', 'light', 'dark'] as const)('reads back %s', (choice) => {
		const held = store();
		expect(setThemeChoice(held, choice)).toBe(true);
		expect(held.getItem(THEME_KEY)).toBe(choice);
		expect(themeChoice(held)).toBe(choice);
	});

	/** A value this build does not know is a preference an older build or the
	 *  seller's own devtools wrote. Defaulting is the whole handling: a
	 *  console that cannot read a palette name still has to paint one. */
	it('reads a word it does not know as light', () => {
		expect(themeChoice(store({ [THEME_KEY]: 'sepia' }))).toBe('light');
		expect(isThemeChoice('sepia')).toBe(false);
		expect(isThemeChoice(undefined)).toBe(false);
	});

	/** Safari's private mode throws from both halves of `Storage`. */
	it('survives a store that throws, and says the write did not land', () => {
		expect(themeChoice(refusing())).toBe('light');
		expect(setThemeChoice(refusing(), 'dark')).toBe(false);
	});

	it('survives no store at all, which is what a server render has', () => {
		expect(themeChoice(null)).toBe('light');
		expect(setThemeChoice(undefined, 'dark')).toBe(false);
	});

	/** System is written rather than removed: an absent key reads as light,
	 *  so clearing it would turn a seller following a dark machine light. */
	it('writes system rather than clearing the key', () => {
		const held = store({ [THEME_KEY]: 'dark' });
		setThemeChoice(held, 'system');
		expect(held.getItem(THEME_KEY)).toBe('system');
	});
});

describe('what a choice paints', () => {
	it.each([
		['system', false, 'light'],
		['system', true, 'dark'],
		['light', true, 'light'],
		['dark', false, 'dark'],
	] as Array<[ThemeChoice, boolean, string]>)(
		'%s on a machine preferring dark=%s paints %s',
		(choice, prefersDark, painted) => {
			expect(resolvedTheme(choice, prefersDark)).toBe(painted);
		},
	);
});

describe('committing a choice', () => {
	it('stores it and paints the element in one step', () => {
		const held = store();
		const root = { dataset: {} as DOMStringMap };
		expect(commitThemeChoice(held, root, true, 'light')).toBe('light');
		expect(root.dataset.theme).toBe('light');
		expect(held.getItem(THEME_KEY)).toBe('light');
	});

	/** The attribute is never absent, so `[data-theme='light']` can pin
	 *  `color-scheme` instead of leaving the resolved scheme to the reader's
	 *  system while the page is painted the other way. */
	it('paints light explicitly when system resolves light', () => {
		const root = { dataset: {} as DOMStringMap };
		commitThemeChoice(store(), root, false, 'system');
		expect(root.dataset.theme).toBe('light');
	});

	it('still paints when the store refuses to remember', () => {
		const root = { dataset: {} as DOMStringMap };
		expect(commitThemeChoice(refusing(), root, false, 'dark')).toBe('dark');
		expect(root.dataset.theme).toBe('dark');
	});
});

/** The inline script in `app.html` is the same decision run before the
 *  stylesheet, and it cannot import this module: it has to be inline or the
 *  first paint is the wrong colour. The landing site's `Base.astro` carries a
 *  third copy for the same reason. So the constants the console's copy
 *  hard-codes are compared against the ones here, and both copies are run
 *  against a stub page and must paint what `resolvedTheme(themeChoice())`
 *  says, so a default changed in one place fails the lane. */
describe('the first-paint scripts agree with this module', () => {
	const html = new URL('../app.html', import.meta.url).pathname;
	const base = new URL('../../../apps/landing/src/layouts/Base.astro', import.meta.url).pathname;

	/** The body of the first inline script in a file. */
	const firstScript = (path: string, opener: string): string => {
		const body = readFileSync(path, 'utf8');
		const start = body.indexOf(opener) + opener.length;
		return body.slice(start, body.indexOf('</script>', start));
	};

	/** Run one copy on a stub page: what it painted, and what it told the
	 *  desktop window, if it has a window to tell. */
	const run = (script: string, stored: string | null, prefersDark: boolean) => {
		const root = { dataset: {} as DOMStringMap };
		const told: string[] = [];
		const media = { matches: prefersDark, addEventListener: () => {} };
		const window = {
			matchMedia: (query: string) => {
				expect(query).toBe(DARK_QUERY);
				return media;
			},
			__TAURI__: {
				core: {
					invoke: (command: string, args: { theme: string }) => {
						told.push(`${command}:${args.theme}`);
						return Promise.resolve();
					},
				},
			},
		};
		const held = store(stored === null ? {} : { [THEME_KEY]: stored });
		new Function('window', 'localStorage', 'document', script)(window, held, {
			documentElement: root,
		});
		return { painted: root.dataset.theme, told };
	};

	it('reads the same key and the same query', () => {
		const body = readFileSync(html, 'utf8');
		expect(body).toContain(`'${THEME_KEY}'`);
		expect(body).toContain(`'${DARK_QUERY}'`);
	});

	const cases = [null, 'sepia', 'system', 'light', 'dark'].flatMap((stored) =>
		[false, true].map((prefersDark): [string | null, boolean] => [stored, prefersDark]),
	);

	it.each(cases)('stored %s on a machine preferring dark=%s', (stored, prefersDark) => {
		const expected = resolvedTheme(
			themeChoice(store(stored === null ? {} : { [THEME_KEY]: stored })),
			prefersDark,
		);
		expect(run(firstScript(html, '<script>'), stored, prefersDark).painted).toBe(expected);
		expect(run(firstScript(base, '<script is:inline>'), stored, prefersDark).painted).toBe(expected);
	});

	/** The window's title bar is outside the page, so the console's copy
	 *  hands it the choice on every load; `system` travels as the word. */
	it('tells the desktop window the choice, not the resolved palette', () => {
		const script = firstScript(html, '<script>');
		expect(run(script, null, true).told).toEqual(['set_theme:light']);
		expect(run(script, 'system', true).told).toEqual(['set_theme:system']);
		expect(run(script, 'dark', false).told).toEqual(['set_theme:dark']);
	});
});
