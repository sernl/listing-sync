/** The account tile, decided once for the two places the shell draws it: the
 *  top strip's avatar above the phone breakpoint, and the page header's avatar
 *  button below it.
 *
 * A module store rather than a per-component derivation, because the two
 * drawings are in different components now -- `Console.svelte` owns the strip
 * and `PageHead.svelte` the header button -- and the one thing they must agree
 * about is a picture whose bytes would not draw. A second copy of `unshowable`
 * would let the header keep retrying an image the strip has already given up
 * on, and the seller would see the initials in one place and a broken frame in
 * the other.
 *
 * `Console.svelte` is the only writer: it holds the organisation and profile
 * reads already, and it is the one component that renders only under a
 * session. A `PageHead` on a public page -- `/status` signed out -- therefore
 * draws no tile and issues no read of its own. */

import { accountTile, type AccountTile } from '$lib/nav';

let picture = $state<string | null>(null);
let orgName = $state<string | undefined | null>(undefined);
/** The picture whose bytes would not draw, held as its address so a later
 *  picture clears it by being a different address. */
let unshowable = $state<string | null>(null);

export const accountTileState = {
	get tile(): AccountTile {
		return accountTile(picture === unshowable ? null : picture, orgName);
	},

	/** What the shell has read so far. Called from an effect rather than
	 *  assigned at setup: both figures arrive from queries, so every frame
	 *  before they land is a real state the tile has to answer for. */
	observe(next: { picture: string | null; orgName: string | undefined | null }) {
		picture = next.picture;
		orgName = next.orgName;
	},

	/** Records that this picture will not draw, which moves every tile to the
	 *  initials at once. */
	unusable() {
		unshowable = picture;
	}
};
