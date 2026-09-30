// The inbox's view logic, shared by the Notifications page and the bell: one
// row -- a finished run or a kept notice -- turned into the line both draw for
// it, and what a Mark read or a Dismiss does to the rows and the unread count.
// Pure, so it tests without a component.

import type { NotificationCounts, NotificationView } from '$lib/api';
import { agoLabel } from '$lib/elapsed';
import type { NoticeTone, NotificationKind } from '$lib/generated/vocab';
import { segments, type Segment } from '$lib/outcome';
import { SHORT_NAME } from '$lib/platforms';

/** What a run of this kind is called in the row's own sentence. A total map
 *  over the generated union rather than the wire value used as a word, so a
 *  kind added to the closed set in Rust stops this file type-checking instead
 *  of rendering a blank where the noun goes. */
const KIND_WORD: Record<NotificationKind, string> = {
	sync: 'update',
	migration: 'move',
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
		total:
			counts.succeeded +
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

/** One row of the inbox, as the page and the bell both draw it.
 *
 * `href` is null for a notice, which names nothing to open, and for a run of
 * a kind this bundle has no path for: the console is a static bundle served
 * from the control plane, so a deploy that widens the enum does not rebuild
 * the page a seller already has open, and a made-up path is worse than none. */
export interface NotificationRow {
	id: string;
	tone: NoticeTone;
	title: string;
	/** One line under the title: a run's counts in words, or the notice's own
	 *  text. Empty where a notice said everything in its title. */
	line: string;
	href: string | null;
	when: string;
	unread: boolean;
	/** A run's counts as the outcome bar draws them; empty for a notice, and
	 *  for a run that changed nothing, which `line` says in words. */
	outcomes: Segment[];
}

function sentence(word: string): string {
	return word.slice(0, 1).toUpperCase() + word.slice(1);
}

/** The row's own sentence.
 *
 * A notice says its own. A run is named by the inventory it wrote to, which
 * is the only thing that tells two runs apart in a list; an import names no
 * platform, because it commits to the catalogue and writes to none. */
export function title(row: NotificationView): string {
	if (row.source === 'notice') {
		return row.title;
	}
	const word = Object.hasOwn(KIND_WORD, row.kind) ? KIND_WORD[row.kind] : 'task';
	if (row.inventory === null || !Object.hasOwn(SHORT_NAME, row.inventory)) {
		return `${sentence(word)} finished`;
	}
	return `${SHORT_NAME[row.inventory]} ${word} finished`;
}

export function href(row: NotificationView): string | null {
	if (row.source === 'notice' || !Object.hasOwn(KIND_HREF, row.kind)) {
		return null;
	}
	return KIND_HREF[row.kind](encodeURIComponent(row.subject_id));
}

export function rows(view: readonly NotificationView[], now: number): NotificationRow[] {
	return view.map((one) => {
		const counted = one.source === 'run' ? outcomes(one.counts) : [];
		return {
			id: one.id,
			tone: one.tone,
			title: title(one),
			line:
				one.source === 'notice'
					? one.body
					: counted.length === 0
						? NOTHING_TO_CHANGE
						: counted.map((segment) => `${segment.count} ${segment.label}`).join(' · '),
			href: href(one),
			when: agoLabel(one.created_at, now),
			unread: one.read_at === null,
			outcomes: counted
		};
	});
}

/** The eyebrow over the list, counting every unread row the seller has --
 *  the figure the bell's badge shows -- rather than only the pages loaded. */
export function unreadLine(unread: number): string {
	if (unread === 0) {
		return 'Nothing new';
	}
	return unread === 1 ? '1 new' : `${unread} new`;
}

/** What the bell's badge prints: nothing at zero, and a cap past 99 so the
 *  circle stays a circle. */
export function badgeLabel(unread: number): string | null {
	if (unread <= 0) {
		return null;
	}
	return unread > 99 ? '99+' : String(unread);
}

/** One write to the inbox, as the page and the bell apply it to what they
 *  hold before the server answers. */
export type InboxChange =
	| { kind: 'read'; id: string; at: number }
	| { kind: 'readAll'; at: number }
	| { kind: 'dismiss'; id: string }
	| { kind: 'dismissRead' };

/** The rows and the unread count after one change, so a pressed Mark read
 *  or Dismiss moves the row and the badge together at once. Rows the change
 *  does not reach are returned as they were. */
export function inboxAfter(
	held: { notifications: readonly NotificationView[]; unread: number },
	change: InboxChange
): { notifications: NotificationView[]; unread: number } {
	const list = held.notifications;
	switch (change.kind) {
		case 'read': {
			const target = list.find((one) => one.id === change.id);
			if (target === undefined || target.read_at !== null) {
				return { notifications: [...list], unread: held.unread };
			}
			return {
				notifications: list.map((one) =>
					one.id === change.id ? { ...one, read_at: change.at } : one
				),
				unread: Math.max(0, held.unread - 1)
			};
		}
		case 'readAll':
			return {
				notifications: list.map((one) =>
					one.read_at === null ? { ...one, read_at: change.at } : one
				),
				unread: 0
			};
		case 'dismiss': {
			const target = list.find((one) => one.id === change.id);
			return {
				notifications: list.filter((one) => one.id !== change.id),
				unread:
					target !== undefined && target.read_at === null
						? Math.max(0, held.unread - 1)
						: held.unread
			};
		}
		case 'dismissRead':
			return {
				notifications: list.filter((one) => one.read_at === null),
				unread: held.unread
			};
	}
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
