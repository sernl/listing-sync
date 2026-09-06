// A small toast store: push a message, it expires or it is closed. No
// dependency earns its place for this.
//
// When a toast goes is a pure function of its tone and the moment it was
// pushed, rather than a `setTimeout` closure per toast, for two reasons. An
// error toast has no expiry at all — the one case where a sentence vanishing
// on its own loses the seller the only account of what did not happen — and a
// rule stated as a timer that is never armed is a rule nothing can test. The
// timer is the caller's: the layout owns it and holds it while the pointer is
// inside the stack, because a sweep that removes the element under the pointer
// steals the click that was about to close it.
//
// `dismiss` and `sweep` are free functions beside `toast` rather than methods
// on `toastStore`, because that object exists only to satisfy the store
// contract Svelte subscribes through, and every verb that mutates this state
// already sits out here.

export interface Toast {
	id: number;
	tone: 'info' | 'error';
	message: string;
	/** When this toast goes on its own, or null when only closing it does. */
	expires: number | null;
}

type Subscriber = (toasts: Toast[]) => void;

/** How long an informational toast stands. An error has no equivalent: it
 *  stands until it is closed. */
export const INFO_TTL_MS = 6000;

let next = 1;
let toasts: Toast[] = [];
const subscribers = new Set<Subscriber>();

function notify() {
	for (const run of subscribers) {
		run(toasts);
	}
}

export const toastStore = {
	subscribe(run: Subscriber) {
		run(toasts);
		subscribers.add(run);
		return () => subscribers.delete(run);
	}
};

/** When a toast of this tone, pushed at this moment, stops standing on its
 *  own; null when nothing but its close control removes it.
 *
 * A total switch rather than a lookup with a default, so a third tone stops
 * the type check instead of quietly inheriting the informational lifetime. */
export function expiresAt(tone: Toast['tone'], pushedAt: number): number | null {
	switch (tone) {
		case 'info':
			return pushedAt + INFO_TTL_MS;
		case 'error':
			return null;
	}
}

export function toast(tone: Toast['tone'], message: string, pushedAt = Date.now()): number {
	const id = next;
	next += 1;
	toasts = [...toasts, { id, tone, message, expires: expiresAt(tone, pushedAt) }];
	notify();
	return id;
}

/** Remove one toast by id. Dismissing an id that has already gone changes
 *  nothing and tells no one, so a close arriving after a sweep is not an
 *  error. */
export function dismiss(id: number): void {
	const left = toasts.filter((entry) => entry.id !== id);
	if (left.length === toasts.length) {
		return;
	}
	toasts = left;
	notify();
}

/** Drop every toast whose moment has come. Silent when none has, so a timer
 *  running four times a second does not redraw the stack four times a
 *  second. */
export function sweep(now: number): void {
	const left = toasts.filter((entry) => entry.expires === null || now < entry.expires);
	if (left.length === toasts.length) {
		return;
	}
	toasts = left;
	notify();
}
