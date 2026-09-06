// What makes a link out of the console land in the seller's own browser rather
// than inside the application window.
//
// An action rather than a markup change, because all three sites have to stay
// anchors: Shopify's brand condition requires its mark and name to link to its
// homepage, the web copy of the console still needs middle-click and copy-link,
// and one of them announces "opens in a new tab". Nothing about the markup, the
// `rel`, the `target` or the accessible name moves, so the tests that assert
// those keep meaning what they meant.

import { desktopInvoker, openExternal } from '$lib/desktop';

/** Send this anchor's address to the browser this computer already uses.
 *
 * Does nothing at all in a web browser, where there is no application to ask
 * and the anchor already behaves correctly. Inside the application it takes the
 * click, because the alternatives there are all wrong: WebKitGTK drops it,
 * WebView2 opens a chromeless popup inside the window, and the Android webview
 * replaces the console with the marketplace.
 *
 * A refusal falls back to `window.open`, which is what the anchor did before
 * this existed. The seller is told nothing, because a refusal here is a fault
 * in our own grant rather than anything they can act on. */
export function external(node: HTMLAnchorElement) {
	const invoke = desktopInvoker();
	if (invoke === null) {
		return;
	}
	const handler = (event: MouseEvent) => {
		// `target` is read here rather than on mount because one of the three
		// sites is an anchor whose action is external for a listed marketplace
		// and internal otherwise, and the same element carries both. An internal
		// route handed to the opener would open the console in a second browser:
		// its address is https, so the plugin's own scope would allow it.
		if (event.defaultPrevented || node.target !== '_blank') {
			return;
		}
		// Read before `preventDefault`, and off the property rather than the
		// attribute, so a relative address arrives resolved.
		const url = node.href;
		event.preventDefault();
		void openExternal(invoke, url).then((outcome) => {
			if (outcome.kind !== 'opened') {
				window.open(url, '_blank', 'noopener');
			}
		});
	};
	// `click` and not `auxclick`: a middle click raises the second and not the
	// first, so it keeps whatever the platform does with it.
	node.addEventListener('click', handler);
	return {
		destroy() {
			node.removeEventListener('click', handler);
		}
	};
}
