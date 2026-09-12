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
 *  call; a bare path answers the caller's own whole catalogue, and the two
 *  parameters below narrow it (`crates/tam-api/src/export.rs`). */
export const EXPORT_PATH = '/v1/products/export';

/** What the seller asked for: the whole catalogue, one collection, or the
 *  resources they ticked.
 *
 *  Three shapes rather than two nullable fields, so "all of it" cannot arrive
 *  alongside a selection that contradicts it -- the same reason
 *  `MigrationSelection` is a sum. */
export type ExportSelection = null | { collection: string } | { products: readonly string[] };

/** The route and the selection, as one address.
 *
 *  The collection is a single identifier and the products are comma-joined,
 *  because the separator is part of the address's own grammar: each identifier
 *  is escaped and the commas are left as commas, which is what the Resources
 *  board's own migration link does. An empty product list asks for the whole
 *  catalogue rather than for nothing -- a selection of no resources is not a
 *  request the server has an answer for. */
export function exportPath(selection: ExportSelection): string {
	if (selection === null) {
		return EXPORT_PATH;
	}
	if ('collection' in selection) {
		return `${EXPORT_PATH}?collection=${encodeURIComponent(selection.collection)}`;
	}
	if (selection.products.length === 0) {
		return EXPORT_PATH;
	}
	const products = selection.products.map((id) => encodeURIComponent(id)).join(',');
	return `${EXPORT_PATH}?products=${products}`;
}

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
export async function fetchCatalogueCsv(
	fetchImpl: typeof fetch = fetch,
	selection: ExportSelection = null
): Promise<CsvDocument> {
	const response = await fetchImpl(exportPath(selection), { headers: { accept: 'text/csv' } });
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
