// How a marketplace is named and which ones this console lets a seller author
// for. Its own module rather than part of the create form's model, because the
// publish dialog and the product page name platforms without composing a
// create. Pure, so it tests without a component.

import { INVENTORY_ORDER, MARKETPLACE_OF } from '$lib/listings-view';
import type { InventoryId, Marketplace } from '$lib/generated/vocab';

/** How one marketplace is named to a seller.
 *
 * The founder's format is the acronym followed by the full name, which is
 * what a seller recognises the platform by. */
export interface PlatformName {
	acronym: string;
	full: string;
}

/** A total map rather than a lookup with a fallback: the union is generated
 *  from the Rust enum, so an inventory added there stops this file
 *  type-checking instead of rendering as an unnamed platform. */
export const PLATFORMS: Record<InventoryId, PlatformName> = {
	Tes: { acronym: 'TES', full: 'Tes.com' },
	Tpt: { acronym: 'TPT', full: 'Teachers Pay Teachers' },
	Etsy: { acronym: 'Etsy', full: 'Etsy.com' }
};

/** The shortest name that still tells one inventory from another.
 *
 * A total map rather than a derivation from `PLATFORMS`, because the acronym
 * a full title leads with is not always the word a chip has room for. */
export const SHORT_NAME: Record<InventoryId, string> = {
	Tpt: 'TPT',
	Tes: 'TES',
	Etsy: 'Etsy'
};

/** How a marketplace is named where the surface is per-marketplace rather
 *  than per-inventory: a connection, a device login, an analytics scope. */
export const MARKETPLACE_NAME: Record<Marketplace, string> = {
	Tpt: 'TPT (Teachers Pay Teachers)',
	Tes: 'TES (Tes.com)',
	Etsy: 'Etsy (Etsy.com)'
};

/** The one word a teacher calls this marketplace inside a sentence, where the
 *  full name would read as a legal entity rather than as a place they sell. */
export const MARKETPLACE_WORD: Record<Marketplace, string> = {
	Tpt: 'TPT',
	Tes: 'Tes',
	Etsy: 'Etsy'
};

/** How this marketplace is titled, or its own identifier where the map has no
 *  name for it.
 *
 * `Object.hasOwn` rather than indexing straight in, and a fallback at all,
 * because the map is total over the union and the argument is not always drawn
 * from it. The chip strip filters through `INVENTORY_ORDER` first, but a row's
 * `mapped` set is built straight from the wire mappings and reaches the delete
 * and bulk dialogs whole, so an inventory a server ahead of this bundle knows
 * about arrives here. Reading `.acronym` off the resulting `undefined` took the
 * dialog down rather than naming one unknown row. */
export function platformTitle(inventory: InventoryId): string {
	if (!Object.hasOwn(PLATFORMS, inventory)) {
		return inventory;
	}
	const name = PLATFORMS[inventory];
	return `${name.acronym} (${name.full})`;
}

/** Whether this client offers the inventory on the create form.
 *
 * Etsy is excluded because no adapter exists for it: the vocabulary endpoint
 * serves its registry entry, and `authoring` states the neutral shape rather
 * than a claim about a write nobody has made. Offering it would let a seller
 * map a platform nothing can ever send to. */
export const AUTHORABLE: Record<InventoryId, boolean> = {
	Tes: true,
	Tpt: true,
	Etsy: false
};

/** The platforms the create form offers, in the one order this console shows
 *  platforms in, so the form and the listings table never order the same pair
 *  differently. */
export const AUTHORABLE_PLATFORMS: readonly InventoryId[] = INVENTORY_ORDER.filter(
	(inventory) => AUTHORABLE[inventory]
);

/** The file each marketplace's own mark is served from, under `web/static`.
 *
 * Held here rather than reached for through the Marketplaces catalogue,
 * because the resource page and the board's chips draw marks without
 * composing a tile, and a page-to-page import for three string constants
 * would put a catalogue of twenty-three marketplaces behind two of them.
 * `platforms.test.ts` asserts these are the same three paths the catalogue
 * holds, so the two cannot drift apart unnoticed. */
export const MARK_SRC: Record<Marketplace, string> = {
	Tes: '/marketplaces/tes-mark.svg',
	Tpt: '/marketplaces/tpt-mark.svg',
	Etsy: '/marketplaces/etsy.svg'
};

/** One tile on the form's marketplace grid.
 *
 * One tile, one marketplace, one inventory: Tes is one marketplace with no
 * regions (`decisions.md`, 2026-09-12), so ticking a tile is the whole answer
 * to where a listing goes.
 *
 * `authorable` is false where no adapter exists, and the tile is still drawn:
 * a marketplace the console cannot write to is a thing a seller should be able
 * to see is coming rather than a gap they cannot ask about. */
export interface MarketplaceTileEntry {
	marketplace: Marketplace;
	/** The full name, shown on hover and read out to a screen reader. */
	name: string;
	/** The inventory this tile publishes to. */
	inventory: InventoryId;
	authorable: boolean;
}

/** The marketplaces the form offers, one tile each. */
export const MARKETPLACE_TILES: readonly MarketplaceTileEntry[] = [
	{
		marketplace: 'Tpt',
		name: MARKETPLACE_NAME.Tpt,
		inventory: 'Tpt',
		authorable: AUTHORABLE.Tpt
	},
	{
		marketplace: 'Tes',
		name: MARKETPLACE_NAME.Tes,
		inventory: 'Tes',
		authorable: AUTHORABLE.Tes
	},
	{
		marketplace: 'Etsy',
		name: MARKETPLACE_NAME.Etsy,
		inventory: 'Etsy',
		authorable: AUTHORABLE.Etsy
	}
];

/** The id of one marketplace's card on the Marketplaces page.
 *
 * A total map over the generated union, so a marketplace added in Rust stops
 * this file type-checking rather than linking at a card that is not there. */
export const CARD_ANCHOR: Record<Marketplace, string> = {
	Tes: 'mp-tes',
	Tpt: 'mp-tpt',
	Etsy: 'mp-etsy'
};

/** The id a Marketplaces card takes, from the name it draws.
 *
 * Derived from the name rather than passed down from the page's own lists,
 * because the card is handed a name and not the tile it came from. The three
 * marketplaces this console links into are held to `CARD_ANCHOR` by
 * `platforms.test.ts`, and every other tile only needs an id that is its own. */
export function cardAnchor(name: string): string {
	const slug = name
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, '-')
		.replace(/^-+|-+$/g, '');
	return `mp-${slug}`;
}

/** The Marketplaces page, scrolled to the marketplace this inventory belongs
 *  to.
 *
 * The page names no marketplace on its own, so a seller sent there to sign in
 * arrives at a grid of twenty-three cards and has to find the one they were
 * sent for. */
export function marketplacesHref(inventory: InventoryId): string {
	return `/marketplaces#${CARD_ANCHOR[MARKETPLACE_OF[inventory]]}`;
}
