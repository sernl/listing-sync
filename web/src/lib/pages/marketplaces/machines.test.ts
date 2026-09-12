import { describe, expect, it } from 'vitest';
import type { DeviceView } from '$lib/api';
import { CONNECTED_WITHIN_MS, checkInControl, checkInNote, machineWords } from './machines';

const NOW = 1_770_000_000_000;
const MINUTE = 60_000;

function machine(over: Partial<DeviceView> = {}): DeviceView {
	return {
		id: 'dev_1',
		name: 'SM-N975F',
		os: 'android',
		arch: 'arm64',
		app_version: '0.7.0',
		first_seen_at: NOW - 40 * 24 * 60 * MINUTE,
		last_seen_at: NOW - MINUTE,
		revoked_at: null,
		wipe_outstanding: false,
		sessions: [],
		...over
	};
}

describe('what one machine is doing', () => {
	// The defect, stated as an ordering. The heartbeat stamps `last_seen_at`
	// on a revoked device — that check-in is how we learn it has wiped its
	// logins — so a machine signed out from the console goes on looking as
	// fresh as any other. The founder read "available just now" beside a
	// machine he had signed out that morning and concluded the app was
	// broken.
	it('says a machine was signed out before it says anything about freshness', () => {
		const said = machineWords(
			machine({ revoked_at: NOW - 3 * 60 * MINUTE, last_seen_at: NOW - MINUTE }),
			NOW
		);
		expect(said).toBe('Signed out from the console 3 h ago');
		expect(said).not.toContain('Connected');
		expect(said).not.toContain('just now');
	});

	it('calls a machine connected while its last check-in is inside the window', () => {
		expect(machineWords(machine({ last_seen_at: NOW - MINUTE }), NOW)).toBe(
			'Connected · checked in 1 min ago'
		);
		expect(machineWords(machine({ last_seen_at: NOW - CONNECTED_WITHIN_MS }), NOW)).toContain(
			'Connected'
		);
	});

	// Past the window it states the age and claims nothing about now. A laptop
	// that shut its lid is not connected, and the list saying it is turns the
	// one honest signal on this page into noise.
	it('states the age alone once the window has passed', () => {
		const said = machineWords(machine({ last_seen_at: NOW - CONNECTED_WITHIN_MS - 1 }), NOW);
		expect(said).toBe('Last seen 15 min ago');
		expect(said).not.toContain('Connected');
	});

	// A machine that is both revoked and stale still leads with the
	// revocation: "last seen 3 days ago" is true and answers a question the
	// seller did not ask.
	it('leads with the revocation even where the machine is also quiet', () => {
		expect(
			machineWords(
				machine({
					revoked_at: NOW - 2 * 24 * 60 * MINUTE,
					last_seen_at: NOW - 2 * 24 * 60 * MINUTE
				}),
				NOW
			)
		).toBe('Signed out from the console 2 days ago');
	});
});

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
