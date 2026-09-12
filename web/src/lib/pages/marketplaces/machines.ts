// How one machine and the marketplace logins on it are worded. Pure, so it
// tests without a component.

import type { DeviceSessionView, DeviceView } from '$lib/api';
import type { CheckInHere } from '$lib/desktop';
import { agoLabel } from '$lib/elapsed';
import type { Tone } from '$lib/StatusPill.svelte';
import type { DeviceSessionStatus } from '$lib/generated/vocab';

/** The operating system as its own vendor writes it.
 *
 *  The device reports a lowercase identifier and this is the only place that
 *  turns it into a name, so an unknown one renders as the device sent it
 *  rather than as a blank. */
const OS_NAME: Record<string, string> = {
	windows: 'Windows',
	macos: 'macOS',
	linux: 'Linux',
	android: 'Android',
	ios: 'iOS'
};

export function osLabel(device: DeviceView): string {
	return OS_NAME[device.os.toLowerCase()] ?? device.os;
}

/** A total map over the generated union, so a status added in Rust takes a tone
 *  here rather than falling through to a default. */
const SESSION_TONE: Record<DeviceSessionStatus, Tone> = {
	connected: 'ok',
	signed_out: 'soon',
	wiped: 'bad'
};

export function sessionTone(status: DeviceSessionStatus): Tone {
	return SESSION_TONE[status];
}

/** The stored status as a word rather than as an identifier.
 *
 *  Not a second vocabulary: these are the generated values with their separator
 *  rendered for a person, because the status pill uppercases whatever it is
 *  given and `signed_out` arrives on screen as SIGNED_OUT, underscore and all.
 *  A total map over the generated union, so a status added in Rust is worded
 *  here rather than leaking its identifier. */
const SESSION_LABEL: Record<DeviceSessionStatus, string> = {
	connected: 'Connected',
	signed_out: 'Signed out',
	wiped: 'Wiped'
};

export function sessionLabel(status: DeviceSessionStatus): string {
	return SESSION_LABEL[status];
}

/**
 * What one held marketplace login is doing, in the seller's words.
 *
 * `wiped` is the one worth spelling out: the login is gone because the machine
 * was signed out and then reached us, which is the sign-out working rather
 * than anything failing.
 */
export function sessionWords(session: DeviceSessionView): string {
	switch (session.status) {
		case 'connected':
			return session.account_label
				? `signed in as ${session.account_label}`
				: 'signed in';
		case 'signed_out':
			return 'disconnected on the device';
		case 'wiped':
			return 'forgotten when this device was signed out';
	}
}

/** How recently a machine must have checked in for the list to call it
 *  connected.
 *
 *  Far shorter than `QUIET_AFTER_MS`, which is two hourly cadences and is the
 *  point at which silence means a machine is off. This is a different claim:
 *  "connected" beside a row is read as right now, and a machine last heard
 *  from fifty minutes ago is not something to say that about. Fifteen minutes
 *  is longer than any check-in the application makes on a resume, so a phone
 *  the seller has just picked up reads as connected and a laptop that shut its
 *  lid does not. */
export const CONNECTED_WITHIN_MS = 15 * 60 * 1000;

/**
 * What one machine is doing, in the order the seller needs to hear it.
 *
 * Revocation first, and the whole defect this precedence exists for is that
 * it was not. A machine signed out from the console keeps checking in — the
 * heartbeat stamps `last_seen_at` on a revoked device, which is how we learn
 * it has wiped its logins — so freshness is the one thing that stays true
 * about a machine that can no longer hold a marketplace login. The founder
 * read "available just now" beside a machine that had been signed out that
 * morning and concluded the app was broken; it was doing exactly what he had
 * told it to.
 *
 * The age of the revocation rather than the instant, because the act is the
 * seller's own and recent enough to recall.
 */
export function machineWords(device: DeviceView, now: number): string {
	if (device.revoked_at !== null) {
		return `Signed out from the console ${agoLabel(device.revoked_at, now)}`;
	}
	return now - device.last_seen_at <= CONNECTED_WITHIN_MS
		? `Connected · checked in ${agoLabel(device.last_seen_at, now)}`
		: `Last seen ${agoLabel(device.last_seen_at, now)}`;
}

/** What the panel says about a check-in that did not reach us, or null where
 *  there is nothing to say.
 *
 *  The sentence is the application's own, unaltered: `ControlPlaneError` writes
 *  four of them and each is already worded for a person — no transport in this
 *  build, the plane refused, this device is not registered, nobody is signed in
 *  here. They name no credential, no jar and no host but our own control plane.
 *
 *  Without this line a machine that could not register is indistinguishable
 *  from one that was never installed: both are an absence in a list. That is
 *  the difference between "nothing appeared" and a cause somebody can act on,
 *  and on a phone it is the only difference available — its log is private
 *  storage and its stdout needs a cable.
 *
 *  An application too old to carry the field says nothing rather than a
 *  substituted cause, because a named cause that was never reported would be
 *  worse than none. */
export function checkInNote(answer: Pick<CheckInHere, 'reached' | 'detail'>): string | null {
	if (answer.reached) {
		return null;
	}
	if (answer.detail === null) {
		return 'This machine could not tell us it is here, and did not say why.';
	}
	const stop = /[.!?]$/.test(answer.detail) ? '' : '.';
	return `This machine could not tell us it is here: ${answer.detail}${stop}`;
}

/** How the check-in control reads, pressed and unpressed.
 *
 *  `reason` is undefined unless the control is disabled, which is what `Button`
 *  requires: a disabled control with no stated reason reads as a fault rather
 *  than as one act at a time. */
export interface CheckInControl {
	label: string;
	reason: string | undefined;
	disabled: boolean;
}

export function checkInControl(pending: boolean): CheckInControl {
	return pending
		? { label: 'Checking in…', reason: 'The check-in is running.', disabled: true }
		: { label: 'Check in now', reason: undefined, disabled: false };
}
