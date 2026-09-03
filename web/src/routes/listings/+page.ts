import { redirect } from '@sveltejs/kit';
import { legacyDestination } from '$lib/nav';
import type { PageLoad } from './$types';

/** The inventory board moved to `/inventory`, carrying its query string so a
 *  saved search still lands filtered. */
export const load: PageLoad = ({ url }) => {
	redirect(308, (legacyDestination(url.pathname) ?? '/inventory') + url.search);
};
