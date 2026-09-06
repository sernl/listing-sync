// Which explainers a viewer has closed for good, and where that is written.
//
// The Labels board had this inline and was the only page in the console that
// remembered anything. Two things made it worth lifting rather than copying:
// the storage key was a bare string, so a second page could take the same one
// and silently close both banners at once, and the guard against a browser
// that refuses storage had to be rewritten correctly at every site.
//
// The union below closes the first hole — a key no one declared fails
// `svelte-check` rather than reaching a seller — and the `Record` over it
// keeps every storage string in one place where a collision is visible.
//
// Only an explainer belongs here. A banner about state that changes is not
// dismissed for good, because remembering it hides a live condition on that
// machine and on no other.

/** Every explainer a viewer can close for good. */
export type DismissKey =
	| 'labels.what-are-labels'
	| 'templates.licence-is-yours'
	| 'sync.schedule-is-fixed'
	| 'sharing.not-built'
	| 'analytics.tpt-reports-only';

/** Where each dismissal is written.
 *
 * `labels.what-are-labels` keeps the string the Labels board already wrote, so
 * a viewer who dismissed that banner before this module existed does not meet
 * it again. */
export const DISMISS_KEYS: Record<DismissKey, string> = {
	'labels.what-are-labels': 'labels.what-are-labels.dismissed',
	'templates.licence-is-yours': 'templates.licence-is-yours.dismissed',
	'sync.schedule-is-fixed': 'sync.schedule-is-fixed.dismissed',
	'sharing.not-built': 'sharing.not-built.dismissed',
	'analytics.tpt-reports-only': 'analytics.tpt-reports-only.dismissed'
};

export const DISMISS_NAMES: readonly DismissKey[] = Object.keys(DISMISS_KEYS) as DismissKey[];

/** The part of `Storage` this needs, so a test can pass one that throws. */
export interface DismissalStore {
	getItem(key: string): string | null;
	setItem(key: string, value: string): void;
}

const YES = 'yes';

function store(given: DismissalStore | undefined): DismissalStore | undefined {
	return given ?? (globalThis as { localStorage?: DismissalStore }).localStorage;
}

/** Whether this viewer has already closed that explainer.
 *
 * False whenever storage cannot be read at all — a private window, storage
 * disabled, or no `localStorage` in this runtime — which degrades the
 * dismissal to the visit rather than throwing on a page that has nothing to do
 * with it. */
export function remembered(key: DismissKey, given?: DismissalStore): boolean {
	try {
		return store(given)?.getItem(DISMISS_KEYS[key]) === YES;
	} catch {
		return false;
	}
}

/** Record that this viewer closed that explainer.
 *
 * A storage that refuses the write leaves the caller's own visit-scoped
 * dismissal standing, which is what the page had before it remembered
 * anything. */
export function remember(key: DismissKey, given?: DismissalStore): void {
	try {
		store(given)?.setItem(DISMISS_KEYS[key], YES);
	} catch {
		// Deliberately silent: see above.
	}
}
