// What a connection card says, derived from the row the shared device view
// already builds. Pure, so it tests without a component.

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

export interface FooterAction {
	label: string;
	href: string;
}

/**
 * The card's one action.
 *
 * A marketplace that is carrying work opens onto the machine holding its login,
 * which is the only thing there is to look at. One that is not sends the seller
 * to the downloads, because a login for a marketplace on the device branch can
 * only be captured in the app on their own machine — there is no server-side
 * connect for it, and there is no endpoint that links one on the API branch
 * either. Neither action ever leaves for the marketplace's own website.
 */
export function footerAction(row: MarketplaceRow): FooterAction {
	return carrying(row)
		? { label: 'Open', href: MACHINES_ANCHOR }
		: { label: 'Connect', href: DOWNLOADS_ANCHOR };
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

/** How the two live marketplaces write their own names, in capitals, which is
 *  what the card heading shows. `$lib/platforms` spells them with the full
 *  name in brackets, which is right for a row in a list and too long for a
 *  card heading beside a logo. */
export const CARD_NAME: Record<Marketplace, string> = {
	Tes: 'TES',
	Tpt: 'TPT',
	Etsy: 'Etsy'
};

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
	action?: FooterAction;
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
export function liveFace(held: LiveRead): LiveFace {
	switch (held.state) {
		case 'read':
			return {
				status: { tone: pillTone(held.row), label: SIGN_IN_LABEL[held.row.signIn.state] },
				handle: held.row.signIn.accountLabel,
				body: held.row.signIn.line,
				action: footerAction(held.row)
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
				body: 'We could not read this one just now, so what it is doing is unknown. Reload to try again.'
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
