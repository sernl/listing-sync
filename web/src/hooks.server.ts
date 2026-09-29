import { existsSync, readFileSync } from 'node:fs';
import type { Handle } from '@sveltejs/kit';
import { building } from '$app/environment';
import inter from '$lib/fonts/inter-400-700-latin.woff2?url';
import poppins from '$lib/fonts/poppins-600-latin.woff2?url';
import { AUTH_PRELOADS, preloadScript } from '$lib/shell-preload';

// Runs once, at build time, when the adapter renders the single shell every
// path is answered with; the console has no server of its own.
//
// The two faces the first screen sets, preloaded under the content-hashed
// names the stylesheet asks for them by. `crossorigin` is required even
// same-origin: a font is always fetched in CORS mode, and a preload made
// without it is a different request the stylesheet's cannot reuse.
const FONT_PRELOADS = [inter, poppins]
	.map((href) => `<link rel="preload" href="${href}" as="font" type="font/woff2" crossorigin>`)
	.join('\n    ');

/** The signed-out screens' chunk lists as the client build left them. Absent
 *  under `vite dev`, which has no chunks, and then nothing is preloaded. */
function authPreloads(): string {
	if (!building || !existsSync(AUTH_PRELOADS)) {
		return '';
	}
	return preloadScript(JSON.parse(readFileSync(AUTH_PRELOADS, 'utf8')));
}

export const handle: Handle = ({ event, resolve }) =>
	resolve(event, {
		transformPageChunk: ({ html }) =>
			html.replace('%teachouse.fonts%', FONT_PRELOADS).replace('%teachouse.auth%', authPreloads())
	});
