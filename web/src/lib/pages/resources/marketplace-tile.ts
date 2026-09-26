// One marketplace's tile on the resource page: its mark, its one status word,
// one short clause of detail, where tapping it goes, and which of the two
// controls it carries. Pure, so it tests without a component.
//
// Nothing here is a second status vocabulary. The word and the tone are
// `STATE_LABEL` and `STATE_TONE` read straight off the chip, so the tile, the
// board's strip and the filter bar cannot disagree about what a marketplace is
// doing. What this module adds is the shorter reading: the chip's sentence is
// written for a row that has a line to spare, and a tile has two.

import { agoLabel } from '$lib/elapsed';
import type { MappingHead } from '$lib/api';
import type { ChipTone, MarketplaceChip } from '$lib/inventory';
import { AUTHORABLE, MARK_SRC, platformTitle } from '$lib/platforms';
import { MARKETPLACE_OF } from '$lib/listings-view';
import type { Readiness } from '$lib/publish-readiness';
import type { InventoryId } from '$lib/generated/vocab';

/** The second control a tile can carry, beside the link that is the tile
 *  itself. `attach` records a listing that already exists on the marketplace;
 *  `crosslist` adds a marketplace this resource does not reach. */
export type TileControl = 'attach' | 'crosslist';

export interface TileInput {
	chip: MarketplaceChip;
	mapping: MappingHead | undefined;
	/** The resource being looked at, so a link back to it can be dropped. */
	product: string;
	now: number;
	/** What the publish dialog would say about sending here, where it has been
	 *  computed. Its phrase joins the accessible name; it is not drawn. */
	readiness: Readiness | undefined;
}

export interface MarketplaceTileView {
	inventory: InventoryId;
	markSrc: string;
	label: string;
	tone: ChipTone;
	/** One clause, not a sentence: the tile has room for a line and the whole
	 *  of the chip's sentence is in `name`. */
	detail: string;
	href: string | null;
	external: boolean;
	/** Everything the tile says, spelled out, for a screen reader and a hover.
	 *  The status is therefore never carried by the tone alone. */
	name: string;
	control: TileControl | null;
	/** The mapping the `attach` control writes against, where there is one. */
	mapping: string | null;
	/** Why sending is paused for the whole marketplace, where it is. */
	paused: string | null;
}

/** How long a clause the tile has room for under its status word. */
const CLAUSE_MAX = 56;

/** The first clause of a recorded failure, short enough for one line.
 *
 * The recorded detail is whatever the marketplace or the adapter said, so it
 * runs to sentences; the whole of it stays in the accessible name. */
function firstClause(text: string): string {
	const head = text.split(/[.;\n]/)[0].trim();
	if (head.length === 0) {
		return 'the send failed';
	}
	if (head.length <= CLAUSE_MAX) {
		return head;
	}
	const cut = head.slice(0, CLAUSE_MAX);
	// Cut at a word rather than through one, unless the first word is itself
	// longer than the line and there is no boundary to cut at.
	const boundary = cut.lastIndexOf(' ');
	return `${(boundary === -1 ? cut : cut.slice(0, boundary)).trimEnd()}…`;
}

/** The clause under the status word.
 *
 * A closed switch over the eight states rather than a shortened `chip.detail`,
 * because a sentence trimmed to fit says a different thing for every state and
 * this says one thing per state. */
function clauseFor(
	chip: MarketplaceChip,
	mapping: MappingHead | undefined,
	now: number
): string {
	switch (chip.state) {
		case 'listed':
			return mapping === undefined
				? 'showing on the marketplace'
				: `updated ${agoLabel(mapping.updated_at, now)}`;
		case 'failed':
			return firstClause(chip.detail);
		case 'needs_signin':
			return 'sign-in needed';
		case 'stranded':
			return 'interrupted, on hold';
		case 'in_flight':
			return 'sending now';
		case 'blocked':
			return 'waiting on an answer';
		case 'draft':
			return 'saved as a draft';
		case 'not_listed':
			return mapping === undefined ? 'not added yet' : 'not sent yet';
		default: {
			// A state this bundle has no word for. The `never` binding fails the
			// build when the vocabulary widens; the clause is what a seller
			// reads on a console built before that widening, because the bundle
			// is static and a Rust deploy does not rebuild an open page.
			const unnamed: never = chip.state;
			return 'status unknown, reload to update';
		}
	}
}

/** Everything the tile says, in one sentence.
 *
 * The same shape the board's chips already announce, plus the readiness phrase
 * where the publish dialog would refuse: the resource page used to draw that
 * phrase as a fourth line per marketplace, and the tile keeps it reachable
 * rather than dropping it. */
function describe(input: TileInput): string {
	const { chip } = input;
	const paused = chip.paused === null ? '' : ` Sending is paused here: ${chip.paused}`;
	const opens = chip.action?.external === true ? ' Opens the listing on the marketplace.' : '';
	const next =
		input.readiness === undefined || input.readiness.ready
			? ''
			: ` Next send: ${input.readiness.line}.`;
	return `${platformTitle(chip.inventory)}: ${chip.label}. ${chip.detail}${paused}${opens}${next}`;
}

/** Which control this marketplace offers under its status.
 *
 * A marketplace with no mapping offers to be added, and only where this
 * console has a create path for it at all; a mapping nothing has been bound to
 * offers to record a listing that already exists. Everything else offers
 * neither, because the tile itself is the action. */
function controlFor(
	inventory: InventoryId,
	mapping: MappingHead | undefined
): TileControl | null {
	if (mapping === undefined) {
		return AUTHORABLE[inventory] ? 'crosslist' : null;
	}
	return mapping.binding_state === 'unbound' ? 'attach' : null;
}

/** One tile.
 *
 * The href is the chip's own destination with one correction: a chip whose
 * action is "open the item" is pointing at the page this tile is drawn on, and
 * a link to where you already are is worse than no link, because the tile then
 * reads as a place to go and is not one. */
export function tileFor(input: TileInput): MarketplaceTileView {
	const { chip, mapping } = input;
	const own = `/resources/${input.product}`;
	const href = chip.action === null || chip.action.href === own ? null : chip.action.href;
	return {
		inventory: chip.inventory,
		markSrc: MARK_SRC[MARKETPLACE_OF[chip.inventory]],
		label: chip.label,
		tone: chip.tone,
		detail: clauseFor(chip, mapping, input.now),
		href,
		external: href !== null && chip.action?.external === true,
		name: describe(input),
		control: controlFor(chip.inventory, mapping),
		mapping: mapping?.id ?? null,
		paused: chip.paused
	};
}
