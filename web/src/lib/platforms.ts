// How a marketplace is named and which ones this console lets a seller author
// for. Its own module rather than part of the create form's model, because the
// publish dialog and the product page name platforms without composing a
// create. Pure, so it tests without a component.

import { INVENTORY_ORDER, MARKETPLACE_OF } from '$lib/listings-view';
import type { InventoryId, Marketplace } from '$lib/generated/vocab';

/** How one marketplace is named to a seller.
 *
 * The founder's format is the acronym followed by the full name, which is
 * what a seller recognises the platform by. `region` disambiguates the three
 * Tes sites, which are one marketplace under three inventories and would
 * otherwise render as three identical rows. */
export interface PlatformName {
	acronym: string;
	full: string;
	region: string | null;
}

/** A total map rather than a lookup with a fallback: the union is generated
 *  from the Rust enum, so an inventory added there stops this file
 *  type-checking instead of rendering as an unnamed platform. */
export const PLATFORMS: Record<InventoryId, PlatformName> = {
	TesGb: { acronym: 'TES', full: 'Tes.com', region: 'United Kingdom' },
	TesUs: { acronym: 'TES', full: 'Tes.com', region: 'United States' },
	TesNz: { acronym: 'TES', full: 'Tes.com', region: 'New Zealand' },
	Tpt: { acronym: 'TPT', full: 'Teachers Pay Teachers', region: null },
	Etsy: { acronym: 'Etsy', full: 'Etsy.com', region: null }
};

/** The shortest name that still tells one inventory from another.
 *
 * A total map rather than a derivation from `PLATFORMS`, because the three
 * Tes sites differ only by region and a derived short name would render them
 * identically on a strip whose whole job is telling them apart. */
export const SHORT_NAME: Record<InventoryId, string> = {
	Tpt: 'TPT',
	TesGb: 'TES GB',
	TesUs: 'TES US',
	TesNz: 'TES NZ',
	Etsy: 'Etsy'
};

/** How a marketplace is named where the surface is per-marketplace rather
 *  than per-inventory: a device holds one login for Tes, not three. */
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
	const head = `${name.acronym} (${name.full})`;
	return name.region === null ? head : `${head} · ${name.region}`;
}

/** Whether this client offers the inventory on the create form.
 *
 * Etsy is excluded because no adapter exists for it: the vocabulary endpoint
 * serves its registry entry, and `authoring` states the neutral shape rather
 * than a claim about a write nobody has made. Offering it would let a seller
 * map a platform nothing can ever send to. */
export const AUTHORABLE: Record<InventoryId, boolean> = {
	TesGb: true,
	TesUs: true,
	TesNz: true,
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
 * Keyed by marketplace rather than by inventory, which is the founder's rule
 * of 2026-09-11: Tes runs three regional catalogues and the form shows one
 * Tes. Which of the three a listing reaches is the Tes panel's Curriculum
 * question, so the tile carries the inventories it stands for and the form
 * derives the request's list from the two answers together.
 *
 * `authorable` is false where no adapter exists, and the tile is still drawn:
 * a marketplace the console cannot write to is a thing a seller should be able
 * to see is coming rather than a gap they cannot ask about. */
export interface MarketplaceTileEntry {
	marketplace: Marketplace;
	/** The full name, shown on hover and read out to a screen reader. */
	name: string;
	/** The inventories this one tile stands for, in offer order. */
	inventories: readonly InventoryId[];
	authorable: boolean;
}

/** The marketplaces the form offers, one tile each. */
export const MARKETPLACE_TILES: readonly MarketplaceTileEntry[] = [
	{
		marketplace: 'Tpt',
		name: MARKETPLACE_NAME.Tpt,
		inventories: ['Tpt'],
		authorable: AUTHORABLE.Tpt
	},
	{
		marketplace: 'Tes',
		name: MARKETPLACE_NAME.Tes,
		inventories: ['TesGb', 'TesUs', 'TesNz'],
		authorable: AUTHORABLE.TesGb
	},
	{
		marketplace: 'Etsy',
		name: MARKETPLACE_NAME.Etsy,
		inventories: ['Etsy'],
		authorable: AUTHORABLE.Etsy
	}
];

/** Where on Tes a listing goes: the question Tes's own upload form asks, in
 *  the words it asks it in, against the inventory each answer publishes to. */
export const TES_CURRICULA: readonly { inventory: InventoryId; label: string }[] = [
	{ inventory: 'TesGb', label: 'England' },
	{ inventory: 'TesUs', label: 'United States' },
	{ inventory: 'TesNz', label: 'New Zealand' }
];

/** The two letters that tell the three Tes sites apart beside a mark, and
 *  nothing where the mark already identifies the marketplace on its own.
 *
 * A total map rather than a slice of `PLATFORMS[inventory].region`, because
 * the region there is the country written out and this is what fits beside a
 * 28px logo. Null exactly where that region is null, which is what
 * `platforms.test.ts` holds it to. */
export const REGION_TAG: Record<InventoryId, string | null> = {
	Tpt: null,
	TesGb: 'GB',
	TesUs: 'US',
	TesNz: 'NZ',
	Etsy: null
};

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
