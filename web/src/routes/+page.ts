import { redirect } from '@sveltejs/kit';
import { HOME_ITEM } from '$lib/nav';
import type { PageLoad } from './$types';

/** The console's home is `/app`, because `/` belongs to the public site (D28).
 *  `/app` answers with the Resources list rather than a dashboard of its own.
 *  Not a `LEGACY_REDIRECTS` entry: that table is matched against every path
 *  string in the source tree, and `/` prefixes all of them. */
export const load: PageLoad = () => {
	redirect(308, HOME_ITEM.href);
};
