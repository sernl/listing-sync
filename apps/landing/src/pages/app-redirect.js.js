import { consoleUrl } from '../site.js';

/* The desktop app has no use for the marketing page. Tauri v2 injects
   `window.__TAURI__` into every window it opens, which is the only signal
   available before anything else loads, so a window that lands here is sent
   to the console's home on the console's own host. Every page loads this
   from the marketing origin, under `script-src 'self'`; it is an endpoint
   rather than a file in `public/` so that the console's address is the one
   `src/site.js` holds. */
export function GET() {
	const body = `if (window.__TAURI__) {\n\twindow.location.replace(${JSON.stringify(`${consoleUrl}/app`)});\n}\n`;
	return new Response(body, {
		headers: { 'Content-Type': 'text/javascript; charset=utf-8' }
	});
}
