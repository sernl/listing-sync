// The dashboard's own reading of the device registry: whether a machine is
// checking in, which marketplaces hold a login on one, and which are waiting
// for the seller to sign in there. Pure, so it tests without a component.
//
// Decision D1 is the reason this belongs on the dashboard rather than in
// settings. Work for a marketplace with no official API originates on the
// seller's own machine, so a machine that is off is a schedule that is not
// running, and the two failures Vendoo's own reviews generate most anger over
// are a missed run and a silently dropped connection. Both are observability.

import type { ConnectionView, DeviceSessionView, DeviceView } from '$lib/api';
import { present } from '$lib/connection-status';
import { TRANSPORT_OF } from '$lib/inventory';
import type { ConnectionStatus, Marketplace } from '$lib/generated/vocab';

/** How often the desktop client checks in, matching
 *  `Scheduler::DEFAULT_CADENCE` in `apps/desktop`. */
export const CHECK_IN_CADENCE_MS = 60 * 60 * 1000;

/** How long a device may be silent before the page stops calling it current.
 *
 * Two cadences, so one missed check-in is not reported as a machine being off:
 * a laptop that slept through a tick is the ordinary case, and two misses is
 * the first point at which silence carries information. */
export const QUIET_AFTER_MS = 2 * CHECK_IN_CADENCE_MS;

export type DeviceStanding = 'checking_in' | 'quiet' | 'signed_out';

export interface DeviceRow {
	device: DeviceView;
	standing: DeviceStanding;
	/** Signed out and not heard from since, so it may still hold the logins
	 *  listed against it. */
	wipeOutstanding: boolean;
	/** The marketplaces this machine reports holding a login for right now. */
	holding: Marketplace[];
}

export function deviceStanding(device: DeviceView, now: number): DeviceStanding {
	if (device.revoked_at !== null) {
		return 'signed_out';
	}
	return now - device.last_seen_at <= QUIET_AFTER_MS ? 'checking_in' : 'quiet';
}

function held(sessions: readonly DeviceSessionView[]): Marketplace[] {
	return sessions
		.filter((session) => session.status === 'connected')
		.map((session) => session.marketplace);
}

export function deviceRows(devices: readonly DeviceView[], now: number): DeviceRow[] {
	return devices.map((device) => ({
		device,
		standing: deviceStanding(device, now),
		wipeOutstanding: device.wipe_outstanding,
		holding: held(device.sessions)
	}));
}

export interface DeviceSummary {
	/** Machines registered, signed-out ones included. */
	total: number;
	/** Machines that have checked in inside the window. */
	checkingIn: number;
	/** Machines that are neither signed out nor checking in. */
	quiet: number;
	/** Machines the seller has signed out. Counted rather than derived from the
	 *  other three, because the copy branches on it and arithmetic that has to
	 *  be right in two places is arithmetic that will be wrong in one. */
	signedOut: number;
	/** Machines signed out that have not been heard from since, so their
	 *  marketplace logins may still be on them. */
	wipesOutstanding: number;
}

export function deviceSummary(rows: readonly DeviceRow[]): DeviceSummary {
	const summary: DeviceSummary = {
		total: rows.length,
		checkingIn: 0,
		quiet: 0,
		signedOut: 0,
		wipesOutstanding: 0
	};
	for (const row of rows) {
		if (row.standing === 'checking_in') {
			summary.checkingIn += 1;
		}
		if (row.standing === 'quiet') {
			summary.quiet += 1;
		}
		if (row.standing === 'signed_out') {
			summary.signedOut += 1;
		}
		if (row.wipeOutstanding) {
			summary.wipesOutstanding += 1;
		}
	}
	return summary;
}

export type SignInState =
	| 'signed_in'
	| 'needs_signin'
	| 'unverified'
	| 'no_account'
	| 'no_device'
	| 'all_signed_out'
	| 'served_here';

/** The pill's word. A total map so a state added above is worded here or
 *  fails the web lane rather than rendering as an identifier. */
export const SIGN_IN_LABEL: Record<SignInState, string> = {
	signed_in: 'Signed in',
	needs_signin: 'Sign in',
	unverified: 'Not verified',
	no_account: 'Not linked',
	no_device: 'No device',
	all_signed_out: 'Signed out',
	served_here: 'Served here'
};

/** What each stored connection status means for a marketplace served from our
 *  own infrastructure.
 *
 * A total map over the generated union, so a status added in Rust is
 * classified here rather than falling into whichever branch happens to be the
 * else. Only `connected` is being served; `checking` and `unstable` are not,
 * and neither is healthy. */
const SERVER_SIDE_STATE: Record<ConnectionStatus, SignInState> = {
	connected: 'served_here',
	checking: 'unverified',
	unstable: 'unverified',
	disconnected: 'needs_signin'
};

/** The order the band lists marketplaces in.
 *
 * Ranked through a total map rather than written out as a list, so a
 * marketplace added to the generated union has to be ranked here instead of
 * silently dropping off the band. */
const RANK: Record<Marketplace, number> = { Tpt: 0, Tes: 1, Etsy: 2 };

const ORDER: readonly Marketplace[] = (Object.keys(RANK) as Marketplace[]).sort(
	(left, right) => RANK[left] - RANK[right]
);

/** Where one marketplace's login lives, and whether it is there.
 *
 * The branch decides which plane is authoritative, which is D1 read back out
 * of the data. For a marketplace with no official API the device registry is
 * the only place a login can be, because no server ever held one; for one with
 * a sanctioned API the connection record is, because no device does. */
export interface MarketplaceSignIn {
	marketplace: Marketplace;
	state: SignInState;
	/** The machine holding it, where one does. */
	device: DeviceView | null;
	/** The name the marketplace itself shows the seller, where the device
	 *  reported one. */
	accountLabel: string | null;
	/** One sentence, in the seller's words. */
	line: string;
	tone: 'ok' | 'run' | 'bad' | 'mut';
}

function servedHere(
	marketplace: Marketplace,
	connection: ConnectionView | undefined
): MarketplaceSignIn {
	const base = { marketplace, device: null, accountLabel: null };
	if (connection === undefined) {
		return {
			...base,
			state: 'no_account',
			line: 'Runs on our own infrastructure under a token this marketplace issues. No account is linked yet.',
			tone: 'mut'
		};
	}
	// The words and the tone are the connections page's own, so one status
	// never reads two ways in one console.
	const shown = present(connection.status);
	const state = SERVER_SIDE_STATE[connection.status];
	return {
		...base,
		state,
		line:
			state === 'served_here'
				? `${shown.explanation} No device is needed: this marketplace's automation is sanctioned and runs on our own infrastructure.`
				: shown.explanation,
		tone: shown.tone
	};
}

/**
 * Where each marketplace's login stands.
 *
 * Three answers are kept apart that a single "needs a sign-in" would merge,
 * because each names a different action. `no_device` is a seller with nothing
 * registered, who has to install the client before signing in anywhere.
 * `all_signed_out` is a seller who signed out every machine they had, which is
 * a decision they took rather than something that broke. `needs_signin` is a
 * machine that is there and does not hold this marketplace's login.
 */
export function signInStates(
	devices: readonly DeviceView[],
	connections: readonly ConnectionView[],
	now: number
): MarketplaceSignIn[] {
	const rows = deviceRows(devices, now);
	const live = rows.filter((row) => row.standing !== 'signed_out');
	return ORDER.map((marketplace) => {
		if (TRANSPORT_OF[marketplace] === 'OfficialApi') {
			return servedHere(
				marketplace,
				connections.find((entry) => entry.marketplace === marketplace)
			);
		}
		const holder = live.find((row) => row.holding.includes(marketplace));
		if (holder !== undefined) {
			const session = holder.device.sessions.find(
				(entry) => entry.marketplace === marketplace && entry.status === 'connected'
			);
			return {
				marketplace,
				state: 'signed_in' as const,
				device: holder.device,
				accountLabel: session?.account_label ?? null,
				line:
					holder.standing === 'checking_in'
						? `Signed in on ${holder.device.name}. Your login never leaves that device.`
						: `Signed in on ${holder.device.name}, which has not checked in lately, so scheduled work for this marketplace is not running.`,
				tone: holder.standing === 'checking_in' ? 'ok' : 'run'
			};
		}
		const base = { marketplace, device: null, accountLabel: null };
		if (devices.length === 0) {
			return {
				...base,
				state: 'no_device' as const,
				line: 'No machine of yours is registered, and this marketplace is signed into on your own device rather than here.',
				tone: 'mut' as const
			};
		}
		if (live.length === 0) {
			return {
				...base,
				state: 'all_signed_out' as const,
				line: 'Every machine of yours is signed out, so nothing holds a login for this marketplace. Sign in on a machine again to let its work run.',
				tone: 'run' as const
			};
		}
		return {
			...base,
			state: 'needs_signin' as const,
			line: 'No machine of yours holds a login for this marketplace. Sign in on the device that runs your syncs; your login never leaves it.',
			tone: 'bad' as const
		};
	});
}

/** The marketplaces waiting on the seller signing in somewhere.
 *
 * Only `needs_signin`: a seller with no machine registered, or one who signed
 * every machine out, is not being asked to sign in to a marketplace, and
 * counting them here would name an action they cannot take next. */
export function needingSignIn(states: readonly MarketplaceSignIn[]): MarketplaceSignIn[] {
	return states.filter((entry) => entry.state === 'needs_signin');
}

/** Whether anything scheduled can run at all right now.
 *
 * False where every marketplace on the device branch is waiting on a machine
 * that is not checking in, which is the fact a seller most needs stated
 * plainly and the one Vendoo buries in a support article. */
export function schedulesRunning(rows: readonly DeviceRow[]): boolean {
	return rows.some((row) => row.standing === 'checking_in' && row.holding.length > 0);
}

export type NoticeKind = 'all_signed_out' | 'nothing_checking_in' | 'no_login';

export interface BandNotice {
	kind: NoticeKind;
	tone: 'attn' | 'warn';
	headline: string;
	body: string;
}

/**
 * What the band says when nothing scheduled is running, or null when it has
 * nothing to say.
 *
 * Silence and a decision are different facts and get different sentences: a
 * seller who signed every machine out is not owed a warning that no machine
 * has checked in, because that is what they asked for. Pure and returned as
 * data rather than branched in the markup, so each sentence is a test.
 *
 * An empty registry returns null: the band renders its own placeholder there,
 * because "install the client" is not a notice about work that is stalled.
 */
export function bandNotice(summary: DeviceSummary, running: boolean): BandNotice | null {
	if (summary.total === 0) {
		return null;
	}
	if (summary.signedOut === summary.total) {
		return {
			kind: 'all_signed_out',
			tone: 'warn',
			headline: 'Every machine is signed out',
			body: 'Nothing scheduled runs until you sign in on a machine again. Queued work waits; nothing is lost.'
		};
	}
	if (running) {
		return null;
	}
	if (summary.checkingIn === 0) {
		return {
			kind: 'nothing_checking_in',
			tone: 'attn',
			headline: 'Nothing scheduled is running',
			body: 'No machine of yours has checked in for over two hours, and a machine checks in every hour. Queued work waits until one does; nothing is lost.'
		};
	}
	return {
		kind: 'no_login',
		tone: 'attn',
		headline: 'Nothing scheduled is running',
		body: 'A machine is checking in, and none holds a marketplace login yet, so there is nothing for it to run.'
	};
}
