// What the console has to re-read once a resource has been created.
//
// A function rather than two lines inside the form's submit handler, and the
// reason is testability rather than tidiness: inlined, the only thing that
// could check the invalidation was a test that performed it itself, which
// would go on passing with the form's own call deleted. Here the call site and
// the assertion name the same function, so deleting it fails the suite.

import type { QueryClient } from '@tanstack/svelte-query';
import { queryKeys } from '$lib/query';

/**
 * Marks everything a create changed as stale, so the next screen reads it.
 *
 * Both keys, because a create writes both: the resource joins the catalogue,
 * and any marketplace chosen at create time becomes a mapping. `products` is a
 * prefix, so it reaches the board's label-keyed entries and the export page's
 * emptiness probe as well as the unfiltered catalogue.
 *
 * Awaited rather than fired and forgotten: the form navigates to the new
 * resource straight afterwards, and a navigation that outran the invalidation
 * would land on a screen reading the catalogue as it was before the write.
 */
export async function refreshAfterCreate(client: QueryClient): Promise<void> {
	await client.invalidateQueries({ queryKey: queryKeys.products });
	await client.invalidateQueries({ queryKey: queryKeys.mappings });
}
