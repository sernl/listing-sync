// The toast stack's lifetime under fake timers: five seconds a toast, three
// drawn at once with the rest waiting their turn, a held stack keeping what is
// left of each toast's time, and only the toasts nobody looked at reaching the
// inbox.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
	MAX_VISIBLE,
	TOAST_TTL_MS,
	type Toast,
	acknowledge,
	dismiss,
	noticeOf,
	pause,
	resetToasts,
	resume,
	setUnreadSink,
	toast,
	toastStore
} from './toast';

function drawn(): Toast[] {
	let seen: Toast[] = [];
	toastStore.subscribe((entries) => (seen = entries))();
	return seen;
}

const messages = () => drawn().map((entry) => entry.message);

let kept: Toast[] = [];

beforeEach(() => {
	vi.useFakeTimers();
	resetToasts();
	kept = [];
	setUnreadSink((entry) => kept.push(entry));
});

afterEach(() => {
	resetToasts();
	vi.useRealTimers();
});

describe('how long a toast stands', () => {
	it('stands to the millisecond before five seconds, of every tone', () => {
		toast('success', 'Saved.');
		toast('error', 'That was not saved.');
		vi.advanceTimersByTime(TOAST_TTL_MS - 1);
		expect(messages()).toEqual(['Saved.', 'That was not saved.']);
		vi.advanceTimersByTime(1);
		expect(messages()).toEqual([]);
	});

	it('keeps what was left of its time while the stack is held', () => {
		toast('info', 'Sending started.');
		vi.advanceTimersByTime(3000);
		pause();
		vi.advanceTimersByTime(60_000);
		expect(messages()).toEqual(['Sending started.']);
		resume();
		vi.advanceTimersByTime(TOAST_TTL_MS - 3000 - 1);
		expect(messages()).toEqual(['Sending started.']);
		vi.advanceTimersByTime(1);
		expect(messages()).toEqual([]);
	});
});

describe('how many stand at once', () => {
	it('draws three and starts the fourth one’s time only when it is drawn', () => {
		for (const word of ['one', 'two', 'three', 'four']) {
			toast('success', word);
		}
		expect(drawn()).toHaveLength(MAX_VISIBLE);
		expect(messages()).toEqual(['one', 'two', 'three']);

		vi.advanceTimersByTime(TOAST_TTL_MS);
		expect(messages()).toEqual(['four']);
		vi.advanceTimersByTime(TOAST_TTL_MS - 1);
		expect(messages()).toEqual(['four']);
		vi.advanceTimersByTime(1);
		expect(messages()).toEqual([]);
	});

	it('draws the waiting one as soon as a drawn one is closed', () => {
		const ids = ['one', 'two', 'three', 'four'].map((word) => toast('info', word));
		dismiss(ids[1]);
		expect(messages()).toEqual(['one', 'three', 'four']);
	});
});

describe('what the inbox keeps', () => {
	it('keeps a toast that went on its own, and not one that was closed or clicked', () => {
		toast('warning', 'Nobody saw this.');
		const closed = toast('error', 'Closed.');
		const clicked = toast('success', 'Clicked.');
		dismiss(closed);
		acknowledge(clicked);
		vi.advanceTimersByTime(TOAST_TTL_MS);
		expect(kept.map((entry) => [entry.tone, entry.message])).toEqual([
			['warning', 'Nobody saw this.']
		]);
	});

	it('closes an id that has already gone without a word', () => {
		const id = toast('info', 'Gone.');
		vi.advanceTimersByTime(TOAST_TTL_MS);
		let calls = 0;
		const stop = toastStore.subscribe(() => (calls += 1));
		dismiss(id);
		stop();
		expect(calls).toBe(1);
	});

	it('titles a notice with the message, cutting a long one and keeping it whole as the body', () => {
		const short = noticeOf({ ...drawnOne('info', 'Saved.') });
		expect(short).toMatchObject({ tone: 'info', title: 'Saved.', body: '' });

		const long = 'a'.repeat(250);
		const cut = noticeOf(drawnOne('error', long));
		expect([...cut.title]).toHaveLength(200);
		expect(cut.title.endsWith('…')).toBe(true);
		expect(cut.body).toBe(long);
	});

	it('gives each toast its own client id', () => {
		toast('info', 'one');
		toast('info', 'two');
		const [first, second] = drawn();
		expect(first.clientId).not.toBe(second.clientId);
	});
});

function drawnOne(tone: Toast['tone'], message: string): Toast {
	toast(tone, message);
	const last = drawn().at(-1);
	if (last === undefined) {
		throw new Error('the toast was not drawn');
	}
	return last;
}
