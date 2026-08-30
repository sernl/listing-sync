// How a marketplace is named and which ones this console lets a seller author
// for. Its own module rather than part of the create form's model, because the
// publish dialog and the product page name platforms without composing a
// create. Pure, so it tests without a component.

import { INVENTORY_ORDER } from '$lib/listings-view';
import type { InventoryId } from '$lib/generated/vocab';

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

export function platformTitle(inventory: InventoryId): string {
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
