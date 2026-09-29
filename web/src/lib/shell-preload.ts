// What the shell asks for before its own script has run.
//
// The console is one shell for every path, so what it preloads by itself is
// only what every path needs: SvelteKit's entry and the root layout. A page's
// own chunks are found by the router once that entry has arrived and run,
// which is a second round trip before anything is drawn. The signed-out
// screens are the one place that trip is worth taking back -- they are the
// first thing a teacher sees, and their whole script is a few dozen kilobytes
// -- so the build lists each one's chunks by path (`authPreloads` in
// `vite.config.ts`) and the shell carries a script that preloads the list for
// the path it was opened at (`hooks.server.ts`).

/** Where the client build leaves the lists, relative to `web/`, for the
 *  shell's render to read; `.svelte-kit` because it is the build's own and
 *  is never served. */
export const AUTH_PRELOADS = '.svelte-kit/auth-preloads.json';

/** The path a module under `src/routes/(auth)/` is the page for, or null for
 *  a module that is not an `(auth)` page -- the group's layout among them,
 *  which every one of the paths needs and so belongs to all of them. */
export function authPageOf(moduleId: string): string | null {
	const match = /\/src\/routes\/\(auth\)((?:\/[^/+]+)*)\/\+page\.(?:svelte|ts)$/.exec(moduleId);
	return match === null || match[1] === '' ? null : match[1];
}

/** The origins the signed-out screens reach beyond their own: Cloudflare's
 *  Turnstile check, whose script and frame the forms load as they mount. The
 *  connection is opened beside the chunks rather than when the form asks. */
export const AUTH_PRECONNECT: readonly string[] = ['https://challenges.cloudflare.com'];

/** The inline script that preloads `files[path]` for the path the shell was
 *  opened at, with a trailing slash ignored, and opens a connection to each
 *  of `AUTH_PRECONNECT` there. A stylesheet is preloaded as a style and
 *  everything else as a module, which is what each is fetched as once the
 *  router asks for it, so the preload is the request it reuses. Every other
 *  path gets nothing: a console page would open a connection it never uses.
 *
 *  Plain ES5 on purpose: it runs before the polyfills matter and on the
 *  oldest WebView the console supports. `tam-server` hashes it into the
 *  policy with every other inline block of the shell. */
export function preloadScript(files: Record<string, readonly string[]>): string {
	return `<script>
      (function () {
        var files = ${JSON.stringify(files)};
        var origins = ${JSON.stringify(AUTH_PRECONNECT)};
        var path = location.pathname.length > 1 ? location.pathname.replace(/\\/+$/, '') : location.pathname;
        if (!Object.prototype.hasOwnProperty.call(files, path)) return;
        var add = function (rel, href, as) {
          var link = document.createElement('link');
          link.rel = rel;
          if (as) link.as = as;
          // Vite's loader asks for each chunk and stylesheet in CORS mode,
          // and a preload made in another mode is a second request.
          if (rel !== 'preconnect') link.crossOrigin = '';
          link.href = href;
          document.head.appendChild(link);
        };
        for (var i = 0; i < origins.length; i++) add('preconnect', origins[i]);
        for (var j = 0; j < files[path].length; j++) {
          var href = files[path][j];
          if (/\\.css$/.test(href)) add('preload', href, 'style');
          else add('modulepreload', href);
        }
      })();
    </script>`;
}
