// How one machine and the marketplace logins on it are worded. Pure, so it
// tests without a component.

import type { DeviceSessionView, DeviceView } from '$lib/api';
import type { CheckInHere } from '$lib/desktop';
import { CHECK_IN_CADENCE_MS } from '$lib/devices-view';
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
			return 'signed out in the app on that machine';
		case 'wiped':
			return 'removed when you signed this machine out';
	}
}

/** How recently a machine must have checked in for the list to call it
 *  connected.
 *
 *  Two check-in cadences, against `QUIET_AFTER_MS`'s six: "connected" beside
 *  a row is read as right now, so it is the stricter of the two claims. A
 *  machine that is running checks in every five minutes, so ten covers one
 *  missed check-in and no more — a phone the seller has just picked up reads
 *  as connected and a laptop that shut its lid does not. */
export const CONNECTED_WITHIN_MS = 2 * CHECK_IN_CADENCE_MS;

/**
 * When this machine was last heard from, and nothing else.
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
 *
 * Contact and nothing else: a saved marketplace login, an eligible app
 * version and a run in progress are three other facts, worded by
 * [`loginWords`], [`sourcedFileWords`] and the run's own view. Collapsing any
 * of them into this sentence is what made a saved login read as a machine
 * being there.
 */
export function machineWords(device: DeviceView, now: number): string {
	if (device.revoked_at !== null) {
		return `You signed it out ${agoLabel(device.revoked_at, now)}`;
	}
	return now - device.last_seen_at <= CONNECTED_WITHIN_MS
		? `Checked in ${agoLabel(device.last_seen_at, now)}`
		: `Last seen ${agoLabel(device.last_seen_at, now)}`;
}

/** What the marketplace logins on this machine amount to.
 *
 *  A saved login is a credential on that machine, and saying so is the whole
 *  of the claim: we have never held it, we cannot test it from here, and the
 *  machine reporting it may have been off since Tuesday. "Ready" was the word
 *  this replaces, and it asserted all three of the things it could not know. */
export function loginWords(device: DeviceView): string {
	const saved = device.sessions.filter((session) => session.status === 'connected').length;
	if (saved === 0) {
		return 'No marketplace login saved on this machine.';
	}
	const plural = saved === 1 ? 'login' : 'logins';
	return `${saved} marketplace ${plural} saved on this machine. A saved login may no longer work.`;
}

/** What this installation's version does and does not allow, in the seller's
 *  words.
 *
 *  Narrow, because the server's floor is narrow: the import claim admits any
 *  registered, unrevoked device without looking at its version, and the
 *  version floor applies only to an item whose payload comes from a
 *  marketplace — publishing a file to one, or fetching one back. So an older
 *  installation imports catalogues and runs ordinary uploaded-file work, and
 *  saying it "cannot run imports" was both wrong and the kind of wrong that
 *  makes a seller update a machine that was working.
 *
 *  A machine at or past the floor says nothing at all rather than offering
 *  reassurance: a line on every row claiming a capability is noise, and the
 *  version itself is in the row's details for anyone who wants it. */
export function sourcedFileWords(device: DeviceView): string | null {
	return device.runs_sourced_payloads
		? null
		: 'Update the Teachouse app on this machine to publish or download files from a marketplace. ' +
				'Importing your catalogue and using your own uploaded files still work.';
}

/** The list split in two, in the order the seller needs it.
 *
 * `current` is what the screen shows: the installations that are not signed
 * out, this machine first, then the rest most recently heard from first. The
 * machine the seller is standing at is identified by its installation id and
 * never by a display name — two laptops called "MacBook Pro" are two
 * machines, and matching on the name is how a console comes to offer one
 * machine's actions on another's row.
 *
 * `history` is every signed-out record, most recently signed out first. Kept
 * rather than deleted or merged: a sign-out is a decision the seller made and
 * the record is the evidence of it, and two records sharing a name are still
 * two installations. A reinstall mints a new identity, so a seller who has
 * reinstalled three times has three records and only one of them is theirs to
 * use — which is exactly what the split says.
 *
 * Sorting is stable within each group, so a background refresh cannot
 * reshuffle rows the seller is reading.
 */
export function inListOrder<T extends { device: DeviceView }>(
	rows: readonly T[],
	here: string | null
): { current: T[]; history: T[] } {
	const current = rows
		.filter((row) => row.device.revoked_at === null)
		.sort((left, right) => {
			if (left.device.id === here) {
				return -1;
			}
			if (right.device.id === here) {
				return 1;
			}
			return right.device.last_seen_at - left.device.last_seen_at;
		});
	const history = rows
		.filter((row) => row.device.revoked_at !== null)
		.sort((left, right) => (right.device.revoked_at ?? 0) - (left.device.revoked_at ?? 0));
	return { current, history };
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
		return 'This machine could not reach Teachouse, and did not say why.';
	}
	const stop = /[.!?]$/.test(answer.detail) ? '' : '.';
	return `This machine could not reach Teachouse: ${answer.detail}${stop}`;
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
		? { label: 'Refreshing…', reason: 'Refreshing this machine.', disabled: true }
		: { label: 'Refresh this machine', reason: undefined, disabled: false };
}
