import { redirect } from '@sveltejs/kit';
import { legacyDestination } from '$lib/nav';
import type { PageLoad } from './$types';

/** The create flow moved to `/resources/new`. A static segment beats `[id]`
 *  beside it, so this path needs its own stub rather than falling through. */
export const load: PageLoad = ({ url }) => {
	redirect(308, legacyDestination(url.pathname) ?? '/resources/new');
};
