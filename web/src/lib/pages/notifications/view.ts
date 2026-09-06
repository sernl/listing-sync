// The Notifications page's own view logic: one finished run turned into the
// line the inbox draws for it, and the read mark that line participates in.
// Pure, so it tests without a component.

import type { NotificationCounts, NotificationView } from '$lib/api';
import { agoLabel } from '$lib/elapsed';
import type { NotificationKind } from '$lib/generated/vocab';
import { segments, type Segment } from '$lib/outcome';
import { SHORT_NAME } from '$lib/platforms';

/** What a run of this kind is called in the row's own sentence. A total map
 *  over the generated union rather than the wire value used as a word, so a
 *  kind added to the closed set in Rust stops this file type-checking instead
 *  of rendering a blank where the noun goes. */
const KIND_WORD: Record<NotificationKind, string> = {
	sync: 'sync',
	migration: 'migration',
	import: 'import'
};

/** Where the row opens. A total map for the same reason as `KIND_WORD`, and
 *  the same rule the email's button follows, so a seller who arrives from the
 *  mail and a seller who arrives from the inbox land on one page. */
const KIND_HREF: Record<NotificationKind, (subject: string) => string> = {
	sync: (subject) => `/sync/requests/${subject}`,
	migration: (subject) => `/sync/requests/${subject}`,
	import: (subject) => `/imports/${subject}`
};

/** What the row says where a run changed nothing.
 *
 * A finished run with every count zero has no segment to draw, and a line
 * with nothing on it reads as a row that failed to render rather than as a
 * run that had nothing to do. */
export const NOTHING_TO_CHANGE = 'Nothing to change';

/** The counts as the outcome bar words and colours them.
 *
 * Routed through `segments` rather than re-listed here: the bar is where this
 * console decides that `succeeded` is called "succeeded" and drawn in
 * `--ok`, and a second list would be a second place for that to be decided.
 * A notification carries only settled outcomes, so the unsettled half of the
 * bar's input is zero and drops out of its own filter. `blocked` is fed
 * through `outcome_blocked`, which is the segment that counts it. */
export function outcomes(counts: NotificationCounts): Segment[] {
	return segments({
		total: counts.succeeded +
			counts.degraded +
			counts.failed +
			counts.ambiguous +
			counts.skipped +
			counts.blocked,
		queued: 0,
		in_flight: 0,
		blocked: 0,
		parked: 0,
		settled: 0,
		succeeded: counts.succeeded,
		degraded: counts.degraded,
		failed: counts.failed,
		ambiguous: counts.ambiguous,
		skipped: counts.skipped,
		outcome_blocked: counts.blocked
	});
}

/** One row of the inbox.
 *
 * `href` is null for a kind this bundle has no path for, and the row draws as
 * a card rather than as a link: the console is a static bundle served from the
 * control plane, so a deploy that widens the enum does not rebuild the page a
 * seller already has open, and a made-up path is worse than none. */
export interface NotificationRow {
	id: string;
	title: string;
	href: string | null;
	when: string;
	unread: boolean;
	/** Empty where the run changed nothing, which the page says in words. */
	outcomes: Segment[];
}

function sentence(word: string): string {
	return word.slice(0, 1).toUpperCase() + word.slice(1);
}

/** The row's own sentence.
 *
 * Named by inventory rather than by marketplace, because the three Tes sites
 * are one marketplace and a seller who syncs to two of them would otherwise
 * read the same line twice. An import names no platform: it commits to the
 * catalogue and writes to none. */
export function title(row: NotificationView): string {
	const word = Object.hasOwn(KIND_WORD, row.kind) ? KIND_WORD[row.kind] : 'run';
	if (row.inventory === null || !Object.hasOwn(SHORT_NAME, row.inventory)) {
		return `${sentence(word)} finished`;
	}
	return `${SHORT_NAME[row.inventory]} ${word} finished`;
}

export function href(row: NotificationView): string | null {
	if (!Object.hasOwn(KIND_HREF, row.kind)) {
		return null;
	}
	return KIND_HREF[row.kind](encodeURIComponent(row.subject_id));
}

export function rows(view: readonly NotificationView[], now: number): NotificationRow[] {
	return view.map((one) => ({
		id: one.id,
		title: title(one),
		href: href(one),
		when: agoLabel(one.created_at, now),
		unread: one.read_at === null,
		outcomes: outcomes(one.counts)
	}));
}

/** The id to mark read through, or null where there is nothing to mark.
 *
 * The list is newest first, so the first row is the furthest the mark can
 * reach. Null where every row already carries a `read_at`, so a revisit of a
 * read page posts nothing rather than posting a write the server would answer
 * with zero. */
export function readThrough(view: readonly NotificationView[]): string | null {
	const newest = view[0];
	if (newest === undefined || !view.some((one) => one.read_at === null)) {
		return null;
	}
	return newest.id;
}

/** The eyebrow over the list, counting what has been loaded rather than what
 *  exists: this list is paginated, so a total would mean "on the page you
 *  have loaded" and would visibly jump on Load more. */
export function unreadLine(rows: readonly NotificationRow[]): string {
	const unread = rows.filter((row) => row.unread).length;
	if (unread === 0) {
		return 'Nothing new';
	}
	return unread === 1 ? '1 new' : `${unread} new`;
}

/** The rows held after a page arrives: a first page replaces, a continuation
 *  appends.
 *
 * The replace is the whole point. A retry after a failed "Load more" asks for
 * the first page again, because that is what a cursorless read is, and
 * appending it to rows already held gives every id twice. The list is keyed on
 * `id`, and Svelte's keyed-each throws on a duplicate key in the shipped
 * bundle as well as in dev, so the page would die rather than draw the
 * duplicates. Deciding it here rather than in the component is what lets that
 * be a test instead of a review note. */
export function heldAfter(
	held: readonly NotificationView[],
	incoming: readonly NotificationView[],
	cursor: string | null | undefined
): NotificationView[] {
	return cursor === undefined || cursor === null ? [...incoming] : [...held, ...incoming];
}
