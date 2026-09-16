// What a connection card says, derived from the row the shared device view
// already builds. Pure, so it tests without a component.

import { connectionIsLinked, connectionStands } from '$lib/connection-standing';
import { APP_CANNOT_FORGET, type LocalSessionOutcome, type SessionOutcome } from '$lib/desktop';
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
export const MACHINES_ANCHOR = '/settings#machines';

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
 * The card's one action, decided by what THIS machine holds rather than by
 * what some machine holds.
 *
 * The defect this replaces: the action was read off `carrying`, which is true
 * as soon as any one of the seller's machines reports a login. A seller
 * standing at a second computer was shown "Open TPT", pointing at the first
 * one, and the card's only other control disconnected the marketplace for the
 * whole account — so the way to sign in on a second device was to take the
 * first one's login away.
 *
 * `local` is this machine's own answer and null is "not answered yet": a read
 * in flight is never read as absent, because acting on a fact we do not have
 * is the same defect one layer down. Where the answer is `known` and negative
 * the app offers its own sign-in whatever other machines report. A saved
 * local login can also be renewed: cookie presence does not promise the
 * marketplace still accepts it. An unanswered read keeps the machine link.
 *
 * A browser ignores `local` and has to: it holds no session, and
 * `sessionStatusHere` has no command to ask there. Its arm names the app
 * instead, as a sentence rather than a verb, because "Connect from the
 * Teachouse app TPT" is what appending a name to a verb would produce.
 *
 * Neither arm ever leaves for the marketplace's own website. The command hands
 * a name to the application; the link goes to our own downloads.
 */
export function footerAction(
	row: MarketplaceRow,
	host: ConnectHost,
	local: LocalSessionOutcome | null
): CardAction {
	const name = CARD_NAME[row.marketplace];
	if (host === 'app' && !carrying(row) && local?.kind !== 'known') {
		return { kind: 'link', label: 'Check this device', href: MACHINES_ANCHOR };
	}
	if (host === 'app' && local?.kind === 'known') {
		return {
			kind: 'command',
			label: local.connected ? `Sign in to ${name} again` : `Connect ${name}`,
			marketplace: row.marketplace
		};
	}
	if (carrying(row)) {
		return { kind: 'link', label: `Open ${name}`, href: MACHINES_ANCHOR };
	}
	return {
		kind: 'link',
		label: `Connect ${name} from the Teachouse app on your computer or phone`,
		href: DOWNLOADS_ANCHOR
	};
}

/** Whether this machine itself holds a login for one marketplace.
 *
 * False on every answer that is not a plain yes, null included. The two
 * predicates below are deliberately not each other's negation: a read in
 * flight, an application too old to answer and a refusal are none of them a
 * statement about what this machine holds. */
export function heldHere(local: LocalSessionOutcome | null): boolean {
	return local !== null && local.kind === 'known' && local.connected;
}

/** Whether this machine is known to hold no login for one marketplace, which
 *  is the one fact that earns a Connect on a card another machine is already
 *  signed in on. */
export function missingHere(local: LocalSessionOutcome | null): boolean {
	return local !== null && local.kind === 'known' && !local.connected;
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

/** The account-level control's label, which says which of the two ways out it
 *  is. This one is about the organisation's connection and removes no
 *  machine's login; the label has to carry that, because the card now offers
 *  both and a bare "Disconnect TPT" would be either. */
export function disconnectLabel(marketplace: Marketplace): string {
	return `Disconnect ${CARD_NAME[marketplace]} from your account`;
}

/** The local control's label. A sign-out rather than a disconnect, because
 *  that is exactly what it does: it removes this machine's copy of the login
 *  and asks the control plane for nothing. */
export function signOutHereLabel(marketplace: Marketplace): string {
	return `Sign out of ${CARD_NAME[marketplace]} on this device`;
}

/**
 * What the seller is asked before the account-level disconnect.
 *
 * Three facts, and the third is the one a control plane must not leave unsaid.
 * Scheduled work stops, because every lease requires a linked connection.
 * Nothing is removed from the marketplace and the listings stay here as
 * records, because a disconnect is a statement about us rather than about the
 * listing. And the marketplace login is still on whichever machine holds it —
 * which is not merely a leftover: a machine that goes on checking in while
 * holding that login lifts the connection back to linked on its next beat,
 * because that is exactly what a check-in is for. Saying only "the login
 * remains on that machine" would leave a seller watching the marketplace
 * reconnect itself with no account of why.
 *
 * Host-independent now, and that is the repair: this control performs one act
 * on one side, so the app is told the same thing a browser is. The claim that
 * a login is removed from this machine belongs to `signOutHereAsk`, which is
 * the control that removes it.
 */
export function disconnectPrompt(marketplace: Marketplace, heldOnAMachine: boolean): string {
	const name = CARD_NAME[marketplace];
	const shared =
		`Scheduled work for ${name} stops. Nothing is removed from ${name} itself, and your ` +
		'listings stay here as records. Connecting again is the same button.';
	if (!heldOnAMachine) {
		return `Disconnect ${name} from your account?\n\n${shared}`;
	}
	return (
		`Disconnect ${name} from your account?\n\n` +
		shared +
		`\n\nThis removes no machine's login: your logins are never on our servers, so the ` +
		`${name} login is still on the machine that holds it. While that machine keeps checking ` +
		`in it reconnects ${name} by itself. To remove the login, sign out of ${name} in the ` +
		'Teachouse app on that machine, or sign the machine out under Preferences > Machine sign-ins.'
	);
}

/**
 * The whole of what the account-level control asks, from the values the page
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
	connection: { state: string } | null | undefined
): string {
	return disconnectPrompt(marketplace, connectionIsLinked(connection));
}

/**
 * What the seller is asked before signing this machine out of a marketplace.
 *
 * Bounded on purpose, and the bound is the honest part: this act reaches one
 * machine's own store. Other machines keep their logins, and the account's
 * connection is not unlinked by it — it follows what machines report, so where
 * this was the only machine holding a login the connection falls away on its
 * next check-in rather than at the press of this button. Promising either more
 * or less than that is what a seller would catch us out on.
 */
export function signOutHereAsk(marketplace: Marketplace, inPlace = false): string {
	const name = CARD_NAME[marketplace];
	return (
		`Sign out of ${name} on this device?\n\n` +
		`The ${name} login is removed from this machine. Nothing is removed from ${name} ` +
		'itself, your listings stay here as records, and your other machines keep their own ' +
		`${name} logins. Your account stays connected while a machine still reports a ${name} ` +
		'login, so if this was the only one the connection falls away on its next check-in. ' +
		'Signing in again is the same button.' +
		(inPlace ? `\n\n${STAYS_IN_THE_PHONES_BROWSER(name)}` : '')
	);
}

/**
 * What a phone's local sign-out has to say that a computer's does not.
 *
 * Our own copy of the session is gone either way. What is left on Android is
 * the marketplace's own cookie, in the WebView's process-global store, and we
 * cannot remove it: `delete_cookie` is a no-op there, and the one lever that
 * works — `clear_all_browsing_data` — is all-or-nothing across every origin the
 * application has visited, which includes ours, so calling it would sign the
 * seller out of Teachouse as a side effect of disconnecting a marketplace.
 *
 * Said rather than silently accepted, because the alternative is a seller who
 * signs out, presses Connect again, is signed straight in with no password,
 * and reasonably concludes the sign-out did nothing.
 */
const STAYS_IN_THE_PHONES_BROWSER = (name: string) =>
	`Your ${name} sign-in stays in this phone's browser, where we cannot remove it, so ` +
	`connecting ${name} again may not ask for your password.`;

/** What the control plane answered when asked to disconnect.
 *
 * Total, because the sentence a seller reads is decided from it and a half
 * that throws out of the mutation cannot be one of the inputs to it. `moved`
 * is the number of connections the route reported disconnected, which is zero
 * where there was nothing standing to disconnect. */
export type DisconnectServer = { kind: 'moved'; moved: number } | { kind: 'refused' };

/** What a seller is told after a disconnect, and in which voice. */
export interface DisconnectSay {
	tone: 'info' | 'error';
	message: string;
}

/**
 * The sentence a finished account-level disconnect leaves behind.
 *
 * One half rather than two, because this control is one half: it asks the
 * control plane to unlink and touches no machine's login. What a machine holds
 * is the local sign-out's business and has its own sentence below, so neither
 * one reports an act the other performed.
 */
export function disconnectSay(marketplace: Marketplace, server: DisconnectServer): DisconnectSay {
	const name = CARD_NAME[marketplace];
	if (server.kind === 'refused') {
		return { tone: 'error', message: `${name} could not be disconnected.` };
	}
	return server.moved === 0
		? { tone: 'info', message: `${name} was already disconnected.` }
		: {
				tone: 'info',
				message: `${name} is disconnected from your account. Connecting again is the same button.`
			};
}

/**
 * The sentence a finished local sign-out leaves behind, total over every
 * answer the application can give.
 *
 * Total rather than defaulted, because the default is what made the old
 * two-half sentence claim success on answers that were not one: `opening` and
 * `signedOut` are in `SessionOutcome` and fell through to the confirming arm.
 * Each is named here for what it is instead.
 */
export function signOutHereSay(marketplace: Marketplace, forgotten: SessionOutcome): DisconnectSay {
	const name = CARD_NAME[marketplace];
	switch (forgotten.kind) {
		case 'done':
			return {
				tone: 'info',
				message: `The ${name} login is removed from this machine.`
			};
		case 'unsupported':
			return { tone: 'error', message: APP_CANNOT_FORGET };
		case 'refused':
			return { tone: 'error', message: forgotten.detail };
		case 'signedOut':
			return {
				tone: 'error',
				message:
					`This machine was signed out from the console, which already removed its ${name} ` +
					'login. Sign the machine back in under Preferences > Machine sign-ins.'
			};
		// A forget never asks for consent; the arm exists because the type is
		// shared with connect, and it is worded as the impossibility it is.
		case 'consentRequired':
		case 'opening':
		case 'unavailable':
			return {
				tone: 'error',
				message: `The ${name} login can only be removed in the Teachouse app on this machine.`
			};
	}
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
	'signed_out',
	'consent'
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
	captured: (name) => ({
		tone: 'info',
		message: `${name} is connected on this device.`
	}),
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
		message: `This machine was signed out from the console, so the ${name} sign-in was not opened and nothing was saved. Sign this machine back in under Preferences > Machine sign-ins, then press Connect ${name}.`
	}),
	// The seller has not agreed to the seller-device notice for this
	// marketplace, so the application refused in front of the password.
	// Worded once here; the desktop's direct answer and the phone's return
	// leg both read it through `connectConsentRequired`.
	consent: (name) => ({
		tone: 'error',
		message: `${name} needs your permission first, so the sign-in was not opened and nothing was saved. Grant it under Account > Permissions, then press Connect ${name}.`
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
	captured: {
		tone: 'info',
		message: 'Your marketplace sign-in is saved on this device.'
	},
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
			'This machine was signed out from the console, so your marketplace sign-in was not opened and nothing was saved. Sign this machine back in under Preferences > Machine sign-ins, then press Connect on the card.'
	},
	consent: {
		tone: 'error',
		message:
			'That marketplace needs your permission first, so your marketplace sign-in was not opened and nothing was saved. Grant it under Account > Permissions, then press Connect on the card.'
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

/** The consent refusal, for a computer's Connect that answered on this page.
 *  The same sentence the phone's return leg carries, for the reason
 *  `connectSignedOut` gives. */
export function connectConsentRequired(marketplace: Marketplace): ConnectReturn {
	return CONNECT_SAID.consent(CARD_NAME[marketplace]);
}

/** Whether a string off the address is one of the verdicts we word.
 *
 * Read off `CONNECT_SAID`'s own keys rather than listed a second time, so the
 * set that is recognised and the set that has a sentence cannot come apart. */
function isVerdictCode(code: string): code is ConnectVerdictCode {
	return Object.hasOwn(CONNECT_SAID, code);
}

/** What one card can be mid-flight at. Three acts now, and the sign-out is
 *  its own: signing this machine out of a marketplace is not the account-level
 *  disconnect, so a card doing one must not report the other. */
export type BusyAt = 'action' | 'disconnect' | 'signout';

/** Which card is mid-flight, and at what. A record rather than one slot,
 *  because the cards are independent: two marketplaces are two logins on two
 *  windows, and a seller starting the second must not make the first look
 *  idle while its own login window is still open. */
export type Busy = Partial<Record<Marketplace, BusyAt>>;

/** Start or end one card's busy state, leaving every other card's alone.
 *
 * A new record rather than a mutation, so a caller holding the old one cannot
 * observe a half-applied change. */
export function withBusy(busy: Busy, marketplace: Marketplace, at: BusyAt | null): Busy {
	const next: Busy = { ...busy };
	if (at === null) {
		delete next[marketplace];
	} else {
		next[marketplace] = at;
	}
	return next;
}

/** What this one card is doing, or undefined where it is idle. */
export function busyAt(busy: Busy, marketplace: Marketplace): BusyAt | undefined {
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
export function headerAction(host: ConnectHost): {
	label: string;
	href: string;
} {
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
	{ state: 'read'; row: MarketplaceRow } | { state: 'pending' } | { state: 'failed' };

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
export function liveFace(
	held: LiveRead,
	host: ConnectHost,
	local: LocalSessionOutcome | null
): LiveFace {
	switch (held.state) {
		case 'read':
			return {
				status: {
					tone: pillTone(held.row),
					label: SIGN_IN_LABEL[held.row.signIn.state]
				},
				handle: held.row.signIn.accountLabel,
				body: held.row.signIn.line,
				action: footerAction(held.row, host, local)
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

/** How a card states what THIS machine holds, which the status pill above it
 *  cannot: that one is the organisation's, lifted from whatever machine last
 *  reported a login, and a seller standing at a second computer read it as a
 *  statement about the computer in front of them. */
export interface HereFace {
	tone: Tone;
	label: string;
	line: string;
}

/**
 * What this machine says about its own session for one marketplace, or null
 * where there is nothing honest to say.
 *
 * Null on three answers, not one. A browser has no session and no command to
 * ask with; an application too old to answer `session_status` answers
 * `unavailable`; and a read still in flight is null here. Rendering any of
 * those as "not on this device" would be the same defect the pill above had,
 * one line further down, so they say nothing at all instead.
 *
 * A refusal does speak, and carries the application's own words: it is the one
 * unknown a seller can act on.
 */
export function hereFace(
	local: LocalSessionOutcome | null,
	marketplace: Marketplace
): HereFace | null {
	if (local === null || local.kind === 'unavailable') {
		return null;
	}
	const name = CARD_NAME[marketplace];
	if (local.kind === 'refused') {
		return { tone: 'warn', label: 'This device not known', line: local.detail };
	}
	return local.connected
		? {
				tone: 'ok',
				label: 'Signed in on this device',
				line: `The ${name} login is on this machine.`
			}
		: {
				tone: 'soon',
				label: 'Not on this device',
				line: `This machine holds no ${name} login.`
			};
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
