import { redirect } from '@sveltejs/kit';
import { legacyDestination } from '$lib/nav';
import type { PageLoad } from './$types';

/** One run's page moved to `/sync/<id>`, carrying its identifier through. */
export const load: PageLoad = ({ url }) => {
	redirect(308, legacyDestination(url.pathname) ?? '/sync');
};
