// The dashboard's own reading of the device registry: whether a machine is
// checking in, which marketplaces hold a login on one, and which are waiting
// for the seller to sign in there. Pure, so it tests without a component.
//
// Decision D1 is the reason this belongs on the dashboard rather than in
// settings. Work for a marketplace with no official API originates on the
// seller's own machine, so a machine that is off is a schedule that is not
// running, and the two failures Vendoo's own reviews generate most anger over
// are a missed run and a silently dropped connection. Both are observability.

import type { AuthorshipView, ConnectionView, DeviceSessionView, DeviceView } from '$lib/api';
import { present } from '$lib/connection-status';
import { TRANSPORT_OF } from '$lib/inventory';
import type { ConnectionStatus, Marketplace, TransportClass } from '$lib/generated/vocab';

/** How often a running installation checks in, matching `CHECK_IN_EVERY` in
 *  `apps/desktop/src-tauri/src/scheduler.rs`.
 *
 * Five minutes, not the hour: the check-in and the hourly marketplace sweep
 * are separate activities on separate cadences since the coordinator split
 * them, and this is the one that says "this machine is there". Reading the
 * sweep's hour here is what made the page call a laptop current two hours
 * after it was shut. */
export const CHECK_IN_CADENCE_MS = 5 * 60 * 1000;

/** How long a device may be silent before the page stops calling it current.
 *
 * Six cadences, so a machine that missed a few check-ins — a laptop asleep
 * for twenty minutes, a phone in a tunnel — is not reported as off, while
 * silence long enough to carry information is. */
export const QUIET_AFTER_MS = 6 * CHECK_IN_CADENCE_MS;

export type DeviceStanding = 'checking_in' | 'quiet' | 'signed_out';

export interface DeviceRow {
	device: DeviceView;
	standing: DeviceStanding;
	/** Signed out and not heard from since, so it may still hold the logins
	 *  listed against it. */
	wipeOutstanding: boolean;
	/** The marketplaces this machine reports holding a login for right now.
	 *
	 *  A saved login, and nothing more. It is not evidence that the machine is
	 *  there, that the login still works, or that anything is running on it:
	 *  those are `standing`, a validated check, and the run's own owner. */
	holding: Marketplace[];
	/** Whether this installation is new enough for work whose payload comes
	 *  from a marketplace — publishing or refetching a marketplace-sourced
	 *  file.
	 *
	 *  Narrow, and named narrowly on purpose. It is *not* permission to
	 *  import: `import_runs::claim` admits any registered, unrevoked device
	 *  without a version check, and the floor applies only to an item whose
	 *  payload is marketplace-sourced. An older installation still imports a
	 *  catalogue and still runs ordinary uploaded-file work, so it is neither
	 *  hidden nor counted as idle — the one thing it cannot do is named where
	 *  it matters and nowhere else. */
	runsSourcedFiles: boolean;
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
		holding: held(device.sessions),
		runsSourcedFiles: device.runs_sourced_payloads
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
	/** Machines the list shows as current: the ones that are not signed out.
	 *
	 *  Counted because it is what the page displays. The old figure counted
	 *  every row the registry held, including installations replaced years of
	 *  reinstalls ago, so a seller with one working laptop read "1 of 4
	 *  machines have checked in" and could see only one.
	 *
	 *  The sourced-file floor is deliberately not part of it. An installation
	 *  below that floor still imports a catalogue and still runs ordinary
	 *  uploaded-file work, so counting it as not current would say something
	 *  false about a machine that is working. */
	current: number;
	/** Machines that are current but too old for marketplace-sourced files, so
	 *  the one thing they cannot do has an update as its remedy. Its own
	 *  figure because it is its own, narrow fact. */
	needingUpdateForSourcedFiles: number;
}

export function deviceSummary(rows: readonly DeviceRow[]): DeviceSummary {
	const summary: DeviceSummary = {
		total: rows.length,
		checkingIn: 0,
		quiet: 0,
		signedOut: 0,
		wipesOutstanding: 0,
		current: 0,
		needingUpdateForSourcedFiles: 0
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
		if (row.standing !== 'signed_out') {
			summary.current += 1;
			if (!row.runsSourcedFiles) {
				summary.needingUpdateForSourcedFiles += 1;
			}
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
	all_signed_out: 'No machine signed in',
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

/** One marketplace's whole row on the Marketplaces screen.
 *
 * The two branches of D1 answer the seller's one question — can this
 * marketplace be written to right now — from different places, so the row
 * carries both answers and the branch that decides which is authoritative.
 * `connection` is null on the device branch because no server has ever held a
 * session for one, and `signIn.device` is null on the API branch because no
 * device has ever held a token for one. */
export interface MarketplaceRow {
	marketplace: Marketplace;
	transport: TransportClass;
	signIn: MarketplaceSignIn;
	/** The stored connection record, where the branch has one. */
	connection: ConnectionView | null;
	/** The seller's own declaration of who holds the copyright, on whichever
	 *  branch the marketplace runs. It is a fact about the seller rather than
	 *  about a server-held session, so it is read off the connection list for
	 *  every marketplace and not off `connection`, which is null on the device
	 *  branch by construction. Undefined where the list carries no row for this
	 *  marketplace, or a row from a surface that serves no declarations. */
	authorship?: AuthorshipView;
	/** The machine holding this login has not checked in inside the window, so
	 *  scheduled work for this marketplace is not running. */
	quiet: boolean;
	/** Machines signed out that have not been heard from since and still report
	 *  holding this marketplace's login, so it may remain on them. */
	wipeOutstandingOn: DeviceView[];
}

/**
 * Every marketplace, once, with whichever branch's facts apply to it.
 *
 * Built over the same ranked order the band uses, so the two surfaces never
 * list the same pair differently, and over the generated `Marketplace` union
 * rather than over the rows the API happened to return: a marketplace with no
 * connection and no device is a row that says so, not a row that is missing.
 */
export function marketplaceRows(
	devices: readonly DeviceView[],
	connections: readonly ConnectionView[],
	now: number
): MarketplaceRow[] {
	const rows = deviceRows(devices, now);
	const states = signInStates(devices, connections, now);
	return states.map((signIn) => {
		const transport = TRANSPORT_OF[signIn.marketplace];
		const holder = rows.find((row) => row.device.id === signIn.device?.id);
		const stored = connections.find((entry) => entry.marketplace === signIn.marketplace);
		return {
			marketplace: signIn.marketplace,
			transport,
			signIn,
			connection: transport === 'OfficialApi' ? (stored ?? null) : null,
			authorship: stored?.authorship,
			quiet: holder?.standing === 'quiet',
			wipeOutstandingOn: rows
				.filter(
					(row) =>
						row.standing === 'signed_out' &&
						row.wipeOutstanding &&
						row.holding.includes(signIn.marketplace)
				)
				.map((row) => row.device)
		};
	});
}

/** Which sign-in states are the seller being asked to do something.
 *
 * A total map over the union, so a state added above is classified here rather
 * than falling silently out of the attention list. `no_account` and
 * `no_device` are excluded deliberately: a marketplace never linked and a
 * seller with no machine registered are both setup this seller has not done
 * yet, not something that broke. */
const ATTENTION_STATE: Record<SignInState, boolean> = {
	signed_in: false,
	needs_signin: true,
	unverified: false,
	no_account: false,
	no_device: false,
	all_signed_out: true,
	served_here: false
};

/** The marketplaces the screen puts at the top because the seller has to act.
 *
 * Both branches, unlike `needingDeviceSignIn`: on the merged screen the remedy
 * for a dropped Etsy connection and the remedy for an unheld TPT login are two
 * rows apart rather than two screens apart, so naming both here no longer
 * sends the seller somewhere that cannot help.
 *
 * Two facts join the state map. A connection that is `unstable` is verifying
 * and failing, which `unverified` alone does not distinguish from the ordinary
 * `checking`; and a login held on a machine that has gone quiet is a schedule
 * that is not running, which the state cannot say because the sign-in itself
 * is intact. */
export function needingAttention(rows: readonly MarketplaceRow[]): MarketplaceRow[] {
	return rows.filter(
		(row) =>
			ATTENTION_STATE[row.signIn.state] ||
			row.connection?.status === 'unstable' ||
			row.quiet
	);
}

/** The marketplaces waiting on the seller signing in on one of their own
 *  machines.
 *
 * Only `needs_signin`, because a seller with no machine registered, or one who
 * signed every machine out, is not being asked to sign in to a marketplace,
 * and counting them here would name an action they cannot take next.
 *
 * And only the device branch. A marketplace served from our own
 * infrastructure whose connection dropped is re-linked on the connections
 * page, which the dashboard's attention panel already says and this panel
 * cannot link to; carrying it here made one dropped connection speak three
 * times on one screen, twice in words that named the wrong remedy. */
export function needingDeviceSignIn(
	states: readonly MarketplaceSignIn[]
): MarketplaceSignIn[] {
	return states.filter(
		(entry) =>
			entry.state === 'needs_signin' && TRANSPORT_OF[entry.marketplace] === 'SellerDevice'
	);
}

/** Whether anything scheduled can run at all right now.
 *
 * False where every marketplace on the device branch is waiting on a machine
 * that is not checking in, which is the fact a seller most needs stated
 * plainly and the one Vendoo buries in a support article.
 *
 * Two facts, both required and neither standing in for the other: the machine
 * has been heard from, and it holds a marketplace login. A held login on a
 * machine that is off is the case this exists to stop reading as "running".
 *
 * The sourced-file floor is deliberately absent. It is not permission to work:
 * an older installation runs a catalogue import and ordinary uploaded-file
 * work, and only an item whose payload comes from a marketplace is withheld
 * from it. Gating this on that flag said "nothing scheduled is running" about
 * a machine that was running most of it. */
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
			body: 'No machine of yours has checked in for half an hour, and a machine that is running checks in every few minutes. Queued work waits until one does; nothing is lost.'
		};
	}
	return {
		kind: 'no_login',
		tone: 'attn',
		headline: 'Nothing scheduled is running',
		body: 'A machine is checking in, and none holds a marketplace login yet, so there is nothing for it to run.'
	};
}

function count(n: number, one: string, many: string): string {
	return n === 1 ? one : many;
}

/**
 * The sentence under the marketplace rows.
 *
 * The denominator is the machines that could check in, not every machine ever
 * registered: a machine the seller signed out is not failing to report, and
 * counting it as one that has not checked in states something false and then
 * repairs it a sentence later. Where nothing is left to count, the sentence
 * leads with that instead of dividing by it.
 *
 * How many have gone quiet is deliberately not stated: quiet is exactly the
 * live machines less the ones checking in, so the figure is already on the
 * page and repeating it is a second way to say one thing.
 *
 * Returned as a string rather than branched in the markup, so each reading is
 * a test. Empty for a registry with no machines, which the band answers with
 * its own placeholder.
 */
export function deviceFootnote(summary: DeviceSummary): string {
	if (summary.total === 0) {
		return '';
	}
	const live = summary.total - summary.signedOut;
	const wipes =
		summary.wipesOutstanding === 0
			? ''
			: ` ${summary.wipesOutstanding} ${count(
					summary.wipesOutstanding,
					'machine was',
					'machines were'
				)} signed out and ${count(
					summary.wipesOutstanding,
					'has',
					'have'
				)} not been heard from since, so ${count(
					summary.wipesOutstanding,
					'it',
					'they'
				)} may still hold the marketplace logins listed against ${count(
					summary.wipesOutstanding,
					'it',
					'them'
				)}.`;
	if (live === 0) {
		return `Every machine you have registered is signed out.${wipes}`;
	}
	const signedOut =
		summary.signedOut === 0
			? ''
			: ` ${summary.signedOut} ${count(
					summary.signedOut,
					'other is',
					'others are'
				)} signed out and kept in history.`;
	const stale =
		summary.needingUpdateForSourcedFiles === 0
			? ''
			: ` ${summary.needingUpdateForSourcedFiles} ${count(
					summary.needingUpdateForSourcedFiles,
					'machine needs',
					'machines need'
				)} a newer Teachouse app before ${count(
					summary.needingUpdateForSourcedFiles,
					'it',
					'they'
				)} can publish or refetch a file from a marketplace. Everything else runs there as normal.`;
	return (
		`${summary.checkingIn} of ${live} ${count(
			live,
			'machine has',
			'machines have'
		)} checked in in the last half hour.${stale}${signedOut}${wipes}`
	);
}
