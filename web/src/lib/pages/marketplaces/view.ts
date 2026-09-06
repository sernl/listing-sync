// What a connection card says, derived from the row the shared device view
// already builds. Pure, so it tests without a component.

import { connectionIsLinked, connectionStands } from '$lib/connection-standing';
import { platformOf } from '$lib/device-merge';
import { SIGN_IN_LABEL } from '$lib/devices-view';
import type { MarketplaceRow, MarketplaceSignIn, SignInState } from '$lib/devices-view';
import type { Tone } from '$lib/StatusPill.svelte';
import type { Marketplace, TransportClass } from '$lib/generated/vocab';

/**
 * The pill tone for one marketplace's sign-in state.
 *
 * A total map from the tone `$lib/devices-view` assigns onto the six the status
 * badge draws, which are not the same vocabulary: the older one carries `mut`
 * for a state with nothing to report and `run` for one that is verifying and
 * failing. The design specification assigns those to grey and amber
 * respectively — a failing verification is something to attend to, and a
 * pending one is not — so `run` lands on `warn` here rather than on the badge's
 * own accent-coloured `run`, which this page never uses.
 */
const PILL_TONE: Record<MarketplaceSignIn['tone'], Tone> = {
	ok: 'ok',
	bad: 'bad',
	mut: 'soon',
	run: 'warn'
};

export function pillTone(row: MarketplaceRow): Tone {
	return PILL_TONE[row.signIn.tone];
}

/** Whether this marketplace is carrying work right now.
 *
 *  A total map over the state union, so a state added in `$lib/devices-view` is
 *  classified here rather than falling into whichever branch happens to be the
 *  else. Two states count: a login held on one of the seller's own machines,
 *  and a connection our own infrastructure is serving. Everything else is a
 *  marketplace that has to be connected before anything moves. */
const CARRYING: Record<SignInState, boolean> = {
	signed_in: true,
	served_here: true,
	needs_signin: false,
	unverified: false,
	no_account: false,
	no_device: false,
	all_signed_out: false
};

export function carrying(row: MarketplaceRow): boolean {
	return CARRYING[row.signIn.state];
}

/** Where the machines this console knows about are listed. One constant,
 *  because the card foot and the sign-in explanation both point at it. */
export const MACHINES_ANCHOR = '#machines';

/** Where a seller gets the app that holds a marketplace login. */
export const DOWNLOADS_ANCHOR = '#downloads';

/** How the two live marketplaces write their own names, in capitals, which is
 *  what the card heading shows. `$lib/platforms` spells them with the full
 *  name in brackets, which is right for a row in a list and too long for a
 *  card heading beside a logo. */
export const CARD_NAME: Record<Marketplace, string> = {
	Tes: 'TES',
	Tpt: 'TPT',
	Etsy: 'Etsy'
};

/**
 * Which of the console's two hosts is reading the page.
 *
 * Not a guess about the machine but a statement about what this copy of the
 * console can do: `app` means a marketplace login can be opened from here, and
 * `browser` means it cannot and the seller has to be told where it can.
 *
 * Android is `browser` even though it is the application, and deliberately.
 * `connect_marketplace` builds a second window unconditionally
 * (`apps/desktop/src-tauri/src/commands.rs`), Tauri's mobile surface is a
 * single Activity, and `docs/notes/design/android-client.md` records that the
 * phone path has to navigate the one webview and navigate back instead. Until
 * that lands, a phone offering the button would offer one that does nothing,
 * which is worse than a sentence saying where to connect.
 */
export type ConnectHost = 'app' | 'browser';

/**
 * Which host this copy of the console is, from the two facts that decide it.
 *
 * The invoker is the only honest test of whether a marketplace login can be
 * opened from here: it is present exactly where the console is running inside
 * the application, and absent in every browser and in the dev server opened
 * directly. The user agent then takes Android back out, for the reason
 * `ConnectHost` states.
 *
 * Both inputs are read by the caller rather than here, so this stays a pure
 * function of two values and does not need a window to be tested.
 */
export function hostOf(invoker: unknown, userAgent: string | null): ConnectHost {
	if (invoker === null || invoker === undefined) {
		return 'browser';
	}
	return platformOf(userAgent) === 'android' ? 'browser' : 'app';
}

/** What a card's foot offers: a place to go, or an act to perform here.
 *
 * Two arms rather than one link with an optional handler, because the two are
 * different elements — an anchor and a button — and a component given both
 * would have to decide which, on a fact only this function knows. */
export type CardAction =
	| { kind: 'link'; label: string; href: string }
	| { kind: 'command'; label: string; marketplace: Marketplace };

/**
 * The card's one action.
 *
 * A marketplace carrying work opens onto the machine holding its login, which
 * is the only thing there is to look at. One that is not is connected here
 * where this console can do it, and named as an act for elsewhere where it
 * cannot: a login for a marketplace on the device branch is captured in the
 * app on the seller's own machine or nowhere, because D1 leaves no server-side
 * connect for it and no endpoint links one on the API branch either.
 *
 * The label carries the marketplace's name rather than the card appending it,
 * because the browser arm is a sentence and not a verb, and "Connect from the
 * Teachouse app TPT" is what appending would produce.
 *
 * Neither arm ever leaves for the marketplace's own website. The command hands
 * a name to the application; the link goes to our own downloads.
 */
export function footerAction(row: MarketplaceRow, host: ConnectHost): CardAction {
	const name = CARD_NAME[row.marketplace];
	if (carrying(row)) {
		return { kind: 'link', label: `Open ${name}`, href: MACHINES_ANCHOR };
	}
	return host === 'app'
		? { kind: 'command', label: `Connect ${name}`, marketplace: row.marketplace }
		: {
				kind: 'link',
				label: `Connect ${name} from the Teachouse app on your computer`,
				href: DOWNLOADS_ANCHOR
			};
}

/**
 * Whether this marketplace has a connection to disconnect.
 *
 * Read off the stored row rather than off the sign-in state, because those are
 * different facts: a machine signed out of TPT still leaves the tenant's
 * connection standing, and it is the connection the seller asked to be able to
 * remove. A marketplace with no row at all has nothing to disconnect and is
 * offered Connect alone.
 *
 * The same predicate the import and migration screens read, so one screen
 * cannot call a marketplace connected while another offers to disconnect it.
 */
export function disconnectable(connection: { state: string } | null | undefined): boolean {
	return connectionStands(connection);
}

/** The disconnect control's label, which names the marketplace for the same
 *  reason the footer action does. */
export function disconnectLabel(marketplace: Marketplace): string {
	return `Disconnect ${CARD_NAME[marketplace]}`;
}

/**
 * What the seller is asked before a disconnect, which differs by what this
 * console can actually reach.
 *
 * Three facts, and the third is the one a control plane must not leave unsaid.
 * Scheduled work stops, because every lease requires a linked connection.
 * Nothing is removed from the marketplace and the listings stay here as
 * records, because a disconnect is a statement about us rather than about the
 * listing. And in a browser the marketplace login is still on whichever
 * machine holds it — which is not merely a leftover: a machine that goes on
 * checking in while holding that login lifts the connection back to linked on
 * its next beat, because that is exactly what a check-in is for. Saying only
 * "the login remains on that machine" would leave a seller watching the
 * marketplace reconnect itself with no account of why.
 */
export function disconnectPrompt(
	marketplace: Marketplace,
	host: ConnectHost,
	heldOnAMachine: boolean
): string {
	const name = CARD_NAME[marketplace];
	const shared =
		`Scheduled work for ${name} stops. Nothing is removed from ${name} itself, and your ` +
		'listings stay here as records. Connecting again is the same button.';
	if (host === 'app') {
		return (
			`Disconnect ${name}?\n\n` +
			`The ${name} login is removed from this machine first. ` +
			shared
		);
	}
	if (!heldOnAMachine) {
		return `Disconnect ${name}?\n\n${shared}`;
	}
	return (
		`Disconnect ${name}?\n\n` +
		shared +
		`\n\nThe ${name} login is still on the machine that holds it, and this page cannot ` +
		'remove it: your logins are never on our servers. While that machine keeps checking in ' +
		`it reconnects ${name} by itself. To remove the login, disconnect in the Teachouse app ` +
		'on that machine, or sign the machine out under Your machines below.'
	);
}

/**
 * The whole of what a card asks before disconnecting, from the values the page
 * holds rather than from a boolean it derived itself.
 *
 * The derivation is here and not at the call site because it is the part that
 * can be wrong: whether a machine will reconnect this marketplace is read off
 * `connection.state`, the field `derive_link` and the disconnect route both
 * operate on, and a component reaching for the device registry's own sign-in
 * state instead would be a third answer to a question this feature already
 * standardised on one answer for.
 */
export function disconnectAsk(
	marketplace: Marketplace,
	host: ConnectHost,
	connection: { state: string } | null | undefined
): string {
	return disconnectPrompt(marketplace, host, connectionIsLinked(connection));
}

/** Which card is mid-flight, and at what. A record rather than one slot,
 *  because the cards are independent: two marketplaces are two logins on two
 *  windows, and a seller starting the second must not make the first look
 *  idle while its own login window is still open. */
export type Busy = Partial<Record<Marketplace, 'action' | 'disconnect'>>;

/** Start or end one card's busy state, leaving every other card's alone.
 *
 * A new record rather than a mutation, so a caller holding the old one cannot
 * observe a half-applied change. */
export function withBusy(
	busy: Busy,
	marketplace: Marketplace,
	at: 'action' | 'disconnect' | null
): Busy {
	const next: Busy = { ...busy };
	if (at === null) {
		delete next[marketplace];
	} else {
		next[marketplace] = at;
	}
	return next;
}

/** What this one card is doing, or undefined where it is idle. */
export function busyAt(busy: Busy, marketplace: Marketplace): 'action' | 'disconnect' | undefined {
	return busy[marketplace];
}

/** The page header's own action, which is a different question from a card's.
 *
 * In a browser there is one answer to "connect a marketplace" and it is the
 * app, so the header is the primary route to it. Inside the app the cards
 * below already are that route, and a header button pointing at them would be
 * a no-op dressed as the page's main action; what the downloads section is
 * still for there is the seller's other machine.
 */
export function headerAction(host: ConnectHost): { label: string; href: string } {
	return host === 'app'
		? { label: 'Install on another machine', href: DOWNLOADS_ANCHOR }
		: { label: 'Connect a marketplace', href: DOWNLOADS_ANCHOR };
}

/** The badge D1 requires on every marketplace row. */
export const TRANSPORT_BADGE: Record<TransportClass, string> = {
	SellerDevice: 'On your device',
	OfficialApi: 'On our servers'
};

/**
 * The transport line beneath the body, which says the same thing as the badge
 * in a sentence.
 *
 * The server-side sentence names the marketplace because the token is theirs:
 * "under a token we hold" would read as a credential of ours, and the whole
 * point of the sanctioned branch is that the marketplace issued it for the
 * purpose.
 */
export function transportLine(transport: TransportClass, name: string): string {
	return transport === 'SellerDevice'
		? 'Runs on your own device.'
		: `Runs on our servers, under a token ${name} issued us.`;
}

/** What the page knows about one live marketplace right now.
 *
 *  Three answers rather than two, and the third is not a row with empty fields:
 *  "the read has not landed" and "the read failed" are different statements
 *  from any state a marketplace can actually be in, and collapsing either into
 *  `disconnected` would tell a seller their marketplace is off when all we know
 *  is that we could not ask. This is the shape `TabBar`'s nullable `count`
 *  already uses for the same reason: a plausible stand-in is worse than an
 *  honest absence. */
export type LiveRead =
	| { state: 'read'; row: MarketplaceRow }
	| { state: 'pending' }
	| { state: 'failed' };

export interface LiveFace {
	status: { tone: Tone; label: string };
	handle: string | null;
	body: string;
	/** Absent unless the read landed. With no row there is no way to know
	 *  whether the honest word is "Open" or "Connect", and guessing either
	 *  sends the seller somewhere on a fact we do not have. */
	action?: CardAction;
}

/**
 * What a live marketplace's card says, whatever the read did.
 *
 * A total function over the three answers, so the tile is rendered from the
 * catalogue and never from the response: a marketplace the seller sells on
 * cannot disappear from a page titled "Every marketplace you sell on" because a
 * fetch failed, and the grid cannot reflow as the queries land.
 *
 * The transport badge is deliberately not this function's business. Which
 * branch a marketplace runs on is a recorded fact about the marketplace rather
 * than anything a read discovers, so D1's badge stands on all three answers.
 */
export function liveFace(held: LiveRead, host: ConnectHost): LiveFace {
	switch (held.state) {
		case 'read':
			return {
				status: { tone: pillTone(held.row), label: SIGN_IN_LABEL[held.row.signIn.state] },
				handle: held.row.signIn.accountLabel,
				body: held.row.signIn.line,
				action: footerAction(held.row, host)
			};
		case 'pending':
			return {
				status: { tone: 'soon', label: 'Checking' },
				handle: null,
				body: 'Reading what this marketplace is doing.'
			};
		case 'failed':
			return {
				status: { tone: 'soon', label: 'Not known' },
				handle: null,
				body: 'We could not read this one just now. Reload to try again.'
			};
	}
}

/**
 * The marketplaces whose work runs on the seller's own machine, in the order
 * this page tiles them.
 *
 * Ordered rather than filtered alone because the page lists the same pair
 * twice — once as cards and once as copyright declarations — and
 * `$lib/devices-view` ranks them the other way round, so without this the two
 * sections would disagree about which comes first on one screen.
 *
 * A marketplace on the device branch that this page does not tile still
 * appears, after the ones it does: dropping it would hide a declaration the
 * seller has to make, which is a worse failure than an order nobody specified.
 */
export function deviceBranchInTileOrder(
	rows: readonly MarketplaceRow[],
	tiled: readonly Marketplace[]
): MarketplaceRow[] {
	const rank = (marketplace: Marketplace) => {
		const at = tiled.indexOf(marketplace);
		return at === -1 ? tiled.length : at;
	};
	return rows
		.filter((row) => row.transport === 'SellerDevice')
		.slice()
		.sort((left, right) => rank(left.marketplace) - rank(right.marketplace));
}
