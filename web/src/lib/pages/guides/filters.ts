// How a reader narrows the published guides, and where that narrowing lives.
//
// In the address, not in the component. A filtered list is a place worth
// linking to and coming back to, so the search text, the topic and the tags all
// ride the URL, the query cache is keyed by the same three, and the back button
// walks them. The server answers the narrowing — a title, a body and a
// taxonomy name are searched in the database that holds them — so what is here
// is only the parsing and the writing back.

import type { GuideTaxon } from '$lib/api';

/** The orders the published listing can be read in. The server's vocabulary,
 *  and it answers which one it applied: a word it does not sort by comes back
 *  as the order it used instead. */
export type GuideSort = 'title' | 'newest';

/** The order a reader gets without asking: the alphabet, which is the order
 *  somebody looking for a title they half remember can navigate. */
export const DEFAULT_SORT: GuideSort = 'title';

/** What the Order by control offers, in the order it offers it. */
export const SORTS: ReadonlyArray<{ id: GuideSort; label: string }> = [
	{ id: 'title', label: 'Title A–Z' },
	{ id: 'newest', label: 'Newest first' }
];

/** How many guides one page holds. The server's page size, mirrored here only
 *  to say which rows of the whole set the page being read is — the rows
 *  themselves are always the ones the server sent. */
export const PAGE_ROWS = 25;

/** The highest page the server will read, held here so an address cannot ask
 *  for an ordinal the server is going to clamp and then disagree with. */
const PAGE_MAX = 10_000;

/** What a reader asked for. Topics and tags are ids: the taxonomy is the
 *  server's, and an id is the only handle on a taxon that a rename cannot
 *  break.
 *
 * The order and the page are here too, because they are part of the question
 * the server is asked and therefore part of what the address has to say and
 * what the cache is keyed by. They are not part of the *narrowing*, which is
 * what [`narrowedBy`] answers: changing a filter starts again at page one,
 * and a page is not something "Clear filters" is about. */
export interface GuideFilters {
	q: string;
	topic: string | null;
	tags: string[];
	sort: GuideSort;
	/** One-based, as a reader counts pages and as the server echoes them. */
	page: number;
}

export const NO_FILTERS: GuideFilters = {
	q: '',
	topic: null,
	tags: [],
	sort: DEFAULT_SORT,
	page: 1
};

/** The filters an address asks for.
 *
 * Canonicalised on the way in — trimmed, de-duplicated, and the tags sorted —
 * so that one narrowing has exactly one cache key and one query string however
 * it was typed. A hand-edited address narrows to what it can rather than
 * asking the server for the same tag twice.
 *
 * The order is a closed vocabulary and the page is a position, so an address
 * naming an order this console does not offer, or a page that is not a page,
 * is read as the default. That is the console's own canonicalisation rather
 * than the server's behaviour: the server refuses a `sort` outside its two
 * words, exactly as it refuses a topic id no taxon holds, and the reason this
 * reads the word against `SORTS` first is so a hand-edited address is asked
 * about in a vocabulary the server has instead of being sent back as a
 * refusal a reader cannot act on. A page is defaulted on both sides: it names
 * a position, and a position that cannot be read is the first one. */
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
	const order = params.get('sort')?.trim();
	const asked = Number(params.get('page')?.trim());
	return {
		q: params.get('q')?.trim() ?? '',
		topic: topic.length === 0 ? null : topic,
		tags: [...chosen].sort(),
		sort: SORTS.some((sort) => sort.id === order) ? (order as GuideSort) : DEFAULT_SORT,
		page: Number.isInteger(asked) && asked >= 1 ? Math.min(asked, PAGE_MAX) : 1
	};
}

/** The query string these filters ask for, without its `?`. Empty where
 *  nothing is narrowed, so a cleared list lands on a bare path rather than on
 *  a trailing question mark.
 *
 * The tags are sorted here rather than only on the way in, because this is
 * what the cache key is made of: ticking two tags in either order is one
 * narrowing and one answer, and an unsorted key would read it as two and hold
 * the same rows twice.
 *
 * The default order and the first page are written as absence for the same
 * reason: `?sort=title&page=1` and a bare path are one question, and two
 * spellings of one question are two cache entries and two addresses for one
 * place. */
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
	if (filters.sort !== DEFAULT_SORT) {
		params.set('sort', filters.sort);
	}
	if (filters.page > 1) {
		params.set('page', String(filters.page));
	}
	return params.toString();
}

/** The cache key for one narrowing, order and page, and null for the bare
 *  list.
 *
 * Exactly what the server is asked for and nothing else: a key that carried
 * anything more would split the cache on something the answer does not depend
 * on, and a key that carried less would draw one answer's rows under another's
 * — which for the page is the whole point, since page two of a search is a
 * different set of rows from page one of it and must not be served as one. */
export function filterKey(filters: GuideFilters): string | null {
	const search = filterSearch(filters);
	return search.length === 0 ? null : search;
}

/** Whether the reader has narrowed the corpus, as against reordered it or
 *  walked it.
 *
 * This is what "Clear filters" undoes and what tells an empty search from an
 * empty shelf. An order is not a narrowing — every guide is still in the
 * answer — and neither is a page. */
export function narrowedBy(filters: GuideFilters): boolean {
	return filters.q.length > 0 || filters.topic !== null || filters.tags.length > 0;
}

/** Which rows of the whole answer this page is, in the words a reader counts
 *  in: `1–25 of 143 guides`.
 *
 * `total` is the server's count over the whole narrowing and `rows` is what
 * this page actually carries, so the range is the range of rows that are on
 * the screen and the total is never the length of them. A page past the end of
 * a set that does have guides says so rather than claiming to be a range. */
export function pageSummary(page: number, rows: number, total: number): string {
	const word = total === 1 ? 'guide' : 'guides';
	if (total === 0) {
		return `No ${word}`;
	}
	if (rows === 0) {
		return `No guides on this page of ${total} ${word}`;
	}
	const first = (page - 1) * PAGE_ROWS + 1;
	return `${first}\u2013${first + rows - 1} of ${total} ${word}`;
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
