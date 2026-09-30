// The toast stack: push a message, it stands for five seconds and goes, or
// it is closed first. No dependency earns its place for this.
//
// Every toast has one of four tones -- success, info, warning, error -- which
// is its colour and its glyph, and which is also the tone it keeps in the
// inbox (`NoticeTone` in Rust). Every tone expires: a toast nobody looked at
// is not lost, because it is handed to the unread sink as it leaves, and the
// console's sink posts it to `/v1/notifications` so the bell can say what the
// seller missed. A toast the seller closed or clicked is one they saw, and it
// goes without a trace.
//
// At most three stand at once. A fourth waits its turn, and its five seconds
// start when it is drawn rather than when it was pushed, so a burst of saves
// does not burn the later ones' time off-screen.
//
// The clock lives here: one `setTimeout` armed for the nearest deadline among
// the drawn toasts, so nothing runs while the stack is empty and a test drives
// the whole lifetime with fake timers. What pauses it -- the pointer or focus
// inside the stack -- is the element's to know, so `pause` and `resume` are
// the component's to call; a paused toast keeps what was left of its time.

import type { NoticeTone } from '$lib/generated/vocab';

export type ToastTone = NoticeTone;

export interface Toast {
	id: number;
	/** The id the inbox dedupes this toast's notice on. */
	clientId: string;
	tone: ToastTone;
	message: string;
	/** When it goes on its own, or null while it waits its turn or the stack
	 *  is held. */
	deadline: number | null;
	/** What is left of its time, which is what a held stack keeps. */
	remaining: number;
	/** Whether the seller clicked it, which makes it seen. */
	seen: boolean;
}

/** A toast as the inbox keeps it: the body of `POST /v1/notifications`. */
export interface ToastNotice {
	client_id: string;
	tone: ToastTone;
	title: string;
	body: string;
}

/** How long a toast stands once it is drawn. */
export const TOAST_TTL_MS = 5000;

/** How many are drawn at once; the rest wait. */
export const MAX_VISIBLE = 3;

type Subscriber = (toasts: Toast[]) => void;
type UnreadSink = (toast: Toast) => void;

let next = 1;
let queue: Toast[] = [];
let held = false;
/** Cancels the one armed timer; null when none is armed. */
let cancelTimer: (() => void) | null = null;
let sink: UnreadSink | null = null;
const subscribers = new Set<Subscriber>();

function visible(): Toast[] {
	return queue.slice(0, MAX_VISIBLE);
}

function clientId(id: number): string {
	const random =
		typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
			? crypto.randomUUID()
			: Math.random().toString(36).slice(2);
	return `${random}-${id}`;
}

/** Starts the clock of every drawn toast that has none, unless the stack is
 *  held, and arms the one timer for the nearest deadline. */
function arm(now: number) {
	cancelTimer?.();
	cancelTimer = null;
	if (held) {
		return;
	}
	let nearest: number | null = null;
	queue = queue.map((entry, index) => {
		if (index >= MAX_VISIBLE || entry.deadline !== null) {
			return entry;
		}
		return { ...entry, deadline: now + entry.remaining };
	});
	for (const entry of visible()) {
		if (entry.deadline !== null && (nearest === null || entry.deadline < nearest)) {
			nearest = entry.deadline;
		}
	}
	if (nearest !== null) {
		const handle = setTimeout(() => expire(Date.now()), Math.max(0, nearest - now));
		cancelTimer = () => clearTimeout(handle);
	}
}

function notify() {
	const shown = visible();
	for (const run of subscribers) {
		run(shown);
	}
}

/** Drops every drawn toast whose moment has come, hands the unseen ones to
 *  the sink, and draws whatever was waiting. */
function expire(now: number) {
	cancelTimer = null;
	const gone = visible().filter((entry) => entry.deadline !== null && entry.deadline <= now);
	if (gone.length > 0) {
		const ids = new Set(gone.map((entry) => entry.id));
		queue = queue.filter((entry) => !ids.has(entry.id));
		for (const entry of gone) {
			if (!entry.seen) {
				sink?.(entry);
			}
		}
	}
	arm(now);
	if (gone.length > 0) {
		notify();
	}
}

/** The drawn toasts: never more than `MAX_VISIBLE`. */
export const toastStore = {
	subscribe(run: Subscriber) {
		run(visible());
		subscribers.add(run);
		return () => subscribers.delete(run);
	}
};

export function toast(tone: ToastTone, message: string): number {
	const id = next;
	next += 1;
	queue = [
		...queue,
		{
			id,
			clientId: clientId(id),
			tone,
			message,
			deadline: null,
			remaining: TOAST_TTL_MS,
			seen: false
		}
	];
	arm(Date.now());
	notify();
	return id;
}

/** The seller closed it: it goes now, and it was seen, so it is not kept.
 *  Closing an id that has already gone changes nothing and tells no one, so
 *  a close arriving after it expired is not an error. */
export function dismiss(id: number): void {
	const left = queue.filter((entry) => entry.id !== id);
	if (left.length === queue.length) {
		return;
	}
	queue = left;
	arm(Date.now());
	notify();
}

/** The seller clicked it: it stands its time out, and is not kept after. */
export function acknowledge(id: number): void {
	queue = queue.map((entry) => (entry.id === id ? { ...entry, seen: true } : entry));
}

/** Holds every drawn toast where it is: the pointer or focus is in the stack,
 *  and a toast removed from under it would steal the click about to close
 *  it. Each keeps what was left of its time. */
export function pause(now = Date.now()): void {
	if (held) {
		return;
	}
	held = true;
	cancelTimer?.();
	cancelTimer = null;
	queue = queue.map((entry) =>
		entry.deadline === null
			? entry
			: { ...entry, deadline: null, remaining: Math.max(0, entry.deadline - now) }
	);
}

/** Lets them go again, each with the time it had left. */
export function resume(now = Date.now()): void {
	if (!held) {
		return;
	}
	held = false;
	arm(now);
}

/** Where a toast nobody looked at goes as it leaves; null for nowhere. The
 *  console sets it, and a signed-out screen has none, because there is no
 *  inbox to keep anything in. */
export function setUnreadSink(next: UnreadSink | null): void {
	sink = next;
}

/** Every toast gone at once, the stack released, and no sink: what a test
 *  starts from. Nothing is handed to the sink. */
export function resetToasts(): void {
	cancelTimer?.();
	cancelTimer = null;
	queue = [];
	held = false;
	sink = null;
	notify();
}

/** What the inbox keeps of a toast: its message as the title, cut to the
 *  inbox's 200 characters, with the whole of a longer one as the body. */
export function noticeOf(entry: Toast): ToastNotice {
	const message = entry.message.trim();
	const chars = [...message];
	if (chars.length <= 200) {
		return { client_id: entry.clientId, tone: entry.tone, title: message, body: '' };
	}
	return {
		client_id: entry.clientId,
		tone: entry.tone,
		title: `${chars.slice(0, 199).join('')}…`,
		body: chars.slice(0, 1000).join('')
	};
}
