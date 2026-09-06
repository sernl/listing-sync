// The toast stack has no rendering lane, so what is asserted here is the part
// that was a `setTimeout` closure before and could not be reached at all: when
// a toast goes, and what happens when it is closed twice.

import { afterEach, describe, expect, it } from 'vitest';
import { INFO_TTL_MS, type Toast, dismiss, expiresAt, sweep, toast, toastStore } from './toast';

/** The stack is one module-level list shared by every test in this file, so a
 *  test reads it through the same subscription a component would and clears it
 *  afterwards rather than assuming it began empty. */
function standing(): Toast[] {
	let seen: Toast[] = [];
	toastStore.subscribe((entries) => (seen = entries))();
	return seen;
}

const ids = () => standing().map((entry) => entry.id);

afterEach(() => {
	for (const entry of standing()) {
		dismiss(entry.id);
	}
});

describe('when a toast goes', () => {
	it('gives an informational toast six seconds and an error none', () => {
		expect(expiresAt('info', 1000)).toBe(1000 + INFO_TTL_MS);
		expect(expiresAt('error', 1000)).toBeNull();
	});

	it('keeps an informational toast to the millisecond before its moment', () => {
		const id = toast('info', 'Organisation name saved.', 1000);
		sweep(1000 + INFO_TTL_MS - 1);
		expect(ids()).toContain(id);
	});

	it('drops an informational toast at its moment', () => {
		const id = toast('info', 'Organisation name saved.', 1000);
		sweep(1000 + INFO_TTL_MS);
		expect(ids()).not.toContain(id);
	});

	it('never sweeps an error toast, however late the clock', () => {
		const id = toast('error', 'The organisation name was not saved.', 1000);
		sweep(Number.MAX_SAFE_INTEGER);
		expect(ids()).toContain(id);
	});
});

describe('closing a toast', () => {
	it('removes the one named and leaves its neighbours standing', () => {
		const first = toast('error', 'First.', 0);
		const second = toast('error', 'Second.', 0);
		const third = toast('error', 'Third.', 0);
		dismiss(second);
		expect(ids()).toEqual([first, third]);
	});

	it('changes nothing when the id has already been swept', () => {
		const id = toast('info', 'Passkey registered.', 0);
		sweep(INFO_TTL_MS);
		expect(() => dismiss(id)).not.toThrow();
		expect(ids()).not.toContain(id);
	});
});

describe('the subscriber', () => {
	it('hears each transition once, and hears nothing when nothing moved', () => {
		const heard: number[] = [];
		const stop = toastStore.subscribe((entries) => heard.push(entries.length));
		expect(heard).toEqual([0]);

		const id = toast('info', 'Password changed. Sign in with the new one.', 0);
		expect(heard).toEqual([0, 1]);

		sweep(INFO_TTL_MS - 1);
		expect(heard).toEqual([0, 1]);

		sweep(INFO_TTL_MS);
		expect(heard).toEqual([0, 1, 0]);

		dismiss(id);
		expect(heard).toEqual([0, 1, 0]);

		stop();
	});
});
