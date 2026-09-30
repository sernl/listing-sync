// The "Filed under" band on the resource page: every marketplace the resource
// is mapped to, each with where it stands there, then the seller's own labels.
//
// Built from the mappings rather than from the labels. The band used to draw
// only the labels, and the one marketplace mark among them is the system label
// an import writes for the shop it read, and only once its listing is bound —
// so a TPT import that the seller then ticked Tes for showed a TPT chip and
// nothing for Tes. A mapping is what says a resource goes to a marketplace;
// the import's label says only where it came from, and is dropped here
// wherever a mapping already names that shop.
//
// The strip's chip is not enough on its own either: it files a mapping that
// was picked but never sent under `not_listed`, the same state as no mapping
// at all, so the band asks the mappings which marketplaces it reaches and the
// chip only how each one stands. Pure, so it tests without a component.

import type { LabelView, MappingHead } from '$lib/api';
import type { InventoryId } from '$lib/generated/vocab';
import type { ChipTone, MarketplaceChip } from '$lib/inventory';
import { MARKETPLACE_OF } from '$lib/listings-view';
import { MARKETPLACE_WORD } from '$lib/platforms';
import { marketplaceOfLabel } from '$lib/system-labels';

/** One marketplace the resource is mapped to, as the band draws it. */
export interface FiledMarketplace {
	inventory: InventoryId;
	/** The word a teacher calls the marketplace. */
	name: string;
	/** Where the resource stands there, in two or three words. */
	state: string;
	tone: ChipTone;
	/** The sentence behind the state, for the chip's title. */
	detail: string;
}

export interface FiledUnder {
	marketplaces: FiledMarketplace[];
	labels: LabelView[];
}

/** The band for one resource. `chips` is the page's strip, so the band and
 *  the tiles above it cannot disagree about how a marketplace stands; which
 *  marketplaces are drawn at all is `mappings`' answer. */
export function filedUnder(
	mappings: readonly MappingHead[],
	chips: readonly MarketplaceChip[],
	labels: readonly LabelView[]
): FiledUnder {
	const mapped = new Set(mappings.map((mapping) => mapping.inventory));
	const marketplaces: FiledMarketplace[] = [];
	for (const chip of chips) {
		if (!mapped.has(chip.inventory)) {
			continue;
		}
		marketplaces.push({
			inventory: chip.inventory,
			name: MARKETPLACE_WORD[MARKETPLACE_OF[chip.inventory]] ?? chip.inventory,
			// The strip's own word, except where the strip's is an instruction —
			// in a band "Sign in" reads as a button that is not one — and where
			// it says "Not listed" of a marketplace the seller did pick.
			state:
				chip.state === 'needs_signin'
					? 'Sign-in needed'
					: chip.state === 'not_listed'
						? 'Not sent yet'
						: chip.label,
			tone: chip.tone,
			detail: chip.detail
		});
	}
	const reached = new Set(marketplaces.map((one) => MARKETPLACE_OF[one.inventory]));
	return {
		marketplaces,
		labels: labels.filter((label) => {
			if (!label.system) {
				return true;
			}
			const marketplace = marketplaceOfLabel(label.name);
			return marketplace === null || !reached.has(marketplace);
		})
	};
}
