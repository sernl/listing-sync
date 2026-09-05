// The import screen as a model: which marketplaces a shop can be brought
// across from, what stands in the way of each, and how a request already made
// reads on the list. Pure, so it tests without a component.
//
// What a request itself says — its stage, its coverage, its listings — stays
// in `$lib/sync-request`, which the migration screen reads too. Only what is
// true of this screen lives here: the per-marketplace card, and the wording of
// a refusal from the machine the seller is standing at.

import type { ConnectionView, SyncRequestHead } from '$lib/api';
import { APP_TOO_OLD, type StartOutcome } from '$lib/desktop';
import { TRANSPORT_OF } from '$lib/inventory';
import { MARKETPLACE_OF, INVENTORY_ORDER } from '$lib/listings-view';
import {
	headStage,
	listRowLine,
	presentStage,
	sourcesOn,
	type StageTone
} from '$lib/sync-request';
import type { InventoryId, Marketplace } from '$lib/generated/vocab';

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
 * marketplace offers no source at all — `sourcesOn` is what decides that, and
 * it mirrors `tam_storage::uncaptured_source`, which admits the three Tes
 * inventories and refuses the rest — so `Tes` carries null and the invariant
 * that every other entry is a sentence is held by a test rather than by the
 * type. */
const UNREADABLE: Record<Marketplace, string | null> = {
	Tes: null,
	Tpt: 'Nothing here reads a TPT shop yet. TPT is where a shop brought across arrives.',
	Etsy: 'Etsy is not built yet, so nothing here can read a shop on it.'
};

/** The site a Tes card starts on.
 *
 * Named rather than taken as the first of the list, so reordering the sources
 * cannot quietly move the default onto another country's shop. */
export const DEFAULT_SITE: InventoryId = 'TesGb';

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
	/** The sites a shop can be read from, in the console's platform order.
	 *  Empty where nothing here reads this marketplace. */
	sites: readonly InventoryId[];
	/** The site the form starts on, and null where there is no choice. */
	preselected: InventoryId | null;
	/** Whether the seller holds a connection for this marketplace, and whether
	 *  this console knows.
	 *
	 * Presence rather than health, which is `migrateSource`'s own predicate: a
	 * connection needing a fresh sign-in is still the seller's shop, and it is
	 * their own device that discovers the sign-in is needed rather than this
	 * page. */
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
	const held = connections === null ? null : new Set(connections.map((one) => one.marketplace));
	const cards = onDeviceBranch().map((marketplace) => {
		const sites = sourcesOn(marketplace);
		return {
			marketplace,
			name: ACRONYM[marketplace],
			sites,
			preselected: startingSite(sites),
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
 *  the seller. */
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

function startingSite(sites: readonly InventoryId[]): InventoryId | null {
	if (sites.includes(DEFAULT_SITE)) {
		return DEFAULT_SITE;
	}
	return sites[0] ?? null;
}

/** Why this card cannot hand off to Marketplace Migration, or null where it
 *  can.
 *
 * Two answers rather than three: under D8 the request is raised on Marketplace
 * Migration, so the authorship declaration is gated where it is raised and is
 * no longer this page's to ask about. What remains is whether a shop can be
 * read at all and whether this console knows the seller has one.
 *
 * Tested in the order a seller can act on the answers: a marketplace nothing
 * reads is not theirs to fix, and an absent connection is fixed on their own
 * machine. A disabled control states its reason, which is what `Button`
 * requires of every caller. */
export function handoffBlocked(card: ImportCard): string | null {
	if (card.unreadable !== null) {
		return card.unreadable;
	}
	if (card.standing === 'unread') {
		return connectionUnknown(card);
	}
	if (card.standing === 'absent') {
		return notConnected(card);
	}
	return null;
}

/** Where the seller goes to raise the request for one site.
 *
 * The query value is the inventory identifier `sourcesOn` yields — `TesGb`,
 * `TesUs`, `TesNz` — because Marketplace Migration validates the parameter
 * against that same list. Sending a prettier name would hand it a value it is
 * right to reject. */
export function migrationHref(site: InventoryId): string {
	return `${MIGRATION_HREF}?${new URLSearchParams({ source: site }).toString()}`;
}

/** The screen that owns raising the request (D8). */
export const MIGRATION_HREF = '/automations/migration';

/** What the handoff control says. Names the destination, because pressing it
 *  leaves this page rather than starting anything here. */
export const HANDOFF_LABEL = 'Continue on Marketplace Migration';

/** What importing is today, said once and plainly.
 *
 * D8: the request both screens raised is a migration — it drafts every listing
 * on TPT — and the backend has no catalogue-only import kind, so this page
 * describes and routes rather than pretending to a verb that does not exist
 * yet. Saying the coming feature outright is what keeps the page honest about
 * why it is a description and not an action. */
export const IMPORT_IS_A_MIGRATION =
	'Today an import runs as a migration: every listing found is drafted on TPT for you to ' +
	'review. Importing without drafting anywhere is a coming feature.';

/** What the card says while this console could not read the connections list.
 *
 * Says that we do not know, never that the marketplace is disconnected. */
export function connectionUnknown(card: ImportCard): string {
	return (
		`Your marketplaces could not be read, so we cannot say whether ${card.name} is ` +
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
	held: { tone: 'ok', label: 'Connected' },
	absent: { tone: 'bad', label: 'Not connected' },
	unread: { tone: 'soon', label: 'Not known' }
};

export function standingBadge(card: ImportCard): { tone: PillTone; label: string } {
	return STANDING_BADGE[card.standing];
}

/** What the card says while the seller holds no connection for it. */
export function notConnected(card: ImportCard): string {
	return `${card.name} is not connected. Connect it on Marketplaces first.`;
}

/** The permanent line under the site choice, which says where the work runs
 *  and therefore when it starts. */
export function deviceLine(card: ImportCard): string {
	return `${card.name} runs on your own device, so the import starts the next time that device checks in.`;
}

/** What an import is, said before anything is chosen.
 *
 * Says what the footnote does not: that the arriving thing is a record, and
 * that the file's own location is part of it. What travels and what does not
 * is `FILES_STAY_ON_YOUR_COMPUTER`'s sentence, and stating it twice on one
 * screen makes a seller read the second as a correction of the first. */
export const WHAT_AN_IMPORT_IS =
	'An import copies a marketplace’s listings into your Resources: the details, ' +
	'and where each file sits on your own computer.';

/** Where a connection is made, which is the marketplaces screen. */
export const CONNECT_HREF = '/marketplaces';

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

/** One import as the list shows it. */
export interface ImportRow {
	request: string;
	/** Where the shop was read and where it arrived. */
	title: string;
	/** The one line the row says about where the request stands. */
	line: string;
	label: string;
	tone: PillTone;
	created_at: number;
}

/** Every import this seller has made, newest first.
 *
 * Filtered to the migrate disposition, because a sync is not an import and
 * belongs to the Automations screens; the endpoint serves both. The order is
 * the endpoint's own, which is newest first, so nothing is re-sorted here. */
export function importRows(
	heads: readonly SyncRequestHead[],
	name: (inventory: InventoryId) => string
): ImportRow[] {
	return heads
		.filter((head) => head.disposition === 'migrate')
		.map((head) => {
			const stage = headStage(head);
			const shown = presentStage(stage);
			return {
				request: head.request,
				title: `${name(head.source)} → ${name(head.target)}`,
				line: listRowLine(stage),
				label: shown.label,
				tone: PILL_TONE[shown.tone],
				created_at: head.created_at
			};
		});
}

/** What the list says while it holds nothing, which is not the same as what it
 *  says when it could not be read. */
export const NO_IMPORT_YET = 'No import has run yet.';

/** An import list that could not be read is not a seller who has brought no
 *  shop across. Told the second when the first is true, they start the import
 *  again. */
export const IMPORTS_UNREAD =
	'Your imports could not be read, so this page cannot list them. Any import already ' +
	'running is unaffected.';

/** A marketplace list that could not be read is not a seller with no
 *  marketplace, so the page says which of the two it is looking at rather than
 *  letting an outage read as "you have no shop". */
export const CONNECTIONS_UNREAD =
	'Your marketplaces could not be read, so we cannot list the shops you can bring ' +
	'across; nothing has been started.';
