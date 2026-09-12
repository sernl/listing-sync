/** The machine this console is being read on, held once for every surface
 *  that says so.
 *
 * A module store rather than a per-component derivation, for the reason
 * `account-tile.svelte.ts` is one: three places say which machine the seller
 * is at — the top strip's account link, the phone's account avatar, and the
 * Preferences head — and only one of them can afford to ask. The check-in that
 * learns it happens once per load in `Console.svelte`, which is the one
 * component that renders only under a session, and the answer is kept here
 * rather than passed down through every page.
 *
 * `revoked` is the fact the founder's 0.7.0 report turned on: a machine he
 * signed out from the console keeps its identity, keeps checking in, and wipes
 * its marketplace logins on every cycle, so every Connect on it fails after
 * the password has been typed. It is held here so the console can say so on
 * the machine it happened to, and offer the one act that ends it. */

import { api } from '$lib/api';
import { checkInHere, desktopInvoker, type CheckInHere } from '$lib/desktop';
import type { MachineHere } from '$lib/machine-here';

let inApp = $state(false);
let device = $state<{ id: string; name: string } | null>(null);
let revoked = $state(false);
let restoring = $state(false);

export const machineHere = {
	/** Which machine, and which host, for the sentence every surface shows. */
	get where(): MachineHere {
		return { inApp, device };
	},

	/** Whether this machine was signed out from the console.
	 *
	 *  Only ever true from a check-in that reached the server: a machine that
	 *  could not reach us knows nothing new, and a banner raised on a dropped
	 *  connection would accuse the seller of an act they did not perform. */
	get revoked(): boolean {
		return revoked;
	},

	get restoring(): boolean {
		return restoring;
	},

	/** What the load-time check-in found. Called from an effect rather than
	 *  assigned at setup, because the check-in is a round trip to the
	 *  application and every frame before it lands is a real state. */
	observe(answer: CheckInHere, host: { inApp: boolean }) {
		inApp = host.inApp;
		device = answer.device;
		revoked = answer.revoked;
	},

	/** Sign this machine back in: clear the revocation, then check in again.
	 *
	 *  Both halves, in this order, because they answer different questions.
	 *  The restore is the seller's explicit act and the only thing that clears
	 *  the mark — the heartbeat deliberately never does, so that signing a
	 *  machine out stays a decision rather than a delay. The check-in that
	 *  follows is what re-reports the marketplace logins this machine holds,
	 *  so the list the seller is looking at is current by the time the button
	 *  goes idle.
	 *
	 *  Throws the restore's own failure, so the caller can show the server's
	 *  sentence: the refusal that matters is the machine cap, and it names a
	 *  plan and a number that we do not. */
	async signBackIn(): Promise<void> {
		const here = device;
		if (here === null) {
			throw new Error('this console does not know which machine it is on');
		}
		restoring = true;
		try {
			await api.restoreDevice(here.id);
			const answer = await checkInHere(desktopInvoker());
			// Only a check-in that reached us may clear the banner. One that did
			// not says nothing about the mark, and the restore's own 200 is the
			// server having cleared it, so the seller is not left reading an
			// accusation that no longer holds.
			revoked = answer.reached ? answer.revoked : false;
		} finally {
			restoring = false;
		}
	}
};
