import { redirect } from '@sveltejs/kit';
import { legacyDestination } from '$lib/nav';
import type { PageLoad } from './$types';

/** The reconciliation queue moved to `/reconciliation`. */
export const load: PageLoad = ({ url }) => {
	redirect(308, legacyDestination(url.pathname) ?? '/reconciliation');
};
