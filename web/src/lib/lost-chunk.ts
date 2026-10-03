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
//
// A chunk can also neither arrive nor fail. On a cold Cloudflare edge a few
// of a first load's requests stalled for 20-30 s (0.19.1), and the browser
// raises nothing until its own timeout, so the founder watched a blank page
// for half a minute before the error above could even happen. The shell
// therefore carries a watchdog (`START_WATCHDOG`) that reloads, under the same
// once-per-window guard, when the first screen has not been drawn in
// `START_TIMEOUT_MS`.

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

/** How long SvelteKit's start -- its two entries, the route's chunks and the
 *  first draw -- may take before the shell reloads. Every chunk is answered
 *  by the origin in well under a second, so this is only ever a stall. */
export const START_TIMEOUT_MS = 8_000;

/** The inline script the shell carries ahead of SvelteKit's entry, written
 *  in by `hooks.server.ts`. It cannot be a module: what it watches is the
 *  module graph arriving, and `hooks.client.ts` is part of that graph.
 *
 *  At `START_TIMEOUT_MS` it looks for anything SvelteKit has drawn beside its
 *  own bootstrap script in the body's wrapper (`app.html`); the router draws
 *  only once every chunk of the route has arrived, so nothing there means
 *  start has not finished. It then reloads under the same record
 *  `mayReloadForLostChunk` keeps, so a stall and a failure together reload
 *  once per window between them. Storage that refuses means no reload,
 *  because without a record the guard cannot hold.
 *
 *  Plain ES5 for the same reason as the shell's other inline blocks, and
 *  hashed into the policy with them by `tam-server`. */
export const START_WATCHDOG = `<script>
      (function () {
        var key = ${JSON.stringify(KEY)};
        setTimeout(function () {
          if (document.querySelector('body > div > :not(script)')) return;
          var now = Date.now();
          try {
            var record = sessionStorage.getItem(key);
            var last = record === null ? -Infinity : Number(record);
            if (isFinite(last) && now - last < ${WINDOW_MS}) return;
            sessionStorage.setItem(key, String(now));
          } catch (refused) {
            return;
          }
          location.reload();
        }, ${START_TIMEOUT_MS});
      })();
    </script>`;
