import { error } from '@sveltejs/kit';
import type { PageLoad } from './$types';

/** An address no route matches, answered inside the console rather than
 *  outside it: behind the same session gate, so a signed-out visit is sent to
 *  sign in as it always was, and drawn in the console's shell by its error
 *  page rather than on a bare signed-out frame. */
export const load: PageLoad = () => {
	error(404, 'Not found');
};
