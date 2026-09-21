import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import type { ProxyOptions } from 'vite';
import { defineConfig } from 'vitest/config';

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

export default defineConfig({
  // `PUBLIC_` beside Vite's own prefix, so a build-time public value follows
  // SvelteKit's naming for one rather than Vite's. Nothing without one of
  // the two prefixes reaches the bundle.
  envPrefix: ['VITE_', 'PUBLIC_'],
  plugins: [tailwindcss(), sveltekit()],
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
});
