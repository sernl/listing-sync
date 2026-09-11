// The palette choice, which is three answers rather than two and has to
// survive a store that refuses to hold it.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

import {
	DARK_QUERY,
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
	it('is system where nothing was ever chosen', () => {
		expect(themeChoice(store())).toBe('system');
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
	it('reads a word it does not know as system', () => {
		expect(themeChoice(store({ [THEME_KEY]: 'sepia' }))).toBe('system');
		expect(isThemeChoice('sepia')).toBe(false);
		expect(isThemeChoice(undefined)).toBe(false);
	});

	/** Safari's private mode throws from both halves of `Storage`. */
	it('survives a store that throws, and says the write did not land', () => {
		expect(themeChoice(refusing())).toBe('system');
		expect(setThemeChoice(refusing(), 'dark')).toBe(false);
	});

	it('survives no store at all, which is what a server render has', () => {
		expect(themeChoice(null)).toBe('system');
		expect(setThemeChoice(undefined, 'dark')).toBe(false);
	});

	/** System is written rather than removed: a seller on a light machine who
	 *  picks System after picking Dark has made a choice, and an absent key
	 *  cannot be told apart from never having chosen. */
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
 *  first paint is the wrong colour. So the two constants it hard-codes are
 *  compared against the ones here, and a rename that touched only this file
 *  fails the lane. */
describe('the first-paint script agrees with this module', () => {
	const html = new URL('../app.html', import.meta.url).pathname;

	it('reads the same key and the same query', () => {
		const body = readFileSync(html, 'utf8');
		expect(body).toContain(`'${THEME_KEY}'`);
		expect(body).toContain(`'${DARK_QUERY}'`);
	});
});
