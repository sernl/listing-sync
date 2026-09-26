// What to do when a piece of the console fails to download.
//
// The console is one shell and many chunks, fetched as the seller moves from
// page to page. A chunk that does not arrive — the phone lost signal for a
// moment, or Cloudflare could not reach the origin for one of the dozen
// connections a page opens — throws from the router's `import()`, and the
// page it belonged to was drawn as "That page could not be opened" with
// nothing to press but a link to a page that needs the same chunks. The
// founder met exactly that on his phone, on every tab, after signing in.
//
// A reload fetches the shell and the chunks again over fresh connections and
// is what Vite's own guidance prescribes for `vite:preloadError`. It is done
// once per window, so a chunk that keeps failing shows the page with its
// cause rather than a phone that reloads forever.

const KEY = 'teachouse.reloaded-for-lost-chunk';
const WINDOW_MS = 30_000;

/** Whether this is a chunk that failed to download, in the words the engines
 *  use: Chromium and WebKitGTK say "dynamically imported module", Safari says
 *  "Importing a module script failed", and Vite's preload helper says
 *  "Unable to preload CSS". */
export function isLostChunk(error: unknown): boolean {
	const message =
		error instanceof Error ? error.message : typeof error === 'string' ? error : '';
	return /dynamically imported module|module script failed|Unable to preload CSS/i.test(message);
}

/** Whether to reload now: true the first time in a window, false until it
 *  passes. `store` and `now` are parameters so a test can turn the window
 *  without waiting for it. */
export function mayReloadForLostChunk(
	store: Pick<Storage, 'getItem' | 'setItem'>,
	now: number = Date.now()
): boolean {
	const record = store.getItem(KEY);
	const last = record === null ? Number.NEGATIVE_INFINITY : Number(record);
	if (Number.isFinite(last) && now - last < WINDOW_MS) {
		return false;
	}
	store.setItem(KEY, String(now));
	return true;
}

/** The sentence the error page shows for a lost chunk, in place of the cause. */
export const LOST_CHUNK_SAID =
	'Part of this page did not download, so it could not be drawn. Check your connection, then reload.';
