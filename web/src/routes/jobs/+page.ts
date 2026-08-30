import { redirect } from '@sveltejs/kit';
import { legacyDestination } from '$lib/nav';
import type { PageLoad } from './$types';

/** The jobs list moved to `/sync`. A bookmark, an old link or a typed path
 *  lands on the same page under its new name rather than on nothing. */
export const load: PageLoad = ({ url }) => {
	redirect(308, legacyDestination(url.pathname) ?? '/sync');
};
