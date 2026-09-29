import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import type { Plugin, ProxyOptions, Rollup } from 'vite';
import { visualizer } from 'rollup-plugin-visualizer';
import { defineConfig } from 'vitest/config';
import { AUTH_PRELOADS, authPageOf } from './src/lib/shell-preload';

/** The API sets its session cookie `Secure`, which is right behind TLS and
 *  wrong on the dev origin: `http://localhost:5173` is plain http, and while
 *  Chromium treats localhost as a secure context and keeps the cookie,
 *  WebKitGTK — the desktop app's webview under `just desktop-dev` — drops it,
 *  so the app signed in and was signed out again on its next request. The
 *  attribute is removed here, on the dev proxy alone, and nowhere else. */
const plainHttp: ProxyOptions = {
	target: 'http://127.0.0.1:8080',
	configure: (proxy) => {
		proxy.on('proxyRes', (proxyRes) => {
			const cookies = proxyRes.headers['set-cookie'];
			if (cookies) {
				proxyRes.headers['set-cookie'] = cookies.map((cookie) => cookie.replace(/;\s*Secure/i, ''));
			}
		});
	}
};

/** Lists, for each signed-out screen, every chunk and stylesheet the router
 *  waits for before drawing it that the shell does not already preload: the
 *  `(auth)` layout, the page itself, and everything they import. The root
 *  error page SvelteKit also fetches at start is left out on purpose -- the
 *  first paint does not wait for it, and on a slow line every byte ahead of
 *  that paint is one it does wait for. See `$lib/shell-preload`. */
function authPreloads(): Plugin {
	let root = '';
	let server = false;
	return {
		name: 'teachouse-auth-preloads',
		apply: 'build',
		configResolved(config) {
			root = config.root;
			server = Boolean(config.build.ssr);
		},
		// `writeBundle`, not `generateBundle`: Vite's CSS plugin deletes a chunk
		// that held nothing but stylesheet imports during `generateBundle`, and a
		// list taken earlier would name a file the build never writes.
		writeBundle(_options, bundle) {
			if (server) {
				return;
			}
			const chunks = Object.values(bundle).filter(
				(file): file is Rollup.OutputChunk => file.type === 'chunk'
			);
			const byName = new Map(chunks.map((chunk) => [chunk.fileName, chunk]));
			const holding = (suffix: string) =>
				chunks.filter((chunk) => chunk.moduleIds.some((id) => id.endsWith(suffix)));
			const reach = (from: Rollup.OutputChunk[]): Rollup.OutputChunk[] => {
				const seen = new Map<string, Rollup.OutputChunk>();
				const pending = [...from];
				for (let chunk = pending.pop(); chunk !== undefined; chunk = pending.pop()) {
					if (seen.has(chunk.fileName)) {
						continue;
					}
					seen.set(chunk.fileName, chunk);
					for (const name of chunk.imports) {
						const next = byName.get(name);
						if (next !== undefined) {
							pending.push(next);
						}
					}
				}
				return [...seen.values()];
			};
			// Exactly what SvelteKit writes into the shell as `modulepreload`:
			// its two entries and the root layout.
			const shell = new Set(
				reach([
					...chunks.filter((chunk) => chunk.fileName.startsWith('_app/immutable/entry/')),
					...holding('/src/routes/+layout.ts')
				]).map((chunk) => chunk.fileName)
			);
			const shared = holding('/src/routes/(auth)/+layout.svelte');
			const files: Record<string, string[]> = {};
			for (const chunk of chunks) {
				for (const id of chunk.moduleIds) {
					const path = authPageOf(id);
					if (path === null) {
						continue;
					}
					const reached = reach([...shared, chunk]);
					const css = reached.flatMap((each) => [...(each.viteMetadata?.importedCss ?? [])]);
					const js = reached.map((each) => each.fileName).filter((name) => !shell.has(name));
					files[path] = [...new Set([...css, ...js])].map((name) => `/${name}`);
				}
			}
			mkdirSync(join(root, '.svelte-kit'), { recursive: true });
			writeFileSync(join(root, AUTH_PRELOADS), JSON.stringify(files));
		}
	};
}

export default defineConfig(({ mode }) => ({
  // `PUBLIC_` beside Vite's own prefix, so a build-time public value follows
  // SvelteKit's naming for one rather than Vite's. Nothing without one of
  // the two prefixes reaches the bundle.
  envPrefix: ['VITE_', 'PUBLIC_'],
  // `vite build --mode analyze` also writes the bundle's treemap to
  // `stats.html`, with gzip and brotli sizes: the view that shows which
  // module a chunk is spending its bytes on. Every other build is unchanged.
  plugins: [
    tailwindcss(),
    sveltekit(),
    authPreloads(),
    mode === 'analyze' &&
      visualizer({ filename: 'stats.html', gzipSize: true, brotliSize: true, template: 'treemap' })
  ],
  server: {
    // The dev flow: Vite serves the client, tam-server serves the API, and
    // the proxy keeps them same-origin so the HttpOnly cookie and the
    // EventSource behave exactly as they will behind ServeDir.
    proxy: {
      '/v1': plainHttp,
      '/healthz': 'http://127.0.0.1:8080',
      '/api/auth': 'http://127.0.0.1:8081'
    }
  },
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'node'
  }
}));
