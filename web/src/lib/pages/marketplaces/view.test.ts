import { describe, expect, it } from 'vitest';
import { SIGN_IN_LABEL } from '$lib/devices-view';
import type { MarketplaceRow, MarketplaceSignIn, SignInState } from '$lib/devices-view';
import type { Marketplace } from '$lib/generated/vocab';
import { osLabel, sessionLabel, sessionTone, sessionWords } from './machines';
import {
	CARD_NAME,
	DOWNLOADS_ANCHOR,
	MACHINES_ANCHOR,
	TRANSPORT_BADGE,
	carrying,
	deviceBranchInTileOrder,
	footerAction,
	liveFace,
	pillTone,
	transportLine
} from './view';

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

describe('the card footer action', () => {
	it('opens onto the machine holding the login when one is carrying work', () => {
		expect(footerAction(row('signed_in', 'ok'))).toEqual({
			label: 'Open',
			href: MACHINES_ANCHOR
		});
	});

	it('sends a seller to the app when nothing is connected, never to the marketplace', () => {
		const action = footerAction(row('no_device', 'bad'));
		expect(action).toEqual({ label: 'Connect', href: DOWNLOADS_ANCHOR });
		expect(action.href.startsWith('#')).toBe(true);
	});
});

describe('the transport line D1 requires', () => {
	it('says whose machine, and names the marketplace whose token it is', () => {
		expect(transportLine('SellerDevice', 'TPT')).toBe('Runs on your own device.');
		expect(transportLine('OfficialApi', 'Etsy')).toBe(
			'Runs on our servers, under a token Etsy issued us.'
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
			const face = liveFace({ state });
			expect(face.status.label, state).not.toBe(SIGN_IN_LABEL.needs_signin);
			expect(face.status.tone, state).toBe('soon');
		}
	});

	it('offers no action, because neither Open nor Connect is known to be right', () => {
		expect(liveFace({ state: 'pending' }).action).toBeUndefined();
		expect(liveFace({ state: 'failed' }).action).toBeUndefined();
	});

	it('shows no account handle it has not read', () => {
		expect(liveFace({ state: 'pending' }).handle).toBeNull();
		expect(liveFace({ state: 'failed' }).handle).toBeNull();
	});

	it('tells a pending read apart from a failed one, which a seller acts on differently', () => {
		expect(liveFace({ state: 'pending' }).body).not.toBe(liveFace({ state: 'failed' }).body);
		expect(liveFace({ state: 'failed' }).body).toContain('could not read');
	});

	it('carries the row through untouched once the read lands', () => {
		const held = row('signed_in', 'ok', 'Tes');
		const face = liveFace({ state: 'read', row: held });
		expect(face.status).toEqual({ tone: 'ok', label: SIGN_IN_LABEL.signed_in });
		expect(face.body).toBe(held.signIn.line);
		expect(face.action).toEqual({ label: 'Open', href: MACHINES_ANCHOR });
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
