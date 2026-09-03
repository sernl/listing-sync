// Joining a machine in our device registry to the browser sign-in it was
// made from, and being honest about the join.
//
// Half is ours: the device registry, which knows a machine's name,
// operating system and the marketplaces it holds. Half is better-auth's: the
// browser login sessions, which know only `ipAddress` and `userAgent` and
// carry no device-name concept at all (decision D14). There is no shared key
// between them, so the match below is a heuristic and the page says so
// wherever it is not certain.
//
// The rule is deliberately conservative. A login session is attributed to a
// registered device only when the platform read off its user agent is one that
// exactly one device and exactly one login session claim. Anything else —
// two Windows machines, two browsers on one machine, a platform no device
// reports — is left unattributed rather than guessed, because a sign-out
// control attached to the wrong row is worse than one the seller has to
// choose between.

import type { DeviceView } from '$lib/api';

/** One better-auth session row, narrowed to what the join and the page read.
 *
 * `token` rather than `id` is what `revokeSession` takes; both are present on
 * the row and only one of them works, so the type carries the one that does. */
export interface BrowserSession {
	id: string;
	token: string;
	ipAddress?: string | null;
	userAgent?: string | null;
	createdAt?: string | Date | null;
	updatedAt?: string | Date | null;
}

/** The platform tokens the device registry stores, which are the ones
 *  `tauri-plugin-os` reports. The user-agent reader below answers in the same
 *  vocabulary so the two are comparable without a translation table. */
export type Platform = 'windows' | 'macos' | 'linux' | 'android' | 'ios';

/** How sure we are that a login session belongs to a device.
 *
 * `matched` is the only value that attaches a session to a row; the other two
 * are rendered as words rather than silently collapsed into `matched`. */
export type Confidence = 'matched' | 'ambiguous' | 'unmatched';

export interface DeviceRow {
	device: DeviceView;
	/** Present only when `confidence` is `matched`. */
	session: BrowserSession | null;
	confidence: Confidence;
	/** The browser this page is being read in. */
	isCurrent: boolean;
}

export interface Merged {
	rows: DeviceRow[];
	/** Login sessions no device row claims: another browser, a phone that has
	 *  not installed the desktop client, or one of several machines on the same
	 *  platform. Signing one out is offered on its own row. */
	orphans: { session: BrowserSession; isCurrent: boolean }[];
}

/** The platform a user-agent string names, or null when it names none we
 *  recognise.
 *
 * Order matters: an Android user agent also contains "Linux", and an iPadOS
 * one can contain "Mac OS X", so the more specific tests come first. */
export function platformOf(userAgent: string | null | undefined): Platform | null {
	if (typeof userAgent !== 'string') {
		return null;
	}
	const agent = userAgent.toLowerCase();
	if (agent.includes('android')) {
		return 'android';
	}
	if (agent.includes('iphone') || agent.includes('ipad') || agent.includes('ipod')) {
		return 'ios';
	}
	if (agent.includes('windows')) {
		return 'windows';
	}
	if (agent.includes('mac os') || agent.includes('macintosh')) {
		return 'macos';
	}
	if (agent.includes('linux') || agent.includes('x11')) {
		return 'linux';
	}
	return null;
}

function normalisedOs(os: string): Platform | null {
	const token = os.trim().toLowerCase();
	const known: Platform[] = ['windows', 'macos', 'linux', 'android', 'ios'];
	return known.find((platform) => platform === token) ?? null;
}

/**
 * Join the registry against the identity service's session list.
 *
 * `currentToken` is the session token of the browser reading the page, which
 * better-auth hands back from `getSession`; the row it lands on is marked so
 * the seller can tell which sign-out ends the session they are using.
 */
export function merge(
	devices: DeviceView[],
	sessions: BrowserSession[],
	currentToken: string | null
): Merged {
	const byPlatform = new Map<Platform, BrowserSession[]>();
	for (const session of sessions) {
		const platform = platformOf(session.userAgent);
		if (platform === null) {
			continue;
		}
		byPlatform.set(platform, [...(byPlatform.get(platform) ?? []), session]);
	}

	const devicePlatforms = devices.map((device) => normalisedOs(device.os));
	const claimed = new Set<string>();

	const rows: DeviceRow[] = devices.map((device, index) => {
		const platform = devicePlatforms[index];
		const candidates = platform === null ? [] : (byPlatform.get(platform) ?? []);
		const siblings = devicePlatforms.filter((other) => other !== null && other === platform).length;

		if (candidates.length === 0) {
			return { device, session: null, confidence: 'unmatched', isCurrent: false };
		}
		if (candidates.length > 1 || siblings > 1) {
			// Two browsers on one machine, or two machines running the same
			// operating system: the platform no longer identifies anything.
			return { device, session: null, confidence: 'ambiguous', isCurrent: false };
		}
		const session = candidates[0];
		claimed.add(session.token);
		return {
			device,
			session,
			confidence: 'matched',
			isCurrent: currentToken !== null && session.token === currentToken
		};
	});

	const orphans = sessions
		.filter((session) => !claimed.has(session.token))
		.map((session) => ({
			session,
			isCurrent: currentToken !== null && session.token === currentToken
		}));

	return { rows, orphans };
}

/** What the page says beside a row about how it was matched. One sentence
 *  each, and never an implied certainty the join does not have. */
export function matchNote(confidence: Confidence): string {
	switch (confidence) {
		case 'matched':
			return 'Matched to a browser sign-in by its reported platform.';
		case 'ambiguous':
			return 'Several sign-ins report this platform, so we cannot say which belongs to this machine. Signing out here signs the machine out; end a browser sign-in from its own row below.';
		case 'unmatched':
			return 'No browser sign-in reports this platform.';
	}
}

/** A short human label for one browser sign-in: the platform if we can read
 *  one, and the address it was last seen from. Both come off better-auth's own
 *  two columns; there is nothing else on the row to name it with. */
export function sessionLabel(session: BrowserSession): string {
	const platform = platformOf(session.userAgent);
	const where = session.ipAddress?.trim();
	const parts = [
		platform === null ? 'Unrecognised browser' : `Browser on ${platform}`,
		where ? `from ${where}` : null
	];
	return parts.filter((part) => part !== null).join(' ');
}
