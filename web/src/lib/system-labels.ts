// The labels this console's own machinery owns, and the marketplace each one
// marks.
//
// A system label is written by an import and is not the seller's: they
// neither make nor remove one, it sits outside their twenty, and
// `PUT /v1/products/{p}/labels` refuses a set that names one. What the
// surfaces need from here is the name, in both directions — the finished
// import links to the board filtered by the name it wrote, and the board
// draws the marketplace's own mark on the chip carrying it.
//
// Mirrored rather than served: the name is the server's, written at commit,
// and no view carries the marketplace beside it. `Record<Marketplace, string>`
// is total, so a marketplace added in Rust is named here or fails the web
// lane.

import type { Marketplace } from '$lib/generated/vocab';

/** The label one marketplace's imports carry. Each is the shop's own short
 *  name, which is what a seller reads on that shop's site. */
export const SYSTEM_LABEL: Record<Marketplace, string> = {
	Tpt: 'TPT',
	Tes: 'Tes',
	Etsy: 'Etsy'
};

/** Which marketplace a label marks, or null where it marks none.
 *
 * Case-insensitive, because `label` is unique on `lower(name)` and a seller
 * who made `tpt` before their first import owns that row: the name we would
 * write and the name that stands are the same label.
 *
 * Never used to decide whether a label is the system's — that is `system` on
 * the view, and a seller's own label called `TPT` is still theirs. This
 * answers only which mark to draw once the flag has said so. */
export function marketplaceOfLabel(name: string): Marketplace | null {
	const needle = name.trim().toLowerCase();
	for (const [marketplace, label] of Object.entries(SYSTEM_LABEL)) {
		if (label.toLowerCase() === needle) {
			return marketplace as Marketplace;
		}
	}
	return null;
}
