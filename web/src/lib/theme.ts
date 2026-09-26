// Which of the two palettes the console paints, and where that choice lives.
//
// Three values and not two: `light`, `dark`, and `system`, which defers to
// `prefers-color-scheme` for a seller who wants the console to follow their
// machine. `light` is the default, the founder's call of 2026-09-26: an
// unconfigured console opens light whatever the machine prefers, and `system`
// is something a seller picks rather than something they are given.
//
// The choice is held in `localStorage` under one key, which `web/src/app.html`
// reads in an inline script before the stylesheet so the first paint is
// already the right colour. There is no server-side preference yet: carrying
// it on the account is the Capabilities work in phase 1 of
// `docs/notes/design/2026-09-12-one-marketplace-per-site-and-the-seller-workflows.md`,
// and until then a seller who signs in on a second machine chooses again
// there.
//
// Everything below is a pure function of a `Storage` and a `MediaQueryList`
// handed in, so the whole module tests without a browser and the same code
// runs in the inline script's place at run time.

import { desktopInvoker } from '$lib/desktop';

/** What the seller chose. `system` is following the machine, said out loud. */
export type ThemeChoice = 'system' | 'light' | 'dark';

/** What a console with no readable choice paints. */
export const DEFAULT_CHOICE: ThemeChoice = 'light';

/** What is actually painted. `system` never reaches a stylesheet. */
export type Theme = 'light' | 'dark';

/** The one key, named here so the console, `app.html` and the landing site's
 *  own script cannot drift apart on its spelling. */
export const THEME_KEY = 'teachouse.theme';

/** The media query that answers `system`. */
export const DARK_QUERY = '(prefers-color-scheme: dark)';

/** The desktop command that moves the application window's own chrome — the
 *  title bar and the scrollbars the webview does not own — onto the seller's
 *  choice. */
export const SET_THEME = 'set_theme';

const CHOICES: readonly ThemeChoice[] = ['system', 'light', 'dark'];

/** Whether a stored string is still one of the three.
 *
 *  Anything else is read as the default rather than repaired or reported: the
 *  value is a preference in storage a seller can edit, an older build may have
 *  written something this one does not know, and neither is a fault worth a
 *  message. */
export function isThemeChoice(value: unknown): value is ThemeChoice {
	return typeof value === 'string' && (CHOICES as readonly string[]).includes(value);
}

/** The seller's choice, or the default where there is none or the store
 *  refuses.
 *
 *  A refusal is ordinary rather than exceptional: Safari's private mode throws
 *  from `getItem` when storage is disabled, and a console that cannot remember
 *  a palette still has to draw one. */
export function themeChoice(store: Storage | null | undefined): ThemeChoice {
	if (!store) return DEFAULT_CHOICE;
	let stored: string | null;
	try {
		stored = store.getItem(THEME_KEY);
	} catch {
		return DEFAULT_CHOICE;
	}
	return isThemeChoice(stored) ? stored : DEFAULT_CHOICE;
}

/** Record the choice, and say whether it was recorded.
 *
 *  `system` is written rather than removed, because an absent key now reads
 *  as light, so removing it would quietly turn a seller who asked to follow a
 *  dark machine back to light. */
export function setThemeChoice(store: Storage | null | undefined, choice: ThemeChoice): boolean {
	if (!store) return false;
	try {
		store.setItem(THEME_KEY, choice);
		return true;
	} catch {
		return false;
	}
}

/** What a choice paints, given what the machine prefers. */
export function resolvedTheme(choice: ThemeChoice, prefersDark: boolean): Theme {
	if (choice === 'system') return prefersDark ? 'dark' : 'light';
	return choice;
}

/** The whole of the choice, applied: store it, paint it, and tell the desktop
 *  window about it.
 *
 *  `dataset.theme` is always one of the two words and never absent, so
 *  `[data-theme='light']` can pin `color-scheme` rather than leaving the
 *  resolved scheme to the reader's system while the page is painted the other
 *  way.
 *
 *  The window's own chrome is outside the webview, so a dark page in a light
 *  title bar is what happens without the last step. In a browser there is no
 *  invoker and the step is skipped; a refusal from the command is swallowed,
 *  because a title bar that stayed light is not worth a message over a page
 *  that is already the colour the seller asked for. */
export function commitThemeChoice(
	store: Storage | null | undefined,
	root: { dataset: DOMStringMap },
	prefersDark: boolean,
	choice: ThemeChoice,
): Theme {
	setThemeChoice(store, choice);
	const theme = resolvedTheme(choice, prefersDark);
	root.dataset.theme = theme;
	void tellDesktop(choice);
	return theme;
}

/** Hand the choice to the application window, where there is one. Exported for
 *  the test, which has no Tauri global to stub out. */
export async function tellDesktop(choice: ThemeChoice): Promise<void> {
	const invoke = desktopInvoker();
	if (!invoke) return;
	try {
		await invoke(SET_THEME, { theme: choice });
	} catch {
		// A build of the application older than the command refuses it by
		// name. The page is already painted.
	}
}
