import { describe, expect, it } from 'vitest';
import { checkInControl, checkInNote } from './machines';

describe('what the panel says about a check-in it asked for', () => {
	it('says nothing at all when the machine reached us', () => {
		expect(checkInNote({ reached: true, detail: null })).toBeNull();
		expect(checkInNote({ reached: true, detail: 'ignored' })).toBeNull();
	});

	it("carries the application's own four sentences unaltered", () => {
		// The exact strings `ControlPlaneError`'s Display writes, which are what
		// the founder reads off a phone with no cable attached. Copied rather
		// than imported because there is no shared vocabulary between the two:
		// this asserts that the console renders whatever it is handed, not that
		// it knows the set.
		const sentences = [
			'this build has no control-plane transport',
			'the control plane refused: error sending request',
			'this device is not registered',
			'nobody is signed in to the console on this device'
		];
		for (const detail of sentences) {
			const said = checkInNote({ reached: false, detail });
			expect(said).toBe(`This machine could not tell us it is here: ${detail}.`);
		}
	});

	it('does not add a second full stop to a sentence that has one', () => {
		expect(checkInNote({ reached: false, detail: 'the control plane refused: no route.' })).toBe(
			'This machine could not tell us it is here: the control plane refused: no route.'
		);
	});

	it('names no cause when the application reported none', () => {
		// An application older than the field, and a rejection, both land here. A
		// substituted cause would read as something the machine actually said.
		const said = checkInNote({ reached: false, detail: null });
		expect(said).toBe('This machine could not tell us it is here, and did not say why.');
		expect(said).not.toContain(':');
	});
});

describe('the check-in control', () => {
	it('reads as the act while it is idle and as the running act while it is not', () => {
		expect(checkInControl(false).label).toBe('Check in now');
		expect(checkInControl(true).label).toBe('Checking in…');
	});

	it('is pressable only when nothing is running', () => {
		expect(checkInControl(false).disabled).toBe(false);
		expect(checkInControl(true).disabled).toBe(true);
	});

	it('states a reason whenever it is disabled, and none when it is not', () => {
		// `Button` shows `reason` as the control's title, and a disabled control
		// with nothing to say reads as a fault rather than as one act at a time.
		expect(checkInControl(true).reason).toBeTypeOf('string');
		expect(checkInControl(false).reason).toBeUndefined();
	});
});
