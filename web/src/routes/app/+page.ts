import { redirect } from '@sveltejs/kit';
import type { PageLoad } from './$types';

/** The console's home is the Resources list, which lives at `/inventory`.
 *
 * A redirect rather than a second rendering of the same board: `sectionFor`
 * answers `null` for this path, so a list drawn here would come with no
 * Crosslist navigation beside it, and one screen reachable at two addresses is
 * two things to keep true. Not a `LEGACY_REDIRECTS` entry, because `/app` is
 * where the public site and the desktop app both land and is a destination the
 * navigation model still names. */
export const load: PageLoad = () => {
	redirect(308, '/inventory');
};
