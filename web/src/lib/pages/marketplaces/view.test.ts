import { describe, expect, it } from 'vitest';
import { SIGN_IN_LABEL } from '$lib/devices-view';
import type { MarketplaceRow, MarketplaceSignIn, SignInState } from '$lib/devices-view';
import { APP_CANNOT_FORGET, type LocalSessionOutcome, type SessionOutcome } from '$lib/desktop';
import type { Marketplace } from '$lib/generated/vocab';
import { osLabel, sessionLabel, sessionTone, sessionWords } from '../account/machines';
import {
	CARD_NAME,
	CONNECT_VERDICT_CODES,
	DOWNLOADS_ANCHOR,
	MACHINES_ANCHOR,
	TRANSPORT_BADGE,
	carrying,
	busyAt,
	deviceBranchInTileOrder,
	disconnectAsk,
	disconnectLabel,
	disconnectPrompt,
	disconnectSay,
	disconnectable,
	footerAction,
	headerAction,
	heldHere,
	hereFace,
	hostOf,
	liveFace,
	pillTone,
	connectReturn,
	signOutHereAsk,
	signOutHereLabel,
	signOutHereSay,
	signsInPlace,
	transportLine,
	withBusy
} from './view';

const ANDROID =
	'Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Mobile Safari/537.36';
const WINDOWS =
	'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36';

function row(
	state: SignInState,
	tone: MarketplaceSignIn['tone'],
	marketplace: Marketplace = 'Tpt'
): MarketplaceRow {
	return {
		marketplace,
		transport: marketplace === 'Etsy' ? 'OfficialApi' : 'SellerDevice',
		signIn: {
			marketplace,
			state,
			device: null,
			accountLabel: null,
			line: 'x',
			tone
		},
		connection: null,
		quiet: false,
		wipeOutstandingOn: []
	};
}

describe('the card status pill', () => {
	it('greys a state with nothing to report and ambers one that is failing', () => {
		// The design specification assigns a pending verification to grey and a
		// failing one to amber; the older device view spells those `mut` and
		// `run`, and `run` is the badge's accent tone, which this page never uses.
		expect(pillTone(row('unverified', 'mut'))).toBe('soon');
		expect(pillTone(row('unverified', 'run'))).toBe('warn');
	});

	it('carries the two plain states straight through', () => {
		expect(pillTone(row('signed_in', 'ok'))).toBe('ok');
		expect(pillTone(row('needs_signin', 'bad'))).toBe('bad');
	});
});

describe('whether a marketplace is carrying work', () => {
	it('counts a login held on a machine and a connection we serve', () => {
		expect(carrying(row('signed_in', 'ok'))).toBe(true);
		expect(carrying(row('served_here', 'ok', 'Etsy'))).toBe(true);
	});

	it('counts nothing else, including a seller with no machine at all', () => {
		for (const state of [
			'needs_signin',
			'unverified',
			'no_account',
			'no_device',
			'all_signed_out'
		] as const) {
			expect(carrying(row(state, 'bad')), state).toBe(false);
		}
	});
});

describe('which host the console is', () => {
	it('is a browser wherever there is no invoker, which is every web page', () => {
		expect(hostOf(null)).toBe('browser');
		expect(hostOf(undefined)).toBe('browser');
	});

	// This inverted with the navigate-and-return path, and the inversion is what
	// removed the second parameter. A phone used to read the browser copy
	// because `connect_marketplace` built a second window unconditionally and
	// Tauri's mobile surface is one Activity, so the button would have done
	// nothing. The command now navigates the one webview instead, and a phone
	// that can hold a login is a machine like any other — so no platform is
	// taken back out, and the user agent is no longer part of this question.
	it('is the app wherever an invoker is, on any platform', () => {
		expect(hostOf(() => undefined)).toBe('app');
	});
});

describe('whether the sign-in replaces this page', () => {
	// The one thing the two surfaces do differently, and the reason it is a
	// separate question from `hostOf`: what a phone can DO with a login is what
	// a computer can, and the only difference is that its sign-in has nowhere
	// else to happen.
	it('is true in the app on a phone and false everywhere else', () => {
		expect(signsInPlace(() => undefined, ANDROID)).toBe(true);
		expect(signsInPlace(() => undefined, WINDOWS)).toBe(false);
		expect(signsInPlace(() => undefined, null)).toBe(false);
	});

	it('is false in a browser on a phone, which opens no sign-in at all', () => {
		expect(signsInPlace(null, ANDROID)).toBe(false);
		expect(signsInPlace(undefined, ANDROID)).toBe(false);
	});
});

describe('what a returning sign-in tells the seller', () => {
	const said = (query: string) => connectReturn(new URLSearchParams(query));

	// Total over every verdict `ConnectVerdict` carries, because this is the
	// whole channel: the page that pressed Connect was unloaded by the
	// navigation to the marketplace, so a verdict with no sentence here is a
	// seller returned to the console with nothing said at all.
	it('has a sentence for every verdict the application sends', () => {
		for (const verdict of CONNECT_VERDICT_CODES) {
			const answer = said(`connect=${verdict}&marketplace=Tpt`);
			expect(answer, verdict).not.toBeNull();
			expect(answer?.message, verdict).toContain('TPT');
		}
	});

	it('is the only good news on the success verdict', () => {
		expect(said('connect=captured&marketplace=Tes')?.tone).toBe('info');
		for (const verdict of CONNECT_VERDICT_CODES.filter((code) => code !== 'captured')) {
			expect(said(`connect=${verdict}&marketplace=Tes`)?.tone, verdict).toBe('error');
		}
	});

	// Every failure says what became of the sign-in, and every failure a press
	// can clear says which press. `bound_elsewhere` is the one that cannot be:
	// the shop belongs to another account, so pressing Connect again signs in
	// to the same shop and is refused again, and naming the button there would
	// send the seller round a loop instead of to a person.
	it('says what was not saved, and how to try again where trying again works', () => {
		for (const verdict of CONNECT_VERDICT_CODES.filter((code) => code !== 'captured')) {
			const answer = said(`connect=${verdict}&marketplace=Tpt`);
			expect(answer?.message, verdict).toContain('saved');
			if (verdict !== 'bound_elsewhere') {
				expect(answer?.message, verdict).toContain('Connect TPT');
			}
		}
	});

	// The split these two codes exist for. They were one code, `refused`, worded
	// only as the first: a seller who signed this phone out from a laptop
	// part-way through a TPT sign-in completed it, had the capture wiped by the
	// check-in that learned of the revocation, and was told the sign-in could
	// not be opened — the opposite of what happened, naming nothing they could
	// act on. Each must now say its own thing and neither may say the other's.
	it('tells a sign-in that never opened apart from one that was not saved', () => {
		const refused = said('connect=refused&marketplace=Tpt')?.message ?? '';
		const notkept = said('connect=notkept&marketplace=Tpt')?.message ?? '';
		expect(refused).toContain('did not open');
		expect(refused).not.toContain('signed out');
		expect(notkept).toContain('could not be saved');
		expect(notkept).toContain('signed out of Teachouse');
		expect(notkept).not.toContain('did not open');
	});

	// Reading the address is only safe while an address nobody wrote says
	// nothing. A code from a newer application, a parameter a seller typed, and
	// an ordinary visit must all be indistinguishable here.
	it('says nothing at all about an address it does not recognise', () => {
		expect(said('')).toBeNull();
		expect(said('connect=')).toBeNull();
		expect(said('connect=something-else&marketplace=Tpt')).toBeNull();
		expect(said('marketplace=Tpt')).toBeNull();
	});

	// A marketplace name we do not know is not a reason to withhold the
	// sentence: the verdict is still true. What it must not do is substitute a
	// stand-in into a sentence shaped around a name, which produced "marketplace
	// is connected on this device." and "Press Connect marketplace to try
	// again."
	it('still speaks when it cannot name the marketplace, and speaks English', () => {
		for (const code of CONNECT_VERDICT_CODES) {
			const answer = said(`connect=${code}&marketplace=Somewhere`);
			expect(answer, code).not.toBeNull();
			const message = answer?.message ?? '';
			expect(message, code).toMatch(/^[A-Z]/);
			expect(message, code).not.toContain('Connect marketplace');
			expect(message, code).not.toContain('The marketplace sign-in');
		}
		expect(said('connect=abandoned&marketplace=Somewhere')?.message).toContain(
			'Your marketplace sign-in did not finish'
		);
		expect(said('connect=captured')?.message).toContain('Your marketplace sign-in is saved');
	});
});

const HERE: LocalSessionOutcome = { kind: 'known', connected: true };
const NOT_HERE: LocalSessionOutcome = { kind: 'known', connected: false };
const NO_LOCAL_ANSWER: readonly (LocalSessionOutcome | null)[] = [
	null,
	{ kind: 'unavailable' },
	{ kind: 'refused', detail: 'The application would not say.' }
];

describe('the card footer action', () => {
	// The defect this covers: the action was read off `carrying`, which is true
	// as soon as ANY machine reports a login, so a seller at a second computer
	// was shown "Open TPT" and the only other control disconnected the whole
	// account. Signing in on a second device meant taking the first one's
	// login away.
	it('offers a local login even when another device holds the marketplace session', () => {
		const action = footerAction(row('signed_in', 'ok'), 'app', NOT_HERE);
		expect(action).toMatchObject({ kind: 'command', marketplace: 'Tpt' });
	});

	it('renews a saved local login without requiring a sign-out first', () => {
		expect(footerAction(row('signed_in', 'ok'), 'app', HERE)).toMatchObject({
			kind: 'command',
			marketplace: 'Tpt'
		});
	});

	// Absent, refused and not-yet-answered are none of them "this machine has
	// no login": offering a sign-in on any of them would be acting on a fact we
	// do not have, which is the defect this change exists to stop making one
	// layer up.
	it('points at the machine that reports a login while this one has not answered', () => {
		for (const local of NO_LOCAL_ANSWER) {
			expect(footerAction(row('signed_in', 'ok'), 'app', local)).toEqual({
				kind: 'link',
				label: 'Open TPT',
				href: MACHINES_ANCHOR
			});
		}
	});

	it('does not offer a new login while this device has not answered', () => {
		for (const state of ['no_device', 'all_signed_out', 'needs_signin'] as const) {
			for (const local of NO_LOCAL_ANSWER) {
				expect(footerAction(row(state, 'bad'), 'app', local)).toMatchObject({
					kind: 'link',
					href: MACHINES_ANCHOR
				});
			}
		}
	});

	// The defect this replaces: the console's only Connect was a link to the
	// downloads section, and the app served the same console, so a seller who
	// had installed it was sent to install it again. There was no screen
	// anywhere that opened a marketplace login.
	it('connects here when this console is the app, for both device-branch marketplaces', () => {
		for (const marketplace of ['Tpt', 'Tes'] as const) {
			expect(footerAction(row('no_device', 'bad', marketplace), 'app', NOT_HERE)).toEqual({
				kind: 'command',
				label: `Connect ${CARD_NAME[marketplace]}`,
				marketplace
			});
		}
	});

	it('names the app rather than offering a dead button when it cannot connect', () => {
		for (const marketplace of ['Tpt', 'Tes'] as const) {
			const action = footerAction(row('no_device', 'bad', marketplace), 'browser', null);
			expect(action.kind).toBe('link');
			expect(action).toEqual({
				kind: 'link',
				label: `Connect ${CARD_NAME[marketplace]} from the Teachouse app on your computer or phone`,
				href: DOWNLOADS_ANCHOR
			});
		}
	});

	// A browser holds no marketplace session and has no command to open one, so
	// a local answer there cannot turn its arm into an act it is unable to
	// perform. `sessionStatusHere` answers `unavailable` in a browser; a
	// `known` answer is only reachable by a caller passing one, and this is the
	// assertion that a caller doing so gets no dead button.
	it('never offers a browser a local sign-in, whatever the local answer says', () => {
		for (const local of [NOT_HERE, HERE, ...NO_LOCAL_ANSWER]) {
			expect(footerAction(row('no_device', 'bad'), 'browser', local).kind).toBe('link');
			expect(footerAction(row('signed_in', 'ok'), 'browser', local).kind).toBe('link');
		}
	});

	it('never leaves for the marketplace itself, on either host', () => {
		for (const host of ['app', 'browser'] as const) {
			for (const state of ['signed_in', 'no_device', 'needs_signin'] as const) {
				for (const local of [HERE, NOT_HERE, ...NO_LOCAL_ANSWER]) {
					const action = footerAction(row(state, 'bad'), host, local);
					if (action.kind === 'link') {
						expect(new URL(action.href, 'https://teachouse.example').origin, `${host}/${state}`)
							.toBe('https://teachouse.example');
					}
				}
			}
		}
	});
});

describe('what this machine says about its own session', () => {
	// The pill above this line is the organisation's, lifted from whatever
	// machine last reported a login. A seller at a second computer read it as a
	// statement about the computer in front of them, so the two facts are now
	// stated separately and have to differ.
	it('tells a login held here apart from one held elsewhere', () => {
		const held = hereFace(HERE, 'Tpt');
		const absent = hereFace(NOT_HERE, 'Tpt');
		expect(held?.label).not.toBe(absent?.label);
		expect(held?.tone).toBe('ok');
		expect(absent?.tone).not.toBe('bad');
		expect(held?.line).toContain('TPT');
		expect(absent?.line).toContain('TPT');
	});

	it('says nothing at all where nothing may be claimed', () => {
		// A read in flight and an application too old to answer are not a
		// machine with no login. Saying "not on this device" on either would be
		// the same defect the status pill had, one line further down.
		expect(hereFace(null, 'Tpt')).toBeNull();
		expect(hereFace({ kind: 'unavailable' }, 'Tpt')).toBeNull();
	});

	it('passes a refusal through, which is the one unknown a seller can act on', () => {
		const refused = hereFace({ kind: 'refused', detail: 'Permission was not granted.' }, 'Tes');
		expect(refused?.line).toBe('Permission was not granted.');
		expect(refused?.label).not.toBe(hereFace(NOT_HERE, 'Tes')?.label);
	});

	it('claims a login here only on a plain yes', () => {
		// What the local sign-out control is offered on. Offering it on an
		// unanswered read would put a destructive button on a card we know
		// nothing about.
		expect(heldHere(HERE)).toBe(true);
		expect(heldHere(NOT_HERE)).toBe(false);
		for (const local of NO_LOCAL_ANSWER) {
			expect(heldHere(local), String(local?.kind)).toBe(false);
		}
	});
});

describe('disconnecting a marketplace from the account', () => {
	it('is offered only where a connection stands', () => {
		expect(disconnectable({ state: 'linked' })).toBe(true);
		expect(disconnectable({ state: 'needs_reauth' })).toBe(true);
		expect(disconnectable({ state: 'linking' })).toBe(true);
	});

	it('is not offered for a marketplace with no connection at all', () => {
		expect(disconnectable(null)).toBe(false);
		expect(disconnectable(undefined)).toBe(false);
	});

	it('is not offered where there is nothing left to disconnect', () => {
		// `unlinked` is what a disconnect writes, so offering it again promises
		// an act the server answers with a count of zero. `revoked` is terminal
		// and nothing moves it.
		expect(disconnectable({ state: 'unlinked' })).toBe(false);
		expect(disconnectable({ state: 'revoked' })).toBe(false);
	});

	// Two controls now reach two different things — one machine's own store,
	// and the organisation's connection — so a seller pressing either has to be
	// able to tell which they pressed.
	it('labels the two ways out apart, and names the marketplace in both', () => {
		for (const marketplace of ['Tpt', 'Tes'] as const) {
			const name = CARD_NAME[marketplace];
			expect(disconnectLabel(marketplace)).not.toBe(signOutHereLabel(marketplace));
			expect(disconnectLabel(marketplace)).toContain(name);
			expect(signOutHereLabel(marketplace)).toContain(name);
		}
	});

	it('promises no removal from the marketplace and no loss of listings', () => {
		for (const heldOnAMachine of [true, false]) {
			const prompt = disconnectPrompt('Tpt', heldOnAMachine);
			expect(prompt).toContain('Nothing is removed from TPT itself');
			expect(prompt).toContain('listings stay here');
			expect(prompt).toContain('Scheduled work for TPT stops');
		}
	});

	// R4, and the half a bare "the login remains there" would leave out. A
	// machine that keeps checking in while holding that login lifts the
	// connection back to linked on its next beat, because that is what a
	// check-in does. A seller told only that the login remains would watch the
	// marketplace reconnect itself with no account of why.
	it('says the login stays on its machine and that the machine will reconnect it', () => {
		const prompt = disconnectPrompt('Tes', true);
		expect(prompt).toContain('still on the machine');
		expect(prompt).toContain('reconnects TES by itself');
		expect(prompt).toContain('sign the machine out');
	});

	it('claims no reconnection where no machine holds the login', () => {
		const prompt = disconnectPrompt('Tes', false);
		expect(prompt).not.toContain('reconnects TES by itself');
		expect(prompt).toContain('Scheduled work for TES stops');
	});

	// The boundary this control has, stated rather than papered over: it asks
	// the control plane to unlink and reaches no machine's store. A prompt that
	// claimed a login was removed would be false in the app as well as in a
	// browser, because this control no longer calls `forgetHere` on either.
	it('never claims a machine loses its login', () => {
		for (const heldOnAMachine of [true, false]) {
			expect(disconnectPrompt('Tpt', heldOnAMachine)).not.toContain('removed from this machine');
		}
	});
});

describe('signing this machine out of a marketplace', () => {
	it('says the login is removed from this machine', () => {
		expect(signOutHereAsk('Tpt')).toContain('removed from this machine');
	});

	// The whole reason the control exists: a seller signing a shared computer
	// out of TPT must not stop the work running on the computer at home, and
	// must not be told they have.
	it('promises the other machines keep their logins, and stops scheduled work nowhere', () => {
		const prompt = signOutHereAsk('Tes');
		expect(prompt).toContain('other machines keep their own TES logins');
		expect(prompt).not.toContain('Scheduled work for TES stops');
	});

	// The failure this prevents: a seller disconnects on a phone, presses
	// Connect again, is signed straight back in with no password, and concludes
	// the disconnect did nothing. Our copy of the session IS gone; what is left
	// is the marketplace's own cookie in the WebView's process-global store,
	// which `delete_cookie` cannot touch on Android and which the one lever that
	// could — `clear_all_browsing_data` — would only clear by signing the seller
	// out of Teachouse as well.
	it('warns on a phone that the marketplace keeps its own sign-in', () => {
		const prompt = signOutHereAsk('Tpt', true);
		expect(prompt).toContain("stays in this phone's browser");
		expect(prompt).toContain('may not ask for your password');
	});

	it('says none of that on a computer, where the login window is our own', () => {
		const prompt = signOutHereAsk('Tpt');
		expect(prompt).not.toContain("this phone's browser");
		expect(prompt).not.toContain('may not ask for your password');
	});
});

describe('what a card asks before disconnecting the account', () => {
	// The regression this covers: the page derived this boolean itself, from the
	// device registry's sign-in state, which is a second computation of the same
	// heartbeat and a third answer to a question the rest of this feature reads
	// one module for. The reconnect warning has to follow `connection.state`,
	// because that is the field `derive_link` lifts and the disconnect route
	// writes.
	it('warns of a reconnect only where a machine is still reporting the login', () => {
		expect(disconnectAsk('Tpt', { state: 'linked' })).toContain('reconnects TPT by itself');
	});

	it('makes no reconnect claim for a connection nothing is reporting', () => {
		for (const state of ['needs_reauth', 'linking', 'unlinked', 'revoked']) {
			expect(disconnectAsk('Tpt', { state }), state).not.toContain('reconnects TPT by itself');
		}
		expect(disconnectAsk('Tpt', null)).not.toContain('reconnects TPT by itself');
	});
});

describe('which card is mid-flight', () => {
	// The defect this replaces: one shared slot. Starting Disconnect on TES while
	// TPT's login window was still open overwrote the slot, so TPT's card stopped
	// saying "Signing in…" and offered its button again while its own mutation
	// was still running.
	it('leaves the first card busy when a second card starts', () => {
		const one = withBusy({}, 'Tpt', 'action');
		const two = withBusy(one, 'Tes', 'disconnect');
		expect(busyAt(two, 'Tpt')).toBe('action');
		expect(busyAt(two, 'Tes')).toBe('disconnect');
	});

	it('ends one card without ending the other', () => {
		const both = withBusy(withBusy({}, 'Tpt', 'action'), 'Tes', 'disconnect');
		const after = withBusy(both, 'Tes', null);
		expect(busyAt(after, 'Tes')).toBeUndefined();
		expect(busyAt(after, 'Tpt')).toBe('action');
	});

	it('reports an untouched card as idle', () => {
		expect(busyAt({}, 'Tpt')).toBeUndefined();
		expect(busyAt(withBusy({}, 'Tes', 'action'), 'Tpt')).toBeUndefined();
	});

	// A new record each time, so a caller holding the old one cannot observe a
	// half-applied change mid-mutation.
	it('leaves the record it was given alone', () => {
		const before = withBusy({}, 'Tpt', 'action');
		withBusy(before, 'Tes', 'disconnect');
		expect(busyAt(before, 'Tes')).toBeUndefined();
	});
});

describe('the page header action', () => {
	it('leads a browser to the app, which is the only way to connect from one', () => {
		expect(headerAction('browser')).toEqual({
			label: 'Connect a marketplace',
			href: DOWNLOADS_ANCHOR
		});
	});

	// In the app the cards below are the route, so a header button pointing at
	// them would be the page's main action doing nothing. What downloads are
	// still for there is the seller's other machine.
	it('offers the app the one thing downloads are still for', () => {
		expect(headerAction('app')).toEqual({
			label: 'Install on another machine',
			href: DOWNLOADS_ANCHOR
		});
	});
});

describe('the transport line D1 requires', () => {
	it('says whose machine, and names the marketplace that gave the permission', () => {
		expect(transportLine('SellerDevice', 'TPT')).toBe('Runs on your own computer.');
		expect(transportLine('OfficialApi', 'Etsy')).toBe(
			'Runs on our servers, with the permission Etsy gave us.'
		);
	});

	it('badges both branches', () => {
		expect(TRANSPORT_BADGE.SellerDevice).toBe('On your device');
		expect(TRANSPORT_BADGE.OfficialApi).toBe('On our servers');
	});
});

describe('what a live tile says when the read has not landed or failed', () => {
	it('never says a marketplace is disconnected on a read we could not make', () => {
		// The failure this replaces: on a failed read the two tiles vanished and
		// the grid began at Etsy, on a page titled "Every marketplace you sell on".
		for (const state of ['pending', 'failed'] as const) {
			const face = liveFace({ state }, 'app', null);
			expect(face.status.label, state).not.toBe(SIGN_IN_LABEL.needs_signin);
			expect(face.status.tone, state).toBe('soon');
		}
	});

	it('offers no action, because neither Open nor Connect is known to be right', () => {
		for (const local of [null, HERE, NOT_HERE]) {
			expect(liveFace({ state: 'pending' }, 'app', local).action).toBeUndefined();
			expect(liveFace({ state: 'failed' }, 'app', local).action).toBeUndefined();
		}
	});

	it('shows no account handle it has not read', () => {
		expect(liveFace({ state: 'pending' }, 'app', null).handle).toBeNull();
		expect(liveFace({ state: 'failed' }, 'app', null).handle).toBeNull();
	});

	it('tells a pending read apart from a failed one, which a seller acts on differently', () => {
		expect(liveFace({ state: 'pending' }, 'app', null).body).not.toBe(
			liveFace({ state: 'failed' }, 'app', null).body
		);
		expect(liveFace({ state: 'failed' }, 'app', null).body).toContain('could not read');
	});

	it('carries the server’s marketplace standing through once the read lands', () => {
		const held = row('signed_in', 'ok', 'Tes');
		const face = liveFace({ state: 'read', row: held }, 'app', HERE);
		expect(face.status).toEqual({ tone: 'ok', label: SIGN_IN_LABEL.signed_in });
		expect(face.body).toBe(held.signIn.line);
	});

	// The acceptance case, through the function the page actually calls: a
	// marketplace the server reports signed in on some machine, read on a
	// machine that holds no login for it, offers the sign-in here.
	it('carries this machine’s own answer through to the action', () => {
		const held = row('signed_in', 'ok', 'Tpt');
		expect(liveFace({ state: 'read', row: held }, 'app', NOT_HERE).action).toEqual({
			kind: 'command',
			label: 'Connect TPT',
			marketplace: 'Tpt'
		});
	});
});

describe('the marketplaces a copyright declaration is made for', () => {
	const TILED: readonly Marketplace[] = ['Tes', 'Tpt'];

	it('lists them in the order the grid tiles them, not the device view order', () => {
		// `$lib/devices-view` ranks TPT first; the grid leads with TES, and one
		// screen must not list one pair two ways.
		const held = [row('signed_in', 'ok', 'Tpt'), row('signed_in', 'ok', 'Tes')];
		expect(deviceBranchInTileOrder(held, TILED).map((entry) => entry.marketplace)).toEqual([
			'Tes',
			'Tpt'
		]);
	});

	it('drops the marketplaces that run on our own servers', () => {
		const held = [row('served_here', 'ok', 'Etsy'), row('signed_in', 'ok', 'Tpt')];
		expect(deviceBranchInTileOrder(held, TILED).map((entry) => entry.marketplace)).toEqual(['Tpt']);
	});

	it('keeps a device-branch marketplace this page does not tile, after the ones it does', () => {
		const held = [row('signed_in', 'ok', 'Tpt'), row('signed_in', 'ok', 'Tes')];
		expect(deviceBranchInTileOrder(held, ['Tes']).map((entry) => entry.marketplace)).toEqual([
			'Tes',
			'Tpt'
		]);
	});

	it('leaves the rows it was given alone', () => {
		const held = [row('signed_in', 'ok', 'Tpt'), row('signed_in', 'ok', 'Tes')];
		deviceBranchInTileOrder(held, TILED);
		expect(held.map((entry) => entry.marketplace)).toEqual(['Tpt', 'Tes']);
	});
});

describe('the card heading name', () => {
	it('writes the two live marketplaces in capitals, as they write themselves', () => {
		expect(CARD_NAME.Tes).toBe('TES');
		expect(CARD_NAME.Tpt).toBe('TPT');
	});
});

describe('one machine and the logins on it', () => {
	it('names an operating system as its vendor writes it', () => {
		expect(osLabel({ os: 'macos' } as never)).toBe('macOS');
		expect(osLabel({ os: 'WINDOWS' } as never)).toBe('Windows');
	});

	it('renders an unknown platform as the device reported it, never as a blank', () => {
		expect(osLabel({ os: 'freebsd' } as never)).toBe('freebsd');
	});

	it('tones a held login by what it is doing', () => {
		expect(sessionTone('connected')).toBe('ok');
		expect(sessionTone('signed_out')).toBe('soon');
		expect(sessionTone('wiped')).toBe('bad');
	});

	it('names the account a marketplace shows, where the device reported one', () => {
		expect(sessionWords({ status: 'connected', account_label: 'Ms Kahu' } as never)).toBe(
			'signed in as Ms Kahu'
		);
		expect(sessionWords({ status: 'connected', account_label: null } as never)).toBe('signed in');
	});

	it('never puts a stored identifier on screen, which the pill would uppercase', () => {
		// The defect this replaces: the pill rendered `signed_out` verbatim and
		// uppercased it, so a seller read SIGNED_OUT, underscore and all.
		for (const status of ['connected', 'signed_out', 'wiped'] as const) {
			expect(sessionLabel(status), status).not.toContain('_');
			expect(sessionLabel(status), status).not.toBe(status);
		}
	});

	it('says a wiped login is the sign-out working, not something failing', () => {
		expect(sessionWords({ status: 'wiped', account_label: null } as never)).toBe(
			'forgotten when this device was signed out'
		);
	});
});

describe('what a finished account disconnect says', () => {
	it('treats a refusal as an error and claims nothing the server did not do', () => {
		// The specific regression this guards: the success wording reaching a
		// seller whose connection still stands.
		const say = disconnectSay('Tes', { kind: 'refused' });
		expect(say.tone).toBe('error');
		expect(say.message).not.toContain('is disconnected');
		expect(say.message).not.toContain('was already disconnected');
	});

	it('tells a seller nothing was standing when the server moved none', () => {
		expect(disconnectSay('Etsy', { kind: 'moved', moved: 0 })).toEqual({
			tone: 'info',
			message: 'Etsy was already disconnected.'
		});
	});

	it('confirms a disconnect the server performed', () => {
		const say = disconnectSay('Tes', { kind: 'moved', moved: 1 });
		expect(say.tone).toBe('info');
		expect(say.message).toContain('TES is disconnected from your account');
	});

	// The boundary: this control asks the control plane and reaches no
	// machine's store, so no answer it gives may report a login removed. The
	// sentence it replaced said exactly that, because the one control did both.
	it('reports no machine’s login, on any answer', () => {
		for (const server of [
			{ kind: 'refused' } as const,
			{ kind: 'moved', moved: 0 } as const,
			{ kind: 'moved', moved: 2 } as const
		]) {
			const say = disconnectSay('Tpt', server);
			expect(say.message, server.kind).not.toContain('this machine');
			expect(say.message.length, server.kind).toBeGreaterThan(0);
		}
	});
});

describe('what a finished local sign-out says', () => {
	const REFUSAL = 'The window would not close.';
	const ANSWERS: readonly SessionOutcome[] = [
		{ kind: 'done' },
		{ kind: 'opening' },
		{ kind: 'signedOut' },
		{ kind: 'refused', detail: REFUSAL },
		{ kind: 'consentRequired' },
		{ kind: 'boundElsewhere' },
		{ kind: 'unsupported' },
		{ kind: 'unavailable' }
	];

	it('confirms the login is gone from this machine', () => {
		const say = signOutHereSay('Tpt', { kind: 'done' });
		expect(say.tone).toBe('info');
		expect(say.message).toContain('removed from this machine');
	});

	it('reports an app that cannot forget', () => {
		expect(signOutHereSay('Tpt', { kind: 'unsupported' })).toEqual({
			tone: 'error',
			message: APP_CANNOT_FORGET
		});
	});

	it('passes the machine’s own refusal through', () => {
		expect(signOutHereSay('Tpt', { kind: 'refused', detail: REFUSAL })).toEqual({
			tone: 'error',
			message: REFUSAL
		});
	});

	// The act this control does not perform, and must never claim to: the
	// account's connection stands until a machine stops reporting the login.
	// A seller told their marketplace was disconnected would stop looking for
	// the scheduled work that is still running.
	it('never says the marketplace was disconnected', () => {
		for (const forgotten of ANSWERS) {
			const say = signOutHereSay('Tes', forgotten);
			expect(say.message, forgotten.kind).not.toContain('is disconnected');
			expect(say.message, forgotten.kind).not.toContain('was already disconnected');
		}
	});

	it('answers every outcome the application can give', () => {
		// Totality, checked rather than asserted: all six of `SessionOutcome`'s
		// kinds. The sentence this replaced defaulted three of them into its
		// confirming arm, so `opening` and `signedOut` read as a success.
		for (const forgotten of ANSWERS) {
			const say = signOutHereSay('Tpt', forgotten);
			expect(say.message.length, forgotten.kind).toBeGreaterThan(0);
			expect(['info', 'error'], forgotten.kind).toContain(say.tone);
		}
		expect(signOutHereSay('Tpt', { kind: 'done' }).tone).toBe('info');
		for (const forgotten of ANSWERS.filter((answer) => answer.kind !== 'done')) {
			expect(signOutHereSay('Tpt', forgotten).tone, forgotten.kind).toBe('error');
		}
	});
});

