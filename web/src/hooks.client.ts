import type { HandleClientError } from '@sveltejs/kit';
import { isLostChunk, LOST_CHUNK_SAID, mayReloadForLostChunk } from '$lib/lost-chunk';
import { renderFailureCause, renderFailureReport } from '$lib/render-failure';

// A chunk the router could not fetch is reloaded once (`$lib/lost-chunk`).
// Vite raises this for its own preload helper, which wraps every `import()`
// in the build, so a viewer or maker chunk that fails outside a navigation
// is caught here too; a navigation's failure also reaches `handleError`.
window.addEventListener('vite:preloadError', (event) => {
	if (mayReloadForLostChunk(sessionStorage)) {
		event.preventDefault();
		location.reload();
	}
});

/** What the error page is given. SvelteKit's default hands it "Internal
 *  Error", which is what the founder's phone showed under the sentence: no
 *  cause at all. This hands it the words the failure carried, and for a lost
 *  chunk either reloads or, when a reload was just tried, says what to do. */
export const handleError: HandleClientError = ({ error, event }) => {
	console.error(renderFailureReport(error, event.url.pathname));
	if (isLostChunk(error)) {
		if (mayReloadForLostChunk(sessionStorage)) {
			location.reload();
		}
		return { message: LOST_CHUNK_SAID };
	}
	return { message: renderFailureCause(error) };
};
