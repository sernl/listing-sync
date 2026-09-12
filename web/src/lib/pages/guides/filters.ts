// How a reader narrows the published guides, and where that narrowing lives.
//
// In the address, not in the component. A filtered list is a place worth
// linking to and coming back to, so the search text, the topic and the tags all
// ride the URL, the query cache is keyed by the same three, and the back button
// walks them. The server answers the narrowing — a title, a body and a
// taxonomy name are searched in the database that holds them — so what is here
// is only the parsing and the writing back.

import type { GuideTaxon } from '$lib/api';

/** What a reader asked for. Topics and tags are ids: the taxonomy is the
 *  server's, and an id is the only handle on a taxon that a rename cannot
 *  break. */
export interface GuideFilters {
	q: string;
	topic: string | null;
	tags: string[];
}

export const NO_FILTERS: GuideFilters = { q: '', topic: null, tags: [] };

/** The filters an address asks for.
 *
 * Canonicalised on the way in — trimmed, de-duplicated, and the tags sorted —
 * so that one narrowing has exactly one cache key and one query string however
 * it was typed. A hand-edited address narrows to what it can rather than
 * asking the server for the same tag twice. */
export function filtersFromUrl(params: URLSearchParams): GuideFilters {
	const topic = params.get('topic')?.trim() ?? '';
	const chosen = new Set<string>();
	// One comma-joined value, because a tag id is a UUID and carries no comma;
	// repeated parameters are read too, so a hand-written address works either
	// way.
	for (const raw of params.getAll('tags')) {
		for (const id of raw.split(',')) {
			const trimmed = id.trim();
			if (trimmed.length > 0) {
				chosen.add(trimmed);
			}
		}
	}
	return {
		q: params.get('q')?.trim() ?? '',
		topic: topic.length === 0 ? null : topic,
		tags: [...chosen].sort()
	};
}

/** The query string these filters ask for, without its `?`. Empty where
 *  nothing is narrowed, so a cleared list lands on a bare path rather than on
 *  a trailing question mark.
 *
 * The tags are sorted here rather than only on the way in, because this is
 * what the cache key is made of: ticking two tags in either order is one
 * narrowing and one answer, and an unsorted key would read it as two and hold
 * the same rows twice. */
export function filterSearch(filters: GuideFilters): string {
	const params = new URLSearchParams();
	if (filters.q.length > 0) {
		params.set('q', filters.q);
	}
	if (filters.topic !== null) {
		params.set('topic', filters.topic);
	}
	if (filters.tags.length > 0) {
		params.set('tags', [...filters.tags].sort().join(','));
	}
	return params.toString();
}

/** The cache key for one narrowing, and null for none.
 *
 * The same three values the server is asked for and nothing else: a key that
 * carried anything more would split the cache on something the answer does not
 * depend on, and a key that carried less would draw one narrowing's rows under
 * another's. */
export function filterKey(filters: GuideFilters): string | null {
	const search = filterSearch(filters);
	return search.length === 0 ? null : search;
}

/** A taxon as a reader sees it named.
 *
 * A retired topic or tag is still shown where a published guide carries it —
 * retirement takes it out of the pickers, not off the guides that reference it
 * — and saying so is what keeps the label intelligible rather than looking
 * like a taxon somebody could have chosen. */
export function taxonLabel(taxon: GuideTaxon): string {
	return taxon.retired ? `${taxon.name} (retired)` : taxon.name;
}

/** The taxa a picker offers: the live ones, plus any retired one the reader is
 *  already filtered by, because dropping the chosen value out of its own
 *  control is how a filter becomes impossible to clear. */
export function offered(taxa: readonly GuideTaxon[], chosen: readonly string[]): GuideTaxon[] {
	return taxa.filter((taxon) => !taxon.retired || chosen.includes(taxon.id));
}
