import { describe, expect, it } from 'vitest';
import type { DeviceView } from '$lib/api';
import {
	CONNECTED_WITHIN_MS,
	checkInControl,
	checkInNote,
	inListOrder,
	loginWords,
	machineWords,
	sourcedFileWords
} from './machines';

const NOW = 1_770_000_000_000;
const MINUTE = 60_000;

function machine(over: Partial<DeviceView> = {}): DeviceView {
	return {
		id: 'dev_1',
		name: 'SM-N975F',
		os: 'android',
		arch: 'arm64',
		app_version: '0.9.0',
		runs_sourced_payloads: true,
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
		expect(said).toBe('You signed it out 3 h ago');
		expect(said).not.toContain('Connected');
		expect(said).not.toContain('just now');
	});

	it('states the check-in and never asserts more than contact', () => {
		const fresh = machineWords(machine({ last_seen_at: NOW - MINUTE }), NOW);
		expect(fresh).toBe('Checked in 1 min ago');
		// "Connected" was the word this replaces, and it asserted a live
		// connection we do not hold: the device checks in and goes quiet again,
		// so the honest claim is when it last spoke.
		expect(fresh).not.toContain('Connected');
		expect(machineWords(machine({ last_seen_at: NOW - CONNECTED_WITHIN_MS }), NOW)).toContain(
			'Checked in'
		);
	});

	// Past the window it states the age and claims nothing about now. A laptop
	// that shut its lid is not connected, and the list saying it is turns the
	// one honest signal on this page into noise.
	it('states the age alone once the window has passed', () => {
		const said = machineWords(machine({ last_seen_at: NOW - CONNECTED_WITHIN_MS - 1 }), NOW);
		expect(said).toBe('Last seen 10 min ago');
		expect(said).not.toContain('Checked in');
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
		).toBe('You signed it out 2 days ago');
	});
});

describe('the facts about a machine, kept apart', () => {
	// The founder's complaint, as three sentences that must not merge. A saved
	// login is a credential on a machine; it is not evidence the machine is
	// there, nor that the credential still works.
	it('never lets a saved login speak about presence', () => {
		const stale = machine({
			last_seen_at: NOW - 3 * 60 * MINUTE,
			sessions: [
				{
					marketplace: 'Tpt',
					account_label: 'Miss Cooper',
					linked_at: NOW - 10 * 24 * 60 * MINUTE,
					last_used_at: NOW - 10 * 24 * 60 * MINUTE,
					status: 'connected'
				}
			]
		});
		expect(machineWords(stale, NOW)).toBe('Last seen 3 h ago');
		const login = loginWords(stale);
		expect(login).toContain('1 marketplace login saved');
		expect(login).toContain('may no longer work');
		expect(login).not.toContain('Checked in');
		expect(login).not.toContain('Ready');
	});

	it('says plainly when no login is saved', () => {
		expect(loginWords(machine())).toBe('No marketplace login saved on this machine.');
	});

	// Eligibility is narrow and its wording must be too. The server's version
	// floor applies only to marketplace-sourced payloads; the import claim
	// itself never looks at a version. So an older installation is told what
	// it cannot do and nothing more, and one at the floor is told nothing.
	it('names only the sourced-file limitation and never calls a machine unable to import', () => {
		expect(sourcedFileWords(machine({ runs_sourced_payloads: true }))).toBeNull();
		const older = sourcedFileWords(machine({ app_version: '0.8.0', runs_sourced_payloads: false }));
		expect(older).toContain('publish or download files from a marketplace');
		expect(older).toContain('Importing your catalogue');
		// The sentence the server does not support, in either direction.
		expect(older).not.toContain('cannot run imports');
		expect(older).not.toContain('too old to run imports');
	});
});

describe('the order the list puts machines in', () => {
	function row(over: Partial<DeviceView>) {
		return { device: machine(over) };
	}

	it('puts this installation first and never matches on the display name', () => {
		const rows = [
			row({ id: 'other-fresh', name: 'SM-N975F', last_seen_at: NOW - MINUTE }),
			row({ id: 'here', name: 'SM-N975F', last_seen_at: NOW - 60 * MINUTE })
		];
		const { current } = inListOrder(rows, 'here');
		expect(current.map((one) => one.device.id)).toEqual(['here', 'other-fresh']);
		// Two machines of one name are two machines: the one the seller is at
		// is decided by installation id, and nothing about the name moves it.
		const { current: byName } = inListOrder(rows, 'absent-id');
		expect(byName.map((one) => one.device.id)).toEqual(['other-fresh', 'here']);
	});

	it('keeps signed-out records in history rather than in the list or the bin', () => {
		const rows = [
			row({ id: 'live' }),
			row({ id: 'old-1', revoked_at: NOW - 2 * 24 * 60 * MINUTE }),
			row({ id: 'old-2', revoked_at: NOW - 60 * MINUTE })
		];
		const { current, history } = inListOrder(rows, null);
		expect(current.map((one) => one.device.id)).toEqual(['live']);
		// Newest sign-out first, and both kept: a sign-out is the seller's own
		// decision and the record is the evidence of it.
		expect(history.map((one) => one.device.id)).toEqual(['old-2', 'old-1']);
	});

	it('does not hide an older installation for being older', () => {
		const { current } = inListOrder(
			[row({ id: 'old-app', app_version: '0.8.0', runs_sourced_payloads: false })],
			null
		);
		expect(current.map((one) => one.device.id)).toEqual(['old-app']);
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
			expect(said).toBe(`This machine could not reach Teachouse: ${detail}.`);
		}
	});

	it('does not add a second full stop to a sentence that has one', () => {
		expect(checkInNote({ reached: false, detail: 'the control plane refused: no route.' })).toBe(
			'This machine could not reach Teachouse: the control plane refused: no route.'
		);
	});

	it('names no cause when the application reported none', () => {
		// An application older than the field, and a rejection, both land here. A
		// substituted cause would read as something the machine actually said.
		const said = checkInNote({ reached: false, detail: null });
		expect(said).toBe('This machine could not reach Teachouse, and did not say why.');
		expect(said).not.toContain(':');
	});
});

describe('the check-in control', () => {
	it('reads as the act while it is idle and as the running act while it is not', () => {
		expect(checkInControl(false).label).toBe('Refresh this machine');
		expect(checkInControl(true).label).toBe('Refreshing…');
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
