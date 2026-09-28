// A press outside a modal dialog closes it; a press inside, a drag that only
// ends outside, and a dialog refusing to close while it saves do not.
//
// Driven through a stand-in dialog built on Node's own EventTarget: the tests
// run without a DOM, and the action touches nothing of the element beyond its
// events, `open` and `close()`.

import { describe, expect, it } from 'vitest';
import { lightDismiss } from './dismiss';

class StandInDialog extends EventTarget {
	open = true;
	close() {
		this.open = false;
	}
}

function mounted() {
	const dialog = new StandInDialog();
	const child = new EventTarget();
	lightDismiss(dialog as unknown as HTMLDialogElement);
	/** Dispatch an event whose `target` reads as `at`, as a press on a child
	 *  bubbling up to the dialog would. */
	const on = (at: EventTarget, type: string) => {
		const event = new Event(type, { cancelable: true });
		Object.defineProperty(event, 'target', { value: at });
		dialog.dispatchEvent(event);
	};
	const press = (down: EventTarget, up: EventTarget) => {
		on(down, 'pointerdown');
		on(up, 'click');
	};
	return { dialog, child, press };
}

describe('closing a modal dialog on a press outside it', () => {
	it('closes on a press on the backdrop', () => {
		const { dialog, press } = mounted();
		press(dialog, dialog);
		expect(dialog.open).toBe(false);
	});

	it('stays open on a press inside', () => {
		const { dialog, child, press } = mounted();
		press(child, child);
		expect(dialog.open).toBe(true);
	});

	it('stays open when a drag starts inside and ends on the backdrop', () => {
		const { dialog, child, press } = mounted();
		press(child, dialog);
		expect(dialog.open).toBe(true);
	});

	it('stays open when a keyboard click lands on the dialog with no press before it', () => {
		const { dialog, child, press } = mounted();
		press(child, child);
		const event = new Event('click');
		dialog.dispatchEvent(event);
		expect(dialog.open).toBe(true);
	});

	it('stays open when its cancel handler refuses, as it does for Escape while saving', () => {
		const { dialog, press } = mounted();
		dialog.addEventListener('cancel', (event) => event.preventDefault());
		press(dialog, dialog);
		expect(dialog.open).toBe(true);
	});

	it('raises cancel before closing, so a dialog that tidies up on Escape tidies up here', () => {
		const { dialog, press } = mounted();
		let cancelled = 0;
		dialog.addEventListener('cancel', () => {
			cancelled += 1;
		});
		press(dialog, dialog);
		expect(cancelled).toBe(1);
	});
});
