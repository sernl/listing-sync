// The import screen as a model: which marketplaces a shop can be read from,
// what stands in the way of each, and how an import already run reads on the
// list. Pure, so it tests without a component.
//
// What one run itself says — its stage, its bar, its review cards — stays in
// `run-view`, which the run page and the spreadsheet batch page both read.
// Only what is true of this screen lives here: the per-marketplace card, the
// wording of a refusal from the machine the seller is standing at, and the
// row one run draws in the list of them.

import type { ConnectionView, ImportRunHead, JobDeletionStatus } from '$lib/api';
import { standingMarketplaces } from '$lib/connection-standing';
import { APP_TOO_OLD, type LocalSessionOutcome, type StartOutcome } from '$lib/desktop';
import { agoLabel } from '$lib/elapsed';
import { TRANSPORT_OF } from '$lib/inventory';
import { MARKETPLACE_OF, INVENTORY_ORDER } from '$lib/listings-view';
import { AUTHORABLE_PLATFORMS } from '$lib/platforms';
import type { StageTone } from '$lib/sync-request';
import type { InventoryId, Marketplace } from '$lib/generated/vocab';
import { countsLine, runBadge, runHref, runName } from './run-view';

/** The tones `StatusPill` renders, named here rather than imported from the
 *  component so this module stays a plain module a node-environment test can
 *  read. Structurally the component's own union. */
export type PillTone = 'ok' | 'warn' | 'bad' | 'run' | 'soon' | 'flat';

/** The stage tones this console has always used, in the badge's vocabulary.
 *
 * Total, so a tone added to `$lib/sync-request` is placed here or fails the
 * web lane. The one that is spelt differently is grey: the stage model calls
 * it `mut` and the badge calls it `soon`, and both mean nothing to act on. */
const PILL_TONE: Record<StageTone, PillTone> = {
	ok: 'ok',
	run: 'run',
	bad: 'bad',
	mut: 'soon'
};

/** A stage's tone in the badge's vocabulary, for the request page as well as
 *  the list. */
export function pillTone(tone: StageTone): PillTone {
	return PILL_TONE[tone];
}

/** The shortest name for a marketplace, which is what a seller reads on the
 *  marketplace's own site.
 *
 * Local rather than in `$lib/platforms`, which names inventories: this
 * screen's cards are one per marketplace, and the three Tes sites share one
 * card because a device holds one login for Tes rather than three. */
const ACRONYM: Record<Marketplace, string> = {
	Tes: 'TES',
	Tpt: 'TPT',
	Etsy: 'Etsy'
};

/** Why nothing here can read a shop on this marketplace.
 *
 * A total map over the generated union, so a marketplace added in Rust is
 * given a sentence here or fails the web lane. Read only where the
 * marketplace offers no site to read at all — `importSitesOn` is what decides
 * that, and it mirrors which marketplaces this console can author on, because
 * an import reads a listing rather than downloading its file and needs no
 * file-download capture to do it.
 *
 * Both shops the app can sign into are readable, so both carry null and the
 * invariant that every other entry is a sentence is held by a test rather
 * than by the type. */
const UNREADABLE: Record<Marketplace, string | null> = {
	Tes: null,
	Tpt: null,
	Etsy: 'Etsy is not built yet, so nothing here can read a shop on it.'
};

/** What this console knows about the seller's connection for a marketplace.
 *
 * Three values because the read has three outcomes and only two of them are
 * facts about the seller. `held` and `absent` are what the connections list
 * said. `unread` is this console failing to read that list, or not having
 * finished reading it, which is a fact about us.
 *
 * Rendering `unread` as `absent` is the one failure this module was written to
 * prevent: it tells a seller whose shop is connected that it is not, and sends
 * them to repair something that is not broken. A boolean cannot hold the
 * distinction, so the type is not a boolean. */
export type ConnectionStanding = 'held' | 'absent' | 'unread';

/** One marketplace as this screen offers it. */
export interface ImportCard {
	marketplace: Marketplace;
	/** The acronym, which is how the card is titled. */
	name: string;
	/** The inventories a shop can be read from, in the console's platform
	 *  order. Empty where nothing here reads this marketplace. */
	sites: readonly InventoryId[];
	/** Whether the seller holds a connection for this marketplace, and whether
	 *  this console knows.
	 *
	 * Standing rather than health: a connection needing a fresh sign-in is
	 * still the seller's shop, and it is their own device that discovers the
	 * sign-in is needed rather than this page. What it is not is presence
	 * alone — a marketplace the seller disconnected leaves its row behind, and
	 * reading presence badged that row Connected. */
	standing: ConnectionStanding;
	/** Why a shop cannot be read from here, or null where one can. */
	unreadable: string | null;
}

/** The marketplaces this screen shows, in the order it shows them.
 *
 * The seller-device branch alone, which is D1's own line: an import is the
 * seller's own machine reading their own shop under their own session, so a
 * marketplace we work from our own infrastructure is not something this screen
 * can describe. `TRANSPORT_OF` is total over the union, so a marketplace added
 * in Rust is placed on a branch or fails the web lane.
 *
 * Ordered with the readable ones first, because the card a seller can act on
 * should not sit under one they cannot. */
export function importCards(connections: readonly ConnectionView[] | null): ImportCard[] {
	const held = connections === null ? null : standingMarketplaces(connections);
	const cards = onDeviceBranch().map((marketplace) => {
		const sites = importSitesOn(marketplace);
		return {
			marketplace,
			name: ACRONYM[marketplace],
			sites,
			standing: standingOf(held, marketplace),
			unreadable: sites.length === 0 ? UNREADABLE[marketplace] : null
		};
	});
	return [
		...cards.filter((card) => card.unreadable === null),
		...cards.filter((card) => card.unreadable !== null)
	];
}

/** A null set is the list not having been read, which is neither answer about
 *  the seller. A marketplace whose row exists but says `unlinked` or `revoked`
 *  is `absent`, because the seller gave that connection up: `$lib/connection-standing`
 *  owns which states those are. */
function standingOf(held: Set<Marketplace> | null, marketplace: Marketplace): ConnectionStanding {
	if (held === null) {
		return 'unread';
	}
	return held.has(marketplace) ? 'held' : 'absent';
}

function onDeviceBranch(): Marketplace[] {
	const seen = new Set<Marketplace>();
	for (const inventory of INVENTORY_ORDER) {
		const marketplace = MARKETPLACE_OF[inventory];
		if (TRANSPORT_OF[marketplace] === 'SellerDevice') {
			seen.add(marketplace);
		}
	}
	return [...seen];
}

/** The authorable sites an import can read on one marketplace, in platform order. */
export function importSitesOn(marketplace: Marketplace): InventoryId[] {
	return AUTHORABLE_PLATFORMS.filter((inventory) => MARKETPLACE_OF[inventory] === marketplace);
}

/** Account entitlement and this device's login gate a start independently
 * of the last connection standing reported to the server. */
export function importBlocked(
	card: ImportCard,
	planRefusal: string | null,
	local: LocalSessionOutcome | undefined,
	inApp: boolean
): string | null {
	if (card.unreadable !== null) {
		return card.unreadable;
	}
	if (planRefusal !== null) {
		return planRefusal;
	}
	if (!inApp) return null;
	if (local === undefined || local.kind === 'unavailable') {
		return 'This device’s marketplace login is not known. Open the current Teachouse app and check its connection.';
	}
	if (local.kind === 'refused') return local.detail;
	return local.connected
		? null
		: `${card.name} is not signed in on this device. Connect it on Marketplaces here.`;
}

/** What the card's own control says. Names the shop, because a page offering
 *  two of them needs each button to say which one it reads. */
export function importLabel(card: ImportCard): string {
	return `Import from ${card.name}`;
}

/** What a seller is told when the card is pressed in an ordinary browser.
 *
 * Not a refusal and not a fault: the shop is read under the login held on the
 * seller's own machine, and no server of ours holds one. The run is created
 * either way, so the sentence names where to carry on rather than telling
 * them nothing happened. */
export const NEEDS_THE_APP =
	'This import waits for an eligible device. Open the current Teachouse app on a device ' +
	'signed in to this marketplace to begin reading.';

/** What the card says while this console could not read the connections list.
 *
 * Says that we do not know, never that the marketplace is disconnected. */
export function connectionUnknown(card: ImportCard): string {
	return (
		`We could not read your marketplaces, so we cannot say whether ${card.name} is ` +
		'connected. Nothing has changed — reload to try again.'
	);
}

/** The badge the card's header carries for its connection.
 *
 * A total map, so a standing added above is given a rendering here or fails
 * the web lane. The unread badge is grey and says so in words rather than
 * being omitted: a card with no badge at all reads as connected by default,
 * which is the claim this whole type exists to avoid making. */
const STANDING_BADGE: Record<ConnectionStanding, { tone: PillTone; label: string }> = {
	held: { tone: 'ok', label: 'Connection recorded' },
	absent: { tone: 'soon', label: 'No connection recorded' },
	unread: { tone: 'soon', label: 'Server standing not known' }
};

export function standingBadge(card: ImportCard): {
	tone: PillTone;
	label: string;
} {
	return STANDING_BADGE[card.standing];
}

/** What the card says while the seller holds no connection for it.
 *
 * Names the app rather than only the screen. Marketplaces is where the control
 * is, and for a marketplace on the device branch that control connects only in
 * the app on the seller's own machine — so sending them to a page and stopping
 * there is what left a seller circling between two screens neither of which
 * could connect anything. */
export function notConnected(card: ImportCard): string {
	return (
		`No ${card.name} connection is currently recorded by the server. ` +
		'Check the login on this device in Marketplaces; a login held on another device is not available here.'
	);
}

/** The permanent line under the site choice, which says where the work runs
 *  and therefore what has to be open for it to run at all. */
export function deviceLine(card: ImportCard): string {
	return `Keep the Teachouse app open on this device while ${card.name} is read.`;
}

/** Where a connection is made, which is the marketplaces screen.
 *
 * One href for both hosts rather than one that names the downloads: inside the
 * app that screen now carries the Connect control itself, and in a browser it
 * carries the sentence naming the app with the downloads below it. Sending a
 * seller who already has the app to install it again is the loop this work
 * closed. */
export const CONNECT_HREF = '/marketplaces';

/** What the control leading there is called. Names the screen rather than the
 *  act, because pressing it connects nothing on its own. */
export const CONNECT_LABEL = 'Connect on Marketplaces';

/** What the import screen says before any card, while the seller has no
 *  marketplace connected at all.
 *
 * Before the cards, because the cards describe an act none of them can
 * complete: the founder's own reading of this screen was that there was no
 * option to connect anywhere on it. */
export const NOTHING_CONNECTED =
	'You have no marketplace connected, so there is nothing to import yet. Connect one on ' +
	'Marketplaces: you sign in through the Teachouse app on your computer, and that login ' +
	'stays on that machine.';

/** What the seller is told after asking this computer to run the import, or
 *  null where there is nothing to say.
 *
 * Read only by the request page now: under D8 this screen raises no request
 * and therefore starts nothing.
 *
 * `started` and `unavailable` both say nothing: the first because the page
 * moves to the request, the second because a browser was never going to run
 * the work and the request's own page says where it does. The application
 * words its own refusals and they are rendered rather than interpreted; the
 * one substitution is the version arm, whose remedy is an update and must not
 * read as a refusal. */
export function startRefusal(outcome: StartOutcome): string | null {
	switch (outcome.kind) {
		case 'started':
		case 'unavailable':
			return null;
		case 'unsupported':
			return APP_TOO_OLD;
		case 'refused':
			return outcome.detail;
	}
}

/** One import as the list shows it, whichever way it came in. */
export interface ImportRow {
	id: string;
	/** Where it is read: its own page for a shop, the batch's report for a
	 *  sheet. */
	href: string;
	/** What it is called: the shop, or the word for the other way in. */
	name: string;
	/** The shop it read, where it read one. The row draws the mark from it
	 *  and a spreadsheet row draws none. */
	source: InventoryId | null;
	/** The one line the row says about what the run did. */
	line: string;
	label: string;
	tone: PillTone;
	/** Whether this run is on its way out of the history, and how far that
	 *  has got. Null for a run nobody has asked to delete, which is almost
	 *  all of them: a deleted run is filtered out by the server, so the only
	 *  ones that reach this list are the retained arms. */
	deletion: JobDeletionStatus | null;
	created_at: number;
}

/** Every import this seller has run, newest first.
 *
 * Both kinds in one list, because a seller who imported a shop on Monday and
 * a spreadsheet on Tuesday has made two imports and not one of each: the two
 * tables behind them are ours, not theirs. The order is the endpoint's own,
 * which is newest first, so nothing is re-sorted here. */
export function importRows(runs: readonly ImportRunHead[]): ImportRow[] {
	return runs.map((run) => {
		const badge = runBadge(run);
		return {
			id: run.id,
			href: runHref(run),
			name: runName(run),
			source: run.source,
			line: countsLine(run.counts, run.read_total),
			label: badge.label,
			tone: badge.tone,
			deletion: run.deletion_status ?? null,
			created_at: run.created_at
		};
	});
}

/** How one import is named where it is not in the list: in a confirmation
 *  that is about to delete it, where the shop's name alone would not tell two
 *  imports of the same shop apart.
 *
 *  Takes `now` rather than reading the clock, for the reason every other
 *  elapsed label in this console does: a pure module that read the clock
 *  could not be tested and would freeze at whenever it was first called. */
export function importRunLabel(row: ImportRow, now: number): string {
	return `${row.name} · started ${agoLabel(row.created_at, now)}`;
}

/** What the list says while it holds nothing, which is not the same as what it
 *  says when it could not be read. */
export const NO_IMPORT_YET = 'No import has run yet.';

/** An import list that could not be read is not a seller who has brought no
 *  shop across. Told the second when the first is true, they start the import
 *  again. */
export const IMPORTS_UNREAD =
	'We could not read your imports, so this page cannot list them. Any import already ' +
	'running carries on.';

/** A marketplace list that could not be read is not a seller with no
 *  marketplace, so the page says which of the two it is looking at rather than
 *  letting an outage read as "you have no shop". */
export const CONNECTIONS_UNREAD =
	'We could not read your marketplaces, so this page cannot list what you can import. ' +
	'Nothing has started.';
