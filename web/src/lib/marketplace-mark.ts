// What one marketplace mark shows and what it is called, for every site that
// draws a marketplace by its logo rather than by its name. Pure, so it tests
// without a component; `MarketplaceMark.svelte` is the one thing that draws it.

import type { InventoryId, Marketplace } from '$lib/generated/vocab';
import { MARKETPLACE_OF } from '$lib/listings-view';
import { MARKETPLACE_NAME, MARK_SRC, REGION_TAG, platformTitle } from '$lib/platforms';

export interface MarkView {
	/** The file the mark is drawn from, or null where this bundle knows no mark
	 *  for the identifier it was handed, in which case the name is drawn in its
	 *  place. */
	src: string | null;
	/** The two letters that tell the three Tes sites apart beside one mark, and
	 *  null where the mark alone identifies the marketplace. */
	region: string | null;
	/** The platform spelled out in words: the accessible name, and what a hover
	 *  shows. */
	name: string;
}

/** The mark for one inventory, named with its region where it has one.
 *
 *  Guarded like `platformTitle`, because a row's inventory is read straight
 *  off the wire and a server ahead of this bundle can name one the generated
 *  union does not hold; indexing `MARK_SRC` through an undefined marketplace
 *  would draw an empty box with no name at all. */
export function inventoryMark(inventory: InventoryId): MarkView {
	if (!Object.hasOwn(MARKETPLACE_OF, inventory)) {
		return { src: null, region: null, name: platformTitle(inventory) };
	}
	return {
		src: MARK_SRC[MARKETPLACE_OF[inventory]],
		region: REGION_TAG[inventory],
		name: platformTitle(inventory)
	};
}

/** The mark for one marketplace, where the surface is per-marketplace rather
 *  than per-inventory: a connection, a device login, an analytics scope. */
export function marketplaceMark(marketplace: Marketplace): MarkView {
	if (!Object.hasOwn(MARK_SRC, marketplace)) {
		return { src: null, region: null, name: marketplace };
	}
	return { src: MARK_SRC[marketplace], region: null, name: MARKETPLACE_NAME[marketplace] };
}
