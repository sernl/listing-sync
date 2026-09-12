import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { SIGN_IN_LABEL } from '$lib/devices-view';
import type { MarketplaceRow, MarketplaceSignIn, SignInState } from '$lib/devices-view';
import { APP_CANNOT_FORGET, type SessionOutcome } from '$lib/desktop';
import type { Marketplace } from '$lib/generated/vocab';
import { osLabel, sessionLabel, sessionTone, sessionWords } from './machines';
import {
	CARD_NAME,
	CONNECT_RETURN_MARKETPLACE_PARAM,
	CONNECT_RETURN_PARAM,
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
	hostOf,
	liveFace,
	pillTone,
	connectReturn,
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
		signIn: { marketplace, state, device: null, accountLabel: null, line: 'x', tone },
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
		for (const state of ['needs_signin', 'unverified', 'no_account', 'no_device', 'all_signed_out'] as const) {
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

	// Every failure says what became of the sign-in and what to press next.
	it('says what was not saved and how to try again on every failure', () => {
		for (const verdict of CONNECT_VERDICT_CODES.filter((code) => code !== 'captured')) {
			const answer = said(`connect=${verdict}&marketplace=Tpt`);
			expect(answer?.message, verdict).toContain('saved');
			expect(answer?.message, verdict).toContain('Connect TPT');
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

describe('the card footer action', () => {
	it('opens onto the machine holding the login when one is carrying work', () => {
		for (const host of ['app', 'browser'] as const) {
			expect(footerAction(row('signed_in', 'ok'), host)).toEqual({
				kind: 'link',
				label: 'Open TPT',
				href: MACHINES_ANCHOR
			});
		}
	});

	// The defect this replaces: the console's only Connect was a link to the
	// downloads section, and the app served the same console, so a seller who
	// had installed it was sent to install it again. There was no screen
	// anywhere that opened a marketplace login.
	it('connects here when this console is the app, for both device-branch marketplaces', () => {
		for (const marketplace of ['Tpt', 'Tes'] as const) {
			expect(footerAction(row('no_device', 'bad', marketplace), 'app')).toEqual({
				kind: 'command',
				label: `Connect ${CARD_NAME[marketplace]}`,
				marketplace
			});
		}
	});

	it('names the app rather than offering a dead button when it cannot connect', () => {
		for (const marketplace of ['Tpt', 'Tes'] as const) {
			const action = footerAction(row('no_device', 'bad', marketplace), 'browser');
			expect(action.kind).toBe('link');
			expect(action).toEqual({
				kind: 'link',
				label: `Connect ${CARD_NAME[marketplace]} from the Teachouse app on your computer or phone`,
				href: DOWNLOADS_ANCHOR
			});
		}
	});

	it('never leaves for the marketplace itself, on either host', () => {
		for (const host of ['app', 'browser'] as const) {
			for (const state of ['signed_in', 'no_device', 'needs_signin'] as const) {
				const action = footerAction(row(state, 'bad'), host);
				if (action.kind === 'link') {
					expect(action.href.startsWith('#'), `${host}/${state}`).toBe(true);
				}
			}
		}
	});
});

describe('disconnecting a marketplace', () => {
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

	it('names the marketplace it removes', () => {
		expect(disconnectLabel('Tpt')).toBe('Disconnect TPT');
		expect(disconnectLabel('Tes')).toBe('Disconnect TES');
	});

	it('promises no removal from the marketplace and no loss of listings', () => {
		for (const host of ['app', 'browser'] as const) {
			const prompt = disconnectPrompt('Tpt', host, true);
			expect(prompt, host).toContain('Nothing is removed from TPT itself');
			expect(prompt, host).toContain('listings stay here');
			expect(prompt, host).toContain('Scheduled work for TPT stops');
		}
	});

	it('says the login is removed from this machine when the app can remove it', () => {
		const prompt = disconnectPrompt('Tpt', 'app', true);
		expect(prompt).toContain('removed from this machine');
		// Never the browser's account of a login it cannot reach: this host can.
		expect(prompt).not.toContain('still on the machine');
	});

	// R4, and the half a bare "the login remains there" would leave out. A
	// machine that keeps checking in while holding that login lifts the
	// connection back to linked on its next beat, because that is what a
	// check-in does. A seller told only that the login remains would watch the
	// marketplace reconnect itself with no account of why.
	it('says a browser cannot remove the login and that the machine will reconnect it', () => {
		const prompt = disconnectPrompt('Tes', 'browser', true);
		expect(prompt).toContain('still on the machine');
		expect(prompt).toContain('reconnects TES by itself');
		expect(prompt).toContain('sign the machine out');
	});

	it('claims no reconnection where no machine holds the login', () => {
		const prompt = disconnectPrompt('Tes', 'browser', false);
		expect(prompt).not.toContain('reconnects TES by itself');
		expect(prompt).toContain('Scheduled work for TES stops');
	});

	// The failure this prevents: a seller disconnects on a phone, presses
	// Connect again, is signed straight back in with no password, and concludes
	// the disconnect did nothing. Our copy of the session IS gone; what is left
	// is the marketplace's own cookie in the WebView's process-global store,
	// which `delete_cookie` cannot touch on Android and which the one lever that
	// could — `clear_all_browsing_data` — would only clear by signing the seller
	// out of Teachouse as well.
	it('warns on a phone that the marketplace keeps its own sign-in', () => {
		const prompt = disconnectPrompt('Tpt', 'app', true, true);
		expect(prompt).toContain('removed from this machine');
		expect(prompt).toContain("stays in this phone's browser");
		expect(prompt).toContain('may not ask for your password');
	});

	it('says none of that on a computer, where the login window is our own', () => {
		const prompt = disconnectPrompt('Tpt', 'app', true);
		expect(prompt).not.toContain("this phone's browser");
		expect(prompt).not.toContain('may not ask for your password');
	});
});

describe('what a card asks before disconnecting', () => {
	// The regression this covers: the page derived this boolean itself, from the
	// device registry's sign-in state, which is a second computation of the same
	// heartbeat and a third answer to a question the rest of this feature reads
	// one module for. The reconnect warning has to follow `connection.state`,
	// because that is the field `derive_link` lifts and the disconnect route
	// writes.
	it('warns of a reconnect only where a machine is still reporting the login', () => {
		expect(disconnectAsk('Tpt', 'browser', { state: 'linked' })).toContain(
			'reconnects TPT by itself'
		);
	});

	it('makes no reconnect claim for a connection nothing is reporting', () => {
		for (const state of ['needs_reauth', 'linking', 'unlinked', 'revoked']) {
			expect(disconnectAsk('Tpt', 'browser', { state }), state).not.toContain(
				'reconnects TPT by itself'
			);
		}
		expect(disconnectAsk('Tpt', 'browser', null)).not.toContain('reconnects TPT by itself');
	});

	it('says the machine loses the login when the app is the one asking', () => {
		expect(disconnectAsk('Tes', 'app', { state: 'linked' })).toContain(
			'removed from this machine'
		);
	});

	it('carries the phone warning through from the page that knows the surface', () => {
		expect(disconnectAsk('Tes', 'app', { state: 'linked' }, true)).toContain(
			"stays in this phone's browser"
		);
		expect(disconnectAsk('Tes', 'app', { state: 'linked' })).not.toContain(
			"stays in this phone's browser"
		);
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
			const face = liveFace({ state }, 'app');
			expect(face.status.label, state).not.toBe(SIGN_IN_LABEL.needs_signin);
			expect(face.status.tone, state).toBe('soon');
		}
	});

	it('offers no action, because neither Open nor Connect is known to be right', () => {
		expect(liveFace({ state: 'pending' }, 'app').action).toBeUndefined();
		expect(liveFace({ state: 'failed' }, 'app').action).toBeUndefined();
	});

	it('shows no account handle it has not read', () => {
		expect(liveFace({ state: 'pending' }, 'app').handle).toBeNull();
		expect(liveFace({ state: 'failed' }, 'app').handle).toBeNull();
	});

	it('tells a pending read apart from a failed one, which a seller acts on differently', () => {
		expect(liveFace({ state: 'pending' }, 'app').body).not.toBe(
			liveFace({ state: 'failed' }, 'app').body
		);
		expect(liveFace({ state: 'failed' }, 'app').body).toContain('could not read');
	});

	it('carries the row through untouched once the read lands', () => {
		const held = row('signed_in', 'ok', 'Tes');
		const face = liveFace({ state: 'read', row: held }, 'app');
		expect(face.status).toEqual({ tone: 'ok', label: SIGN_IN_LABEL.signed_in });
		expect(face.body).toBe(held.signIn.line);
		expect(face.action).toEqual({ kind: 'link', label: 'Open TES', href: MACHINES_ANCHOR });
	});

	it('carries the host through to the action once the read lands', () => {
		const held = row('no_device', 'bad', 'Tpt');
		expect(liveFace({ state: 'read', row: held }, 'app').action).toEqual({
			kind: 'command',
			label: 'Connect TPT',
			marketplace: 'Tpt'
		});
		expect(liveFace({ state: 'read', row: held }, 'browser').action?.kind).toBe('link');
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
		expect(deviceBranchInTileOrder(held, TILED).map((entry) => entry.marketplace)).toEqual([
			'Tpt'
		]);
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
		expect(
			sessionWords({ status: 'connected', account_label: 'Ms Kahu' } as never)
		).toBe('signed in as Ms Kahu');
		expect(sessionWords({ status: 'connected', account_label: null } as never)).toBe(
			'signed in'
		);
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

describe('what a finished disconnect says', () => {
	const REFUSAL = 'The window would not close.';
	const REFUSED: SessionOutcome = { kind: 'refused', detail: REFUSAL };
	const HALVES: readonly SessionOutcome[] = [
		{ kind: 'done' },
		REFUSED,
		{ kind: 'unsupported' },
		{ kind: 'unavailable' }
	];

	it('names both halves when the machine forgot and the server refused', () => {
		// The case the whole change exists for. A single sentence saying the
		// disconnect failed is false about the half that ran, and false in the
		// direction that matters: the cookies are gone and the leases are not.
		const say = disconnectSay('Tpt', { kind: 'done' }, { kind: 'refused' });
		expect(say.tone).toBe('error');
		expect(say.message).toContain('login has been removed from this machine');
		expect(say.message).toContain('TPT is still connected here');
		expect(say.message).toContain('scheduled work has not stopped');
	});

	it('says only that nothing happened when nothing was forgotten either', () => {
		// A browser has no local login to forget, so a server refusal there did
		// leave everything as it was. Naming the marketplace is the only thing
		// this gains over the sentence it replaces.
		const say = disconnectSay('Tes', { kind: 'unavailable' }, { kind: 'refused' });
		expect(say).toEqual({ tone: 'error', message: 'TES could not be disconnected.' });
	});

	it('treats every server refusal as an error, whatever the machine answered', () => {
		// An implementation that let the device half decide the tone would call
		// a standing connection an `info`.
		for (const forgotten of HALVES) {
			expect(disconnectSay('Tpt', forgotten, { kind: 'refused' }).tone, forgotten.kind).toBe(
				'error'
			);
		}
	});

	it('never claims a disconnect the server refused', () => {
		// The specific regression this guards: the success wording reaching a
		// seller whose connection still stands.
		for (const forgotten of HALVES) {
			const { message } = disconnectSay('Tpt', forgotten, { kind: 'refused' });
			expect(message, forgotten.kind).not.toContain('is disconnected');
			expect(message, forgotten.kind).not.toContain('was already disconnected');
		}
	});

	it('reports an app that cannot forget, once the server half has landed', () => {
		expect(disconnectSay('Tpt', { kind: 'unsupported' }, { kind: 'moved', moved: 1 })).toEqual({
			tone: 'error',
			message: APP_CANNOT_FORGET
		});
	});

	it('passes the machine’s own refusal through', () => {
		expect(disconnectSay('Tpt', REFUSED, { kind: 'moved', moved: 1 })).toEqual({
			tone: 'error',
			message: REFUSAL
		});
	});

	it('tells a seller nothing was standing when the server moved none', () => {
		for (const forgotten of [{ kind: 'done' } as const, { kind: 'unavailable' } as const]) {
			expect(disconnectSay('Etsy', forgotten, { kind: 'moved', moved: 0 })).toEqual({
				tone: 'info',
				message: 'Etsy was already disconnected.'
			});
		}
	});

	it('confirms a disconnect that both halves completed', () => {
		const say = disconnectSay('Tes', { kind: 'done' }, { kind: 'moved', moved: 1 });
		expect(say.tone).toBe('info');
		expect(say.message).toBe('TES is disconnected. Connecting again is the same button.');
	});

	it('answers every pairing of the two halves', () => {
		// Totality, checked rather than asserted: `SessionOutcome`'s four kinds
		// against the server's two. A branch that fell through would return
		// undefined here rather than a sentence.
		const servers = [
			{ kind: 'refused' } as const,
			{ kind: 'moved', moved: 0 } as const,
			{ kind: 'moved', moved: 2 } as const
		];
		for (const forgotten of HALVES) {
			for (const server of servers) {
				const say = disconnectSay('Tpt', forgotten, server);
				expect(say.message.length, `${forgotten.kind}/${server.kind}`).toBeGreaterThan(0);
				expect(['info', 'error'], `${forgotten.kind}/${server.kind}`).toContain(say.tone);
			}
		}
	});
});

describe('the address the application and this page share', () => {
	const connectRs = readFileSync(
		fileURLToPath(
			new URL('../../../../../apps/desktop/src-tauri/src/connect.rs', import.meta.url)
		),
		'utf8'
	);

	// Both parameter names are written twice, once in each language, and nothing
	// but this holds them together. The address is the only channel between the
	// application and this page, because the page that pressed Connect was
	// unloaded by the sign-in it started — so a rename on either side loses
	// every verdict sentence, which is the one failure the return leg exists to
	// prevent, and no other test anywhere goes red.
	it('names the two parameters the way the application spells them', () => {
		expect(connectRs).toContain(`RETURN_PARAM: &str = "${CONNECT_RETURN_PARAM}"`);
		expect(connectRs).toContain(
			`RETURN_MARKETPLACE_PARAM: &str = "${CONNECT_RETURN_MARKETPLACE_PARAM}"`
		);
	});

	// And the verdicts themselves, in both directions. A code the application
	// can send that this page does not word returns a seller to the console in
	// silence; a code worded here that the application never sends is copy
	// nobody will read. `ConnectVerdict::code` is an exhaustive match, so this
	// reads the whole set rather than a sample of it.
	it('words exactly the verdicts the application can send', () => {
		// `[a-z_]+`, not `[a-z]+`: `signed_out` is two words on the wire, and a
		// class that stopped at the underscore matched nothing on that arm —
		// which would have read as the application never sending the code.
		const sent = [...connectRs.matchAll(/Self::[A-Za-z]+ => "([a-z_]+)"/g)].map(([, code]) => code);
		expect(sent.length).toBeGreaterThan(0);
		expect([...sent].sort()).toEqual([...CONNECT_VERDICT_CODES].sort());
		for (const code of sent) {
			expect(
				connectReturn(new URLSearchParams(`connect=${code}&marketplace=Tpt`)),
				code
			).not.toBeNull();
		}
	});
});
