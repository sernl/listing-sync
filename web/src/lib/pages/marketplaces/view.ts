// What a connection card says, derived from the row the shared device view
// already builds. Pure, so it tests without a component.

import { connectionIsLinked, connectionStands } from '$lib/connection-standing';
import { APP_CANNOT_FORGET, type SessionOutcome } from '$lib/desktop';
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
 * A phone is `app`. It was `browser` until the application gained a login path
 * that does not need a second window: `connect_marketplace` now navigates the
 * one webview to the marketplace's own sign-in and navigates back with the
 * verdict, which is what `signsInPlace` below distinguishes. What a phone can
 * do with a login is what a computer can — hold it, run the seller's queued
 * work against it, and be signed out from the console — so the two are one host
 * here and differ only in how the sign-in is presented.
 */
export type ConnectHost = 'app' | 'browser';

/**
 * Which host this copy of the console is.
 *
 * The invoker is the whole test, and the only honest one: it is present exactly
 * where the console is running inside the application, and absent in every
 * browser and in the dev server opened directly. It used to take the user agent
 * as well, to take Android back out; nothing takes Android out any more, so the
 * parameter went with the reason for it. Which surface the sign-in takes is
 * `signsInPlace`'s question, not this one's.
 */
export function hostOf(invoker: unknown): ConnectHost {
	return invoker === null || invoker === undefined ? 'browser' : 'app';
}

/**
 * Whether a sign-in started here replaces this page rather than opening a
 * window beside it.
 *
 * True on a phone alone, and it is a fact about the application rather than
 * about the screen: Android has one Activity, so tao answers
 * `NoAvailableActivity` for a second window and the login has to happen in the
 * one webview the console itself is in. Two things follow that a computer never
 * has to say — the connect's answer arrives in the address rather than in the
 * promise, and a disconnect leaves the marketplace's own cookie in the phone's
 * shared browser store, which `delete_cookie` cannot reach on Android.
 */
export function signsInPlace(invoker: unknown, userAgent: string | null): boolean {
	return hostOf(invoker) === 'app' && platformOf(userAgent) === 'android';
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
				label: `Connect ${name} from the Teachouse app on your computer or phone`,
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
	heldOnAMachine: boolean,
	inPlace = false
): string {
	const name = CARD_NAME[marketplace];
	const shared =
		`Scheduled work for ${name} stops. Nothing is removed from ${name} itself, and your ` +
		'listings stay here as records. Connecting again is the same button.';
	if (host === 'app') {
		return (
			`Disconnect ${name}?\n\n` +
			`The ${name} login is removed from this machine first. ` +
			shared +
			(inPlace ? `\n\n${STAYS_IN_THE_PHONES_BROWSER(name)}` : '')
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
	connection: { state: string } | null | undefined,
	inPlace = false
): string {
	return disconnectPrompt(marketplace, host, connectionIsLinked(connection), inPlace);
}

/**
 * What a phone's disconnect has to say that a computer's does not.
 *
 * Our own copy of the session is gone either way. What is left on Android is
 * the marketplace's own cookie, in the WebView's process-global store, and we
 * cannot remove it: `delete_cookie` is a no-op there, and the one lever that
 * works — `clear_all_browsing_data` — is all-or-nothing across every origin the
 * application has visited, which includes ours, so calling it would sign the
 * seller out of Teachouse as a side effect of disconnecting a marketplace.
 *
 * Said rather than silently accepted, because the alternative is a seller who
 * disconnects, presses Connect again, is signed straight in with no password,
 * and reasonably concludes the disconnect did nothing.
 */
const STAYS_IN_THE_PHONES_BROWSER = (name: string) =>
	`Your ${name} sign-in stays in this phone's browser, where we cannot remove it, so ` +
	`connecting ${name} again may not ask for your password.`;

/** What the control plane answered when asked to disconnect.
 *
 * As total as `SessionOutcome` already makes the device half, and for the same
 * reason: the sentence a seller reads depends on both halves, and a half that
 * throws out of the mutation cannot be one of the inputs to it. `moved` is the
 * number of connections the route reported disconnected, which is zero where
 * there was nothing standing to disconnect. */
export type DisconnectServer = { kind: 'moved'; moved: number } | { kind: 'refused' };

/** What a seller is told after a disconnect, and in which voice. */
export interface DisconnectSay {
	tone: 'info' | 'error';
	message: string;
}

/**
 * The sentence a finished disconnect leaves behind, from what both halves
 * answered.
 *
 * The order the two halves run in is what makes this a decision rather than two
 * independent sentences: the machine forgets first, and only then is the
 * control plane asked. So a server refusal arrives with the login already gone,
 * and the seller is standing in front of a machine that is signed out of a
 * marketplace this console still calls connected — which is not a half-failure
 * to be reported as a failure, because the half that matters is the one that
 * did not run: while the connection stands, so does every lease against it.
 *
 * A server refusal therefore leads, ahead of anything the device half reported.
 * Where the device half also failed, nothing at all happened, and saying so is
 * the whole of it; the machine's own refusal is reached on the retry that gets
 * past the server.
 */
export function disconnectSay(
	marketplace: Marketplace,
	forgotten: SessionOutcome,
	server: DisconnectServer
): DisconnectSay {
	const name = CARD_NAME[marketplace];
	if (server.kind === 'refused') {
		return {
			tone: 'error',
			message:
				forgotten.kind === 'done'
					? `The ${name} login has been removed from this machine, but ${name} is still ` +
						'connected here, so scheduled work has not stopped. Try Disconnect again.'
					: `${name} could not be disconnected.`
		};
	}
	if (forgotten.kind === 'unsupported') {
		return { tone: 'error', message: APP_CANNOT_FORGET };
	}
	if (forgotten.kind === 'refused') {
		return { tone: 'error', message: forgotten.detail };
	}
	return server.moved === 0
		? { tone: 'info', message: `${name} was already disconnected.` }
		: {
				tone: 'info',
				message: `${name} is disconnected. Connecting again is the same button.`
			};
}

/** The query parameter the application returns a phone's sign-in verdict in,
 *  and the one naming which marketplace it was about.
 *
 *  Spelled here as `apps/desktop/src-tauri/src/connect.rs` spells them; the
 *  address is the only channel between the two, because the page that asked for
 *  the sign-in was unloaded by it. */
export const CONNECT_RETURN_PARAM = 'connect';
export const CONNECT_RETURN_MARKETPLACE_PARAM = 'marketplace';

/** What a returning sign-in is told, in the shape `disconnectSay` already
 *  uses. */
export type ConnectReturn = DisconnectSay;

/**
 * The verdicts a phone's sign-in can end on.
 *
 * Listed as values rather than only as a type, because `view.test.ts` reads
 * `ConnectVerdict::code` out of `apps/desktop/src-tauri/src/connect.rs` and
 * compares the two sets: the address is the whole channel between the
 * application and this page, and a code the application sends that this file
 * does not word is a seller returned to the console in silence.
 */
export const CONNECT_VERDICT_CODES = [
	'captured',
	'deadline',
	'abandoned',
	'refused',
	'notkept',
	'signed_out'
] as const;

export type ConnectVerdictCode = (typeof CONNECT_VERDICT_CODES)[number];

/**
 * What each verdict says, where the marketplace is one this console knows.
 *
 * A total map rather than a switch with a default, so a verdict added in
 * `ConnectVerdict` is worded here or fails to compile — and the failure mode it
 * prevents is the one this whole return leg exists to fix: a seller returned to
 * the console with nothing said.
 *
 * `refused` and `notkept` are two different failures and were one code until a
 * review found what that cost: a seller whose device had been signed out of
 * Teachouse mid-sign-in was told the sign-in could not be opened, which is the
 * opposite of what happened and names nothing they can act on. `refused` is now
 * only the page never appearing, and `notkept` is a sign-in that got no further
 * than this device.
 *
 * `signed_out` is the machine having been signed out from the console before
 * the seller pressed Connect. The application checks in first now and refuses
 * in front of the password rather than capturing a session it is about to
 * wipe, so this sentence names the act that ended the sign-in and the act
 * that undoes it. `notkept` remains the case where the revocation arrived
 * mid-sign-in, which no check-in beforehand can prevent.
 *
 * Only `captured` is good news, and it is deliberately not the one that decides
 * the card. Whether a marketplace is connected is read from the server's own
 * connection list, so this sentence can be wrong about nothing: a hand-typed
 * parameter changes the line at the top of the page and changes no state at
 * all.
 */
const CONNECT_SAID: Record<ConnectVerdictCode, (name: string) => ConnectReturn> = {
	captured: (name) => ({ tone: 'info', message: `${name} is connected on this device.` }),
	deadline: (name) => ({
		tone: 'error',
		message: `The ${name} sign-in was not finished in time, so nothing was saved. Press Connect ${name} to try again.`
	}),
	abandoned: (name) => ({
		tone: 'error',
		message: `The ${name} sign-in did not finish, so nothing was saved. Press Connect ${name} to try again.`
	}),
	refused: (name) => ({
		tone: 'error',
		message: `The ${name} sign-in page did not open, so nothing was saved. Press Connect ${name} to try again.`
	}),
	notkept: (name) => ({
		tone: 'error',
		message: `The ${name} sign-in could not be saved on this device. If this device was signed out of Teachouse, sign in again here and then press Connect ${name}.`
	}),
	signed_out: (name) => ({
		tone: 'error',
		message: `This machine was signed out from the console, so the ${name} sign-in was not opened and nothing was saved. Sign this machine back in under Your machines, then press Connect ${name}.`
	})
};

/**
 * The same verdicts where the marketplace is one this console does not know,
 * which is an application newer than the page or an address somebody typed.
 *
 * Worded separately rather than by substituting a stand-in name into the
 * sentences above, because those are shaped around a name: the substitution
 * produced "marketplace is connected on this device." — a sentence starting
 * lower case — and "Press Connect marketplace to try again."
 */
const CONNECT_SAID_UNNAMED: Record<ConnectVerdictCode, ConnectReturn> = {
	captured: { tone: 'info', message: 'Your marketplace sign-in is saved on this device.' },
	deadline: {
		tone: 'error',
		message:
			'Your marketplace sign-in was not finished in time, so nothing was saved. Press Connect on the card to try again.'
	},
	abandoned: {
		tone: 'error',
		message:
			'Your marketplace sign-in did not finish, so nothing was saved. Press Connect on the card to try again.'
	},
	refused: {
		tone: 'error',
		message:
			'Your marketplace sign-in page did not open, so nothing was saved. Press Connect on the card to try again.'
	},
	notkept: {
		tone: 'error',
		message:
			'Your marketplace sign-in could not be saved on this device. If this device was signed out of Teachouse, sign in again here and then press Connect on the card.'
	},
	signed_out: {
		tone: 'error',
		message:
			'This machine was signed out from the console, so your marketplace sign-in was not opened and nothing was saved. Sign this machine back in under Your machines, then press Connect on the card.'
	}
};

/**
 * What the page says when a phone's sign-in has just returned it, or null when
 * this is an ordinary visit.
 *
 * Null on an unrecognised verdict, which is what makes reading an address safe:
 * a parameter nobody wrote and a code from a newer application both say nothing
 * rather than guessing. A marketplace name this console does not know is not a
 * reason to withhold the sentence — the verdict is still true — so it takes the
 * unnamed wording instead.
 */
export function connectReturn(params: URLSearchParams): ConnectReturn | null {
	const code = params.get(CONNECT_RETURN_PARAM) ?? '';
	if (!isVerdictCode(code)) {
		return null;
	}
	const named = params.get(CONNECT_RETURN_MARKETPLACE_PARAM) ?? '';
	return Object.hasOwn(CARD_NAME, named)
		? CONNECT_SAID[code](CARD_NAME[named as Marketplace])
		: CONNECT_SAID_UNNAMED[code];
}

/**
 * The same sentence, for a connect that answered on this page rather than
 * through the address.
 *
 * A computer's Connect never leaves the console, so its refusal arrives in the
 * promise and not in a query parameter — but it is the same refusal, and a
 * seller who met it on a phone and then on a laptop must not be given two
 * accounts of one state. Read out of the verdict table rather than written a
 * second time, so there is one sentence to change.
 */
export function connectSignedOut(marketplace: Marketplace): ConnectReturn {
	return CONNECT_SAID.signed_out(CARD_NAME[marketplace]);
}

/** Whether a string off the address is one of the verdicts we word.
 *
 * Read off `CONNECT_SAID`'s own keys rather than listed a second time, so the
 * set that is recognised and the set that has a sentence cannot come apart. */
function isVerdictCode(code: string): code is ConnectVerdictCode {
	return Object.hasOwn(CONNECT_SAID, code);
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
		? 'Runs on your own computer.'
		: `Runs on our servers, with the permission ${name} gave us.`;
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
