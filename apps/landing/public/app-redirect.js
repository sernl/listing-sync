// The desktop app and the browser reach the same origin, and the desktop app
// has no use for the marketing page. Tauri v2 injects `window.__TAURI__` into
// every window it opens, which is the only signal available before the console
// itself loads.
const path = window.location.pathname;
if (window.__TAURI__ && path !== '/app' && !path.startsWith('/app/')) {
	window.location.replace('/app');
}
