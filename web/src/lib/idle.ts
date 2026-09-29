// When work that nobody is waiting for may start: after the page has loaded
// and the main thread has a moment, or after the visitor has done something.
//
// Pure over the host they are handed, so they test against a stub window
// rather than a browser. `requestIdleCallback` is optional on the host because
// WebKit has none, and the desktop app's Linux webview is WebKit: there the
// idle wait falls back to the next task after `load`, which is still after
// everything the page asked for up front.

/** The part of `window` the two waits read. */
export interface IdleHost {
	document: { readyState: DocumentReadyState };
	addEventListener(type: string, listener: () => void, options: AddEventListenerOptions): void;
	removeEventListener(type: string, listener: () => void, options: EventListenerOptions): void;
	requestIdleCallback?: (callback: () => void, options: { timeout: number }) => number;
	setTimeout(callback: () => void, ms: number): unknown;
}

/** How long an idle wait may be put off by a busy main thread before it runs
 *  anyway. Long enough to clear a console page's first render on a slow phone,
 *  short enough that a session is not missing its first seconds. */
export const IDLE_TIMEOUT_MS = 4000;

/** Resolves once `load` has fired and the main thread has then been idle, or
 *  after `IDLE_TIMEOUT_MS` of waiting for idle. */
export function afterLoadIdle(host: IdleHost): Promise<void> {
	// The executor form rather than `Promise.withResolvers`, which the oldest
	// Android WebView the console supports lacks (see `app.html`).
	return new Promise<void>((resolve) => {
		const idle = () => {
			if (host.requestIdleCallback) {
				host.requestIdleCallback(() => resolve(), { timeout: IDLE_TIMEOUT_MS });
			} else {
				host.setTimeout(resolve, 0);
			}
		};
		if (host.document.readyState === 'complete') {
			idle();
		} else {
			host.addEventListener('load', idle, { once: true });
		}
	});
}

/** The events that say a person is here and doing something. Scrolling and
 *  pointer movement are not among them: a page that is merely looked at stays
 *  a page nothing extra was fetched for. */
export const INTERACTION_EVENTS: readonly string[] = ['pointerdown', 'keydown', 'focusin'];

/** Runs `then` once, on the first interaction. Returns the cancel, which
 *  removes every listener so a page left without one being made leaves
 *  nothing behind. */
export function onFirstInteraction(host: IdleHost, then: () => void): () => void {
	const options = { capture: true, passive: true } as const;
	const cancel = () => {
		for (const type of INTERACTION_EVENTS) {
			host.removeEventListener(type, fire, options);
		}
	};
	function fire() {
		cancel();
		then();
	}
	for (const type of INTERACTION_EVENTS) {
		host.addEventListener(type, fire, options);
	}
	return cancel;
}
