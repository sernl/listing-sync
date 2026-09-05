import { redirect } from '@sveltejs/kit';
import { legacyDestination } from '$lib/nav';
import type { PageLoad } from './$types';

/** "Resources" is the catalogue now, so this path lands there; the help
 *  placeholder that used to live here moved to `/guides`. */
export const load: PageLoad = ({ url }) => {
	redirect(308, legacyDestination(url.pathname) ?? '/inventory');
};
