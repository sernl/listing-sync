// The Ctrl-K palette's own logic: which key does what, and what the popup has
// to say for each answer a search can have. Pure, so it tests without a
// browser, and separate from the component for the reason the shared-sheet
// sweep gives: a substitution written in a template is reachable only by
// rendering the page, and this one has to distinguish a catalogue that failed
// to read from one that holds nothing matching.

import type { MappingHead, ProductHead } from '$lib/api';
import { INVENTORY_ORDER, formatPrice, matchesQuery } from '$lib/listings-view';
import { SHORT_NAME } from '$lib/platforms';
import { standingOf } from '$lib/tes-portfolio';

/** How many matches the popup shows at once.
 *
 * A palette is read at a glance rather than scrolled, and eight is what fits
 * above the fold on the phone sheet. Where more match, the popup says so and
 * offers the board, rather than silently ending the list. */
export const RESULTS_SHOWN_MAX = 8;

export interface PaletteResult {
	id: string;
	title: string;
	/** The one secondary line: what it costs, and which marketplaces show it. */
	meta: string;
	href: string;
	/** Where this resource's cover is fetched from, or null where it has none.
	 *
	 *  Read straight off the list view and never composed here: the server
	 *  names the URL under the version the request came in on, so a client that
	 *  built the path would be a second place the route is written down and
	 *  would get the version wrong the first time one changed. Null rather than
	 *  optional so the row always has the field and the absent case is a value
	 *  a test can pass rather than a property it can forget. */
	cover: string | null;
}

/**
 * What the popup is saying right now.
 *
 * Five answers rather than a list and a boolean, because a catalogue that was
 * never read and a catalogue holding nothing matching are different facts and
 * a seller acts differently on each. `reading` is the third answer the sweep
 * note asks for: not "no matches", which would be a claim this client cannot
 * support until the read lands.
 */
export type PaletteView =
	/** Open, nothing typed. */
	| { kind: 'blank' }
	/** Typed, and the catalogue is not here yet. */
	| { kind: 'reading' }
	/** Typed, and there is no catalogue to search: nothing was ever read. */
	| { kind: 'failed' }
	| { kind: 'none'; query: string; stale: boolean }
	| {
			kind: 'results';
			rows: PaletteResult[];
			/** How many matched in total, which is a fact only because this
			 *  client holds the whole catalogue to count. A source serving one
			 *  page at a time answers null, and the popup then says nothing
			 *  about a total rather than reporting the page's own length as one. */
			total: number | null;
			stale: boolean;
	  };

export interface PaletteInput {
	query: string;
	/** The catalogue, or null where it has not been read yet. */
	catalogue: readonly ProductHead[] | null;
	/** Every mapping, or null where they have not been read. A resource is
	 *  still named without them; only its meta line is poorer. */
	mappings: readonly MappingHead[] | null;
	/** Whether the newest catalogue read failed. Read together with
	 *  `catalogue`, because the two co-occur: a failed background refetch
	 *  leaves the previously read rows in place, and those rows are still
	 *  searchable. */
	failed: boolean;
}

/** Which marketplaces are showing each resource, shortest name each, in the
 *  chip strip's own order.
 *
 *  Only a live listing counts as showing. A draft or an unsent mapping is a
 *  resource the marketplace is not displaying, and naming it here would tell a
 *  seller their resource is on sale where it is not. */
export function showingByProduct(mappings: readonly MappingHead[]): Map<string, string[]> {
	const live = new Map<string, Set<string>>();
	for (const mapping of mappings) {
		if (standingOf(mapping) !== 'live') {
			continue;
		}
		const carried = live.get(mapping.product) ?? new Set<string>();
		carried.add(mapping.inventory);
		live.set(mapping.product, carried);
	}
	const named = new Map<string, string[]>();
	for (const [product, inventories] of live) {
		named.set(
			product,
			INVENTORY_ORDER.filter((inventory) => inventories.has(inventory)).map(
				(inventory) => SHORT_NAME[inventory]
			)
		);
	}
	return named;
}

/** A resource's secondary line.
 *
 * Two facts, both of them the board's own: what it costs and who is showing
 * it. A resource no marketplace shows says nothing rather than "nowhere",
 * matching `metaLine`; that also means an unread mapping list reads the same
 * as a resource on no marketplace, which is sound here only because neither
 * says anything a seller could act on. A count would not be. */
function metaOf(product: ProductHead, showing: readonly string[]): string {
	const parts = [formatPrice(product.price)];
	if (showing.length > 0) {
		parts.push(showing.join(', '));
	}
	return parts.join(' · ');
}

/**
 * The cover a row should actually draw.
 *
 * A URL that failed to load is not drawn again, so a deleted or unreachable
 * cover falls back to the same placeholder the absent case draws rather than
 * leaving the browser's broken-image glyph in a 32px box.
 *
 * The failure is remembered by URL rather than by row, which is `RowCard`'s
 * rule generalised from one row to a list of them: that component holds a
 * single `failed` string because it draws a single cover, and a palette
 * recomputes its rows on every keystroke, so a row index means nothing across
 * two answers while a URL that 404s stays gone. Keying by URL also means a row
 * whose cover changes is tried afresh instead of being punished for the old
 * one.
 */
export function coverToDraw(cover: string | null, failed: ReadonlySet<string>): string | null {
	return cover !== null && !failed.has(cover) ? cover : null;
}

/** Where a result opens. The resource's own page, which is the whole point of
 *  the palette: the founder's item asks for the resource rather than for the
 *  board filtered down to it. */
export function resultHref(id: string): string {
	return `/resources/${id}`;
}

/**
 * The matches, best first.
 *
 * Two rules, because a palette that answers in catalogue order puts the oldest
 * resource at the top and the seller's own word for the thing they are looking
 * for is usually how its title starts. Titles beginning with the query lead;
 * within each group the most recently updated leads, which is the board's own
 * default order.
 */
export function rankMatches(
	catalogue: readonly ProductHead[],
	query: string
): readonly ProductHead[] {
	const needle = query.trim().toLowerCase();
	const matched = catalogue.filter((product) => matchesQuery(product.title, needle));
	const leads = (product: ProductHead) => product.title.toLowerCase().startsWith(needle);
	return [...matched].sort((left, right) => {
		const byLead = Number(leads(right)) - Number(leads(left));
		return byLead === 0 ? right.updated_at - left.updated_at : byLead;
	});
}

/**
 * What the popup shows, for every state its two reads can be in.
 *
 * Whether there is anything to search is asked before whether the last read
 * failed, and the order is the whole of the distinction: a read that failed
 * having never landed leaves nothing to answer with, while one that failed
 * refreshing rows already held leaves those rows perfectly searchable. Refusing
 * to search a catalogue sitting in memory is a worse answer than the "no
 * matches" this popup is careful not to fabricate, so the failure becomes a
 * staleness note carried beside the rows rather than a refusal in place of
 * them.
 */
export function paletteView(input: PaletteInput): PaletteView {
	const query = input.query.trim();
	if (query.length === 0) {
		return { kind: 'blank' };
	}
	if (input.catalogue === null) {
		return input.failed ? { kind: 'failed' } : { kind: 'reading' };
	}
	// How this branch is reached, recorded because the ordinary path to it is
	// closed: both reads are `staleTime: Infinity`, so opening the palette a
	// second time refetches nothing and cannot fail. What remains is an
	// invalidation followed by a failing read -- the board mutates a resource,
	// invalidates `queryKeys.catalogue`, and the refetch that starts does not
	// land. The rows already held stay in the cache alongside the error, which
	// is what `stale` is reporting. It is not a dead branch, and a later reader
	// who concludes it is would be removing the only handling of that case.
	const stale = input.failed;
	const ordered = rankMatches(input.catalogue, query);
	if (ordered.length === 0) {
		return { kind: 'none', query, stale };
	}
	const showing =
		input.mappings === null ? new Map<string, string[]>() : showingByProduct(input.mappings);
	return {
		kind: 'results',
		rows: ordered.slice(0, RESULTS_SHOWN_MAX).map((product) => ({
			id: product.id,
			title: product.title,
			meta: metaOf(product, showing.get(product.id) ?? []),
			href: resultHref(product.id),
			// Absent and null are one answer here: both mean there are no bytes
			// behind a request, so the row draws its placeholder rather than
			// asking for a cover that is not there.
			cover: product.cover ?? null
		})),
		total: ordered.length,
		stale
	};
}

/** Whether the rows on show were read before a failure and may be behind the
 *  catalogue. Its own function because two of the five answers can carry it and
 *  the template would otherwise ask twice. */
export function isStale(view: PaletteView): boolean {
	return (view.kind === 'results' || view.kind === 'none') && view.stale;
}

/** How many of a view's results a key can move through. Zero for every state
 *  that shows none, so the arrows and Enter are inert there by construction
 *  rather than by a guard each caller remembers. */
export function resultCount(view: PaletteView): number {
	return view.kind === 'results' ? view.rows.length : 0;
}

export interface PaletteKeyPress {
	key: string;
	/** How many results the popup is showing. */
	count: number;
	/** Which of them is highlighted. */
	highlighted: number;
	/** Whether an input method is mid-composition. Required rather than
	 *  optional so a later call site cannot forget it and still compile. */
	isComposing: boolean;
}

/**
 * What a key does to an open palette.
 *
 * A returned action rather than a mutation, so the rules are testable without
 * a component and the component only reports what happened -- the shape
 * `menu-dismissal` already uses for why a menu closes.
 */
export type PaletteAction =
	| { kind: 'move'; to: number }
	| { kind: 'open'; index: number }
	| { kind: 'close' }
	| { kind: 'ignore' };

export function keyAction(press: PaletteKeyPress): PaletteAction {
	const { key, count, highlighted } = press;
	// A key pressed mid-composition belongs to the input method, whatever it
	// is. Composing with a Japanese or Chinese keyboard, the arrows walk the
	// candidate list and Enter commits the candidate; taking either would move
	// the highlight under the seller and open a resource they never chose.
	// Escape is in here too: mid-composition it cancels the composition, and
	// closing the palette on it would throw away what they were typing.
	if (press.isComposing) {
		return { kind: 'ignore' };
	}
	if (key === 'Escape') {
		// Closing does not depend on there being results: Escape is the seller's
		// way out of a popup that answered nothing, which is when they most want
		// it.
		return { kind: 'close' };
	}
	if (count <= 0) {
		return { kind: 'ignore' };
	}
	const at = clampHighlight(highlighted, count);
	switch (key) {
		case 'ArrowDown':
			return { kind: 'move', to: (at + 1) % count };
		case 'ArrowUp':
			return { kind: 'move', to: (at - 1 + count) % count };
		case 'Enter':
			return { kind: 'open', index: at };
		// Home and End are deliberately absent, and belong to the caret: this
		// is an editable combobox, where the ARIA pattern binds those two to
		// the list only in the select-only variant. Taking them would leave a
		// seller unable to reach the start of a typo they can see.
		default:
			return { kind: 'ignore' };
	}
}

/** The highlight held inside a list that has just changed length.
 *
 * The results are re-read on every keystroke, so an index taken against the
 * previous answer can point past the current one; opening whatever that index
 * lands on would open a resource the seller never saw highlighted. */
export function clampHighlight(highlighted: number, count: number): number {
	if (count <= 0) {
		return 0;
	}
	if (!Number.isFinite(highlighted) || highlighted < 0) {
		return 0;
	}
	return Math.min(Math.floor(highlighted), count - 1);
}

/** Whether this key press is the one that opens the palette. */
export function opensPalette(press: {
	key: string;
	ctrlKey: boolean;
	metaKey: boolean;
}): boolean {
	return (press.ctrlKey || press.metaKey) && press.key.toLowerCase() === 'k';
}

/** The head the seller reads and types into, which `shell.css` draws at 52px.
 *  Named here because the placement below is arithmetic about that band and a
 *  height guessed at would place the input somewhere the stylesheet does not
 *  draw it. */
const HEAD_HEIGHT = 52;

/** The margin kept between the sheet and both ends of the visible band. */
const GUTTER = 12;

/** Where the phone sheet sits, against the band the browser reports as visible.
 *
 * The head's middle sits a third of the way down that band, and never above
 * the gutter. `maxHeight` is what is left down to the far gutter, floored at a
 * head: that floor is the whole of the guarantee that a band too short to hold
 * the sheet still answers with one whose field can be read and typed into.
 *
 * A third rather than the middle because with the keyboard up the band is
 * roughly the top 60% of the screen, and the exact middle of that leaves room
 * for about two result rows. Fixing the top and letting the sheet grow
 * downwards is also what keeps the input still as matches arrive, which a
 * vertically centred box cannot do.
 *
 * `offsetTop` is added to the answer because a modal dialog is positioned
 * against the layout viewport while the band is reported against the visual
 * one; on an unzoomed page those agree and on a pinch-zoomed one they do not. */
export function paletteAnchor(band: { height: number; offsetTop: number }): {
	top: number;
	maxHeight: number;
} {
	const within = Math.round(Math.max(GUTTER, band.height / 3 - HEAD_HEIGHT / 2));
	return {
		top: within + Math.round(band.offsetTop),
		// Floored, so the rounding cannot push the foot past the gutter it was
		// placed against: every band a browser reports is fractional.
		maxHeight: Math.max(HEAD_HEIGHT, Math.floor(band.height - within - GUTTER))
	};
}
