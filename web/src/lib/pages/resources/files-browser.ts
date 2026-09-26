/**
 * The file browser at `/resources/files`: every file of the seller's that
 * one of their machines holds or one of their resources names, and what can
 * be done with it from here.
 *
 * Pure, so it tests without a component. Two accounts meet in these
 * functions and are never mixed: the server's, which is who holds which
 * digest and which resources use it and carries no byte of any file, and
 * this machine's own, which is the library the Teachouse application keeps
 * and only exists where the console runs inside it. A browser has the first
 * and not the second, and nothing here invents the second for it.
 */

import type { LibraryFileView } from '$lib/api';
import { formatBytes } from '$lib/authoring';
import { agoLabel } from '$lib/elapsed';
import type { LibraryEntry } from '$lib/desktop';

/** Where the browser lives, named once for every surface that links to it. */
export const FILES_HREF = '/resources/files';

/** The promise the browser closes with. One sentence; which machine holds
 *  what, and how a copy crosses between them, is the `your-files` guide's. */
export const FILES_STAY_ON_YOUR_MACHINES = 'Your files stay on your own machines.';

/** What the page says in a browser, where no machine is keeping files. */
export const BROWSER_SENTENCE = 'Your files are kept on the machines that run the Teachouse app.';

/** What the page says when the application keeps no library at all. */
export const NOT_KEEPING_SENTENCE = 'This machine is not keeping files.';

/** The label on the setting. */
export const KEEP_LABEL = 'Keep a copy of imported files on this machine';

/** How a holder that is this very machine is named. By role rather than by
 *  name: the seller knows which machine they are sitting at, and a name read
 *  off the device registry is the name of some machine rather than proof it
 *  is this one. */
export const HERE = 'This machine';

/** The page this browser asks for, which is the page the server serves
 *  without being asked. Stated here because the pager does the arithmetic
 *  against it.  */
export const PAGE_SIZE = 25;

/** The confirmation before a file is removed from this machine. */
export function removePrompt(name: string): string {
	return `Remove "${name}" from this machine? Your listing and the copy on the marketplace stay as they are.`;
}

/** The usage line above the list, for the machine that is keeping files. */
export function usageLine(entries: readonly LibraryEntry[], usage: number): string {
	if (entries.length === 0) {
		return 'No files are kept on this machine yet.';
	}
	const count = entries.length === 1 ? '1 file' : `${entries.length} files`;
	return `${count}, ${formatBytes(usage)} on this machine.`;
}

// ------------------------------------------------------------- the filters

export type Availability = 'online' | 'offline' | 'missing';
export type Linked = 'linked' | 'unlinked';

/** What the seller asked to see. Every member rides the URL, because a
 *  narrowed browser is a place worth linking to: a machine's row links here
 *  with its own id, and an import links here with the digest it just wrote. */
export interface FileFilters {
	q: string;
	device: string | null;
	availability: Availability | null;
	linked: Linked | null;
	offset: number;
}

const AVAILABILITIES: readonly Availability[] = ['online', 'offline', 'missing'];
const LINKED: readonly Linked[] = ['linked', 'unlinked'];

export const EMPTY_FILTERS: FileFilters = {
	q: '',
	device: null,
	availability: null,
	linked: null,
	offset: 0
};

/** The filters as the address states them. Anything unreadable is dropped
 *  rather than guessed: a filter nobody can have chosen would narrow the
 *  list to nothing and say the seller has no files. */
export function filtersFromUrl(params: URLSearchParams): FileFilters {
	const availability = params.get('availability');
	const linked = params.get('linked');
	const offset = Number.parseInt(params.get('offset') ?? '', 10);
	return {
		q: params.get('q')?.trim() ?? '',
		device: params.get('device')?.trim() || null,
		availability: AVAILABILITIES.find((one) => one === availability) ?? null,
		linked: LINKED.find((one) => one === linked) ?? null,
		offset: Number.isSafeInteger(offset) && offset > 0 ? offset : 0
	};
}

/** The address one set of filters is read back from. Only what was chosen is
 *  written, so a bare browser has a bare URL and the one a seller copies
 *  says what they are looking at. */
export function filtersToUrl(filters: FileFilters): string {
	const params = new URLSearchParams();
	if (filters.q !== '') {
		params.set('q', filters.q);
	}
	if (filters.device !== null) {
		params.set('device', filters.device);
	}
	if (filters.availability !== null) {
		params.set('availability', filters.availability);
	}
	if (filters.linked !== null) {
		params.set('linked', filters.linked);
	}
	if (filters.offset > 0) {
		params.set('offset', String(filters.offset));
	}
	const query = params.toString();
	return query === '' ? FILES_HREF : `${FILES_HREF}?${query}`;
}

/** What the read asks the server for. `limit` is always stated, so the page
 *  the pager counts in and the page the server cuts cannot drift. */
export function filtersToQuery(filters: FileFilters): {
	q?: string;
	device?: string;
	availability?: Availability;
	linked?: Linked;
	offset: number;
	limit: number;
} {
	return {
		...(filters.q === '' ? {} : { q: filters.q }),
		...(filters.device === null ? {} : { device: filters.device }),
		...(filters.availability === null ? {} : { availability: filters.availability }),
		...(filters.linked === null ? {} : { linked: filters.linked }),
		offset: filters.offset,
		limit: PAGE_SIZE
	};
}

export function filtersActive(filters: FileFilters): boolean {
	return (
		filters.q !== '' ||
		filters.device !== null ||
		filters.availability !== null ||
		filters.linked !== null
	);
}

/** The link a machine's row, or an import, uses to arrive here narrowed to
 *  one machine. */
export function machineFilesHref(device: string): string {
	return filtersToUrl({ ...EMPTY_FILTERS, device });
}

// ---------------------------------------------------------------- the rows

/** Where a file can be reached from, as the row says it.
 *
 *  Read off the server's own account of who holds what, never off bytes.
 *  `missing` is a digest a resource names that no machine of the seller's
 *  reports holding, which is the one state the old whole-library list could
 *  not show at all. */
export function availabilityOf(file: LibraryFileView): Availability {
	if (file.holders.length === 0) {
		return 'missing';
	}
	return file.holders.some((holder) => holder.online) ? 'online' : 'offline';
}

export const AVAILABILITY_LABEL: Record<Availability, string> = {
	online: 'Available now',
	offline: 'Machine offline',
	missing: 'No machine has it'
};

/** What the row offers or says about getting a file onto this machine.
 *
 *  `get` is another live machine holding a file this one does not; `waiting`
 *  is a want this machine placed on a holder that is offline, and names the
 *  machine so the seller knows what to switch on; `elsewhere` is a console
 *  that cannot ask at all — a browser, or a machine that has not checked in
 *  — and so states who holds the file and offers nothing. */
export type TransferLabel =
	| { kind: 'held' }
	| { kind: 'get'; from: string }
	| { kind: 'waiting'; on: string | null }
	| { kind: 'fetching'; from: string }
	| { kind: 'elsewhere'; on: string }
	| { kind: 'missing' };

/** A null local observation means unavailable; false means confirmed absent. */
export function transferLabel(
	file: LibraryFileView,
	thisDevice: string | null,
	heldHere: boolean | null
): TransferLabel {
	if (
		heldHere === true ||
		(heldHere === null && thisDevice !== null && file.holders.some((one) => one.device === thisDevice))
	) {
		return { kind: 'held' };
	}
	const others = file.holders.filter((holder) => holder.device !== thisDevice);
	const online = others.find((holder) => holder.online);
	if (thisDevice !== null && file.wanted_by.includes(thisDevice)) {
		return online === undefined
			? { kind: 'waiting', on: others[0]?.name ?? null }
			: { kind: 'fetching', from: online.name };
	}
	if (others.length === 0) {
		return { kind: 'missing' };
	}
	if (thisDevice === null || heldHere === null) {
		return { kind: 'elsewhere', on: (online ?? others[0]).name };
	}
	return { kind: 'get', from: (online ?? others[0]).name };
}

/** The sentence a transfer label reads as, beside the control it may carry. */
export function transferSentence(label: TransferLabel): string | null {
	switch (label.kind) {
		case 'held':
			return null;
		case 'get':
			return `${label.from} has this file.`;
		case 'waiting':
			return label.on === null ? 'Waiting for a machine that has this file.' : `Waiting for ${label.on} to come online.`;
		case 'fetching':
			return `Copying from ${label.from}…`;
		case 'elsewhere':
			return `${label.on} has this file.`;
		case 'missing':
			return 'None of your machines has this file.';
	}
}

/** Who holds the file, with this machine named by its role rather than by
 *  the name its registration carries. */
export function holderNames(file: LibraryFileView, thisDevice: string | null): string[] {
	return file.holders.map((holder) => (holder.device === thisDevice ? HERE : holder.name));
}

export interface FileRow {
	hash: string;
	/** The catalogue's name for these bytes, or the digest where nothing
	 *  names them. */
	name: string;
	/** Whether `name` is the digest standing in for a name, so the row can
	 *  draw it as the placeholder it is. */
	anonymous: boolean;
	size: string;
	availability: Availability;
	/** The machines holding it, this one named by its role. */
	machines: readonly string[];
	/** When the most recently seen of those machines last checked in, or
	 *  `null` where none holds it or none has been seen. */
	lastSeen: number | null;
	resources: readonly { id: string; title: string }[];
	label: TransferLabel;
	/** What this machine's own library says about the file, where it keeps
	 *  it. Absent for every file this machine does not hold, so no row shows
	 *  a source or a kept date for bytes that are somewhere else. */
	kept: LibraryEntry | null;
}

/** One page of rows. The order is the server's, so paging is stable; this
 *  function only folds in what the machine itself knows. */
export function fileRows(
	files: readonly LibraryFileView[],
	here: {
		thisDevice: string | null;
		kept: ReadonlyMap<string, LibraryEntry> | null;
		/** Each machine's last check-in, by device id, from the devices read. */
		seen?: ReadonlyMap<string, number>;
	}
): FileRow[] {
	return files.map((file) => {
		const kept = here.kept?.get(file.hash) ?? null;
		return {
			hash: file.hash,
			name: file.file_name ?? kept?.file_name ?? `${file.hash.slice(0, 12)}…`,
			anonymous: file.file_name === null && kept === null,
			size: formatBytes(file.byte_len),
			availability: availabilityOf(file),
			machines: holderNames(file, here.thisDevice),
			lastSeen: file.holders.reduce<number | null>((latest, holder) => {
				const at = here.seen?.get(holder.device);
				return at === undefined || (latest !== null && latest >= at) ? latest : at;
			}, null),
			resources: file.resources,
			label: transferLabel(file, here.thisDevice, here.kept === null ? null : kept !== null),
			kept
		};
	});
}

/** The Last seen cell: a machine holding it that is online now says so,
 *  otherwise the age of the latest check-in among its holders. */
export function seenLine(row: FileRow, now: number): string {
	if (row.availability === 'online') {
		return 'Online now';
	}
	if (row.lastSeen === null) {
		return row.availability === 'missing' ? 'No machine' : 'Not seen yet';
	}
	return agoLabel(row.lastSeen, now);
}

/** When this machine kept its copy, for a file it keeps. */
export function keptLine(row: FileRow): string | null {
	return row.kept === null ? null : `Kept here ${new Date(row.kept.kept_at).toLocaleDateString()}`;
}

// --------------------------------------------------------------- the pager

export interface PageWindow {
	/** The one-based position of the first row shown, or 0 for an empty page. */
	from: number;
	to: number;
	total: number;
	hasPrev: boolean;
	hasNext: boolean;
	prevOffset: number;
	nextOffset: number;
}

/** The window one page covers, from what the server itself said it served.
 *  Derived from the answer rather than from what was asked, so a page cut
 *  short by a filter does not claim rows it was not given. */
export function pageWindow(shown: number, total: number, offset: number, limit: number): PageWindow {
	const step = limit > 0 ? limit : PAGE_SIZE;
	const from = shown === 0 ? 0 : offset + 1;
	return {
		from,
		to: offset + shown,
		total,
		hasPrev: offset > 0,
		hasNext: offset + shown < total,
		prevOffset: Math.max(0, offset - step),
		nextOffset: offset + step
	};
}

/** The figure above the list. Only ever said of a read that landed: the
 *  caller shows this in place of nothing, and nothing in place of a read
 *  that failed. */
export function countSentence(window: PageWindow): string {
	if (window.total === 0) {
		return 'No files';
	}
	const noun = window.total === 1 ? 'file' : 'files';
	if (window.from === 0) {
		return `${window.total} ${noun}`;
	}
	return `Showing ${window.from}–${window.to} of ${window.total} ${noun}`;
}
