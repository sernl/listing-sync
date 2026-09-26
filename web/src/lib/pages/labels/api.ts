// The two label writes the console had no client for, and the walk that counts
// what a label is on. Beside the page rather than in `$lib/api`, which this
// slice does not edit.

import { ApiFailure, type LabelView, type ProductsPage } from '$lib/api';

/** The fetch wrapper `$lib/api` keeps module-private: the structured error body
 *  parsed into the failure the console already renders, and 204 answered as
 *  nothing.
 *
 *  The header merge is the one deliberate difference from the original. There,
 *  `...init` comes last and so replaces the merged headers whenever a caller
 *  passes any of their own, which drops the `accept` this line means to set. */
async function send<T>(path: string, init: RequestInit = {}): Promise<T> {
	const response = await fetch(path, {
		...init,
		headers: { accept: 'application/json', ...(init.headers ?? {}) }
	});
	if (response.status === 204) {
		return undefined as T;
	}
	if (!response.ok) {
		let body = null;
		try {
			body = await response.json();
		} catch {
			body = null;
		}
		throw new ApiFailure(response.status, body);
	}
	return (await response.json()) as T;
}

/** A label is addressed by its own text, which is why the name is escaped
 *  rather than interpolated: the server refuses a name carrying `/` at the
 *  point one is minted, and everything else a person might type reaches the
 *  route percent-encoded (`crates/tam-api/src/resources.rs:1053-1073`). */
function labelPath(name: string): string {
	return `/v1/labels/${encodeURIComponent(name)}`;
}

/** Renames one label, keeping every resource that carries it.
 *
 *  The answer is the label as stored, colour included: the colour is derived
 *  from the name, so a rename changes it and a page that rendered what it sent
 *  would show a colour this organisation's label does not have. */
export function renameLabel(from: string, to: string): Promise<LabelView> {
	return send<LabelView>(labelPath(from), {
		method: 'PATCH',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify({ name: to })
	});
}

/** Removes one label, and with it every resource's carrying of it. */
export function deleteLabel(name: string): Promise<void> {
	return send<void>(labelPath(name), { method: 'DELETE' });
}

/** The largest page `GET /v1/products` will serve, which it clamps to
 *  (`crates/tam-api/src/resources.rs:62,148-151`).
 *
 *  Asked for by name rather than left to the default of fifty, because this
 *  walk exists only to count: a page of two hundred heads costs one round trip
 *  where four would otherwise, and nothing here reads a row. */
const PAGE_MAX = 200;

/** How many resources carry one label.
 *
 *  Walked rather than read: `GET /v1/labels` answers a name and a colour, and
 *  no route counts the carriers, so the count is the length of the catalogue
 *  narrowed to this label. A count on the labels route would replace this walk
 *  entirely and is recorded as a backend follow-up.
 *
 *  The pages are summed rather than accumulated, because only the figure is
 *  wanted and a large catalogue would otherwise be held in memory once per
 *  label. `$lib/api`'s own `products` is not used: it passes no `limit`, and
 *  the limit is the whole economy of this walk. */
export async function countCarriers(name: string): Promise<number> {
	let total = 0;
	let cursor: string | null = null;
	for (;;) {
		const query = new URLSearchParams({ label: name, limit: String(PAGE_MAX) });
		if (cursor) {
			query.set('cursor', cursor);
		}
		const page: ProductsPage = await send<ProductsPage>(`/v1/products?${query.toString()}`);
		total += page.products.length;
		if (!page.next_cursor) {
			return total;
		}
		cursor = page.next_cursor;
	}
}

/** The cache entry the counts live under. Named here so the page and its
 *  invalidations cannot drift; `$lib/query` holds the shared keys and this one
 *  belongs to this page alone. */
export const LABEL_COUNTS_KEY = ['label-counts'] as const;

/** What one pass over the vocabulary found.
 *
 *  Two fields rather than one map, because a label whose walk failed and a
 *  label that is genuinely absent are different facts and the row says
 *  different words for them. */
export interface CountedLabels {
	counts: Map<string, number>;
	failed: string[];
}

/** Counts every label, a few at a time.
 *
 *  One walk per label is the only count this API offers, so the width is the
 *  difference between a vocabulary that counts in one round trip and one that
 *  counts in as many as it has labels. Four rather than all at once: a seller
 *  with a hundred labels would otherwise open a hundred connections on the one
 *  page load.
 *
 *  A failed walk costs its own label its figure and nothing else. Racing the
 *  workers instead — rejecting the whole pass on the first failure — threw away
 *  every count that had already landed and then had the query retry the entire
 *  pass three more times, which is several complete walks of the catalogue in
 *  answer to one flaky request. */
export async function countAll(
	names: readonly string[],
	count: (name: string) => Promise<number> = countCarriers,
	width = 4
): Promise<CountedLabels> {
	const counts = new Map<string, number>();
	const failed: string[] = [];
	let next = 0;
	async function worker(): Promise<void> {
		for (;;) {
			const index = next;
			next += 1;
			if (index >= names.length) {
				return;
			}
			const name = names[index];
			try {
				counts.set(name, await count(name));
			} catch {
				failed.push(name);
			}
		}
	}
	await Promise.all(Array.from({ length: Math.min(width, names.length) }, worker));
	return { counts, failed };
}

/** What putting a new label on resources did: how many carry it now, and the
 *  refusal that stopped it part way, if one did. */
export interface Labelled {
	count: number;
	refusal: string | null;
}

/** Puts one label on each of `products`, keeping what each already carries.
 *
 *  There is no empty label to create — a label exists while a resource carries
 *  it — so creating one is adding it to resources. Each resource's own labels
 *  are read and the new one appended, because the route replaces the whole
 *  set, and a system label is left out of what is written back because the
 *  route refuses a set that names one (it keeps those itself). Stops at the
 *  first refusal and says how far it got, since the writes before it stand. */
export async function labelResources(
	products: readonly string[],
	name: string,
	io: {
		read: (product: string) => Promise<{ labels: LabelView[] }>;
		write: (product: string, labels: string[]) => Promise<unknown>;
	}
): Promise<Labelled> {
	let count = 0;
	for (const product of products) {
		try {
			const held = await io.read(product);
			const own = held.labels.filter((label) => !label.system).map((label) => label.name);
			if (!own.some((one) => one.toLowerCase() === name.toLowerCase())) {
				own.push(name);
			}
			await io.write(product, own);
			count += 1;
		} catch (failure) {
			const reason =
				failure instanceof ApiFailure ? failure.message : 'That resource could not be labelled.';
			return { count, refusal: reason };
		}
	}
	return { count, refusal: null };
}
