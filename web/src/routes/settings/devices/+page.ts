import { redirect } from '@sveltejs/kit';
import { legacyDestination } from '$lib/nav';
import type { PageLoad } from './$types';

/** The machines and their marketplace logins moved onto `/marketplaces`,
 *  which now answers the one question this screen and the connections screen
 *  each answered half of: whether a marketplace can be written to right now. */
export const load: PageLoad = ({ url }) => {
	redirect(308, legacyDestination(url.pathname) ?? '/marketplaces');
};
