// The one call this page makes that the shared client cannot: every function
// in `$lib/api` reads a JSON body, and this route answers `text/csv`.
//
// The failure path is the shared one on purpose. A non-2xx answer from this
// route carries the same structured error body as every other route, so it is
// parsed into the same `ApiFailure` the rest of the console already branches
// on, and the page gains no second error vocabulary.

import { ApiFailure, type APIErrorBody } from '$lib/api';
import { filenameFrom, type CsvDocument } from './download';

/** The catalogue export. Same origin and same session cookie as every other
 *  call; the route takes no parameters and answers the caller's own
 *  organisation (`crates/tam-api/src/lib.rs`). */
export const EXPORT_PATH = '/v1/products/export';

/** The cache entry answering only "does the catalogue hold anything at all".
 *
 * Its own key rather than `queryKeys.products`, which holds the whole
 * `ProductHead[]` that Analytics and the console home build: one key under two
 * shapes is a cache collision. It is nested under `products` so the existing
 * `invalidateQueries({ queryKey: queryKeys.products })` call sites reach it by
 * prefix, and this answer refreshes when the catalogue changes.
 *
 * Not in `$lib/query`'s registry because nothing outside this page names it;
 * the registry exists so an invalidation and its query cannot drift, and there
 * is no invalidation of this key to drift from. */
export const EMPTINESS_KEY = ['products', 'export-emptiness'] as const;

/** Fetch the catalogue as one CSV document, named the way the server named it.
 *
 * `fetchImpl` is an argument so the page tests without a network. */
export async function fetchCatalogueCsv(fetchImpl: typeof fetch = fetch): Promise<CsvDocument> {
	const response = await fetchImpl(EXPORT_PATH, { headers: { accept: 'text/csv' } });
	if (!response.ok) {
		let body: APIErrorBody | null = null;
		try {
			body = (await response.json()) as APIErrorBody;
		} catch {
			body = null;
		}
		throw new ApiFailure(response.status, body);
	}
	return {
		blob: await response.blob(),
		filename: filenameFrom(response.headers.get('content-disposition'))
	};
}
