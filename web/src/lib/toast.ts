// A small toast store: push a message, it expires. No dependency earns its
// place for this.

export interface Toast {
	id: number;
	tone: 'info' | 'error';
	message: string;
}

type Subscriber = (toasts: Toast[]) => void;

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

export function toast(tone: Toast['tone'], message: string, ttlMs = 6000) {
	const id = next;
	next += 1;
	toasts = [...toasts, { id, tone, message }];
	notify();
	setTimeout(() => {
		toasts = toasts.filter((entry) => entry.id !== id);
		notify();
	}, ttlMs);
}
