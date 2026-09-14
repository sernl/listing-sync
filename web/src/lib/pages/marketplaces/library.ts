/**
 * The "Files on this machine" section: the originals the Teachouse app kept
 * from imports, sealed on the seller's own machine.
 *
 * Pure, so it tests without a component. The bytes never reach this module;
 * it renders what the application lists.
 */

import type { LibraryFileView } from '$lib/api';
import type { LibraryEntry } from '$lib/desktop';
import type { Marketplace } from '$lib/generated/vocab';
import { CARD_NAME } from './view';

/** What the section says in a browser, where no machine is keeping files. */
export const BROWSER_SENTENCE = 'Files are kept on the machines running the Teachouse app.';

/** What the section says when the application keeps no library at all. */
export const NOT_KEEPING_SENTENCE = 'This machine is not keeping files.';

/** The label on the setting. */
export const KEEP_LABEL = 'Keep imported originals on this machine';

/** A byte count as the seller reads it. Powers of 1024 with one decimal
 *  above kilobytes, and no decimal on bytes. */
export function formatBytes(bytes: number): string {
	if (bytes < 1024) {
		return `${bytes} B`;
	}
	const units = ['KB', 'MB', 'GB', 'TB'];
	let value = bytes / 1024;
	let unit = 0;
	while (value >= 1024 && unit < units.length - 1) {
		value /= 1024;
		unit += 1;
	}
	return `${value.toFixed(1)} ${units[unit]}`;
}

/** The usage line above the list. */
export function usageLine(entries: readonly LibraryEntry[], usage: number): string {
	if (entries.length === 0) {
		return 'No files are kept on this machine yet.';
	}
	const count = entries.length === 1 ? '1 file' : `${entries.length} files`;
	return `${count}, ${formatBytes(usage)} on this machine.`;
}

export interface LibraryRow {
	hash: string;
	name: string;
	marketplace: Marketplace;
	marketplaceName: string;
	size: string;
	keptAt: number;
	kept: string;
}

/** The rows, newest first. */
export function libraryRows(entries: readonly LibraryEntry[]): LibraryRow[] {
	return [...entries]
		.sort((left, right) => right.kept_at - left.kept_at)
		.map((entry) => ({
			hash: entry.hash,
			name: entry.file_name,
			marketplace: entry.marketplace,
			marketplaceName: CARD_NAME[entry.marketplace],
			size: formatBytes(entry.byte_len),
			keptAt: entry.kept_at,
			kept: new Date(entry.kept_at).toLocaleDateString()
		}));
}

/** The confirmation before a file is removed from this machine. */
export function removePrompt(name: string): string {
	return `Remove "${name}" from this machine? Your listing and the marketplace copy are untouched.`;
}

/** What the row offers or says about getting a file onto this machine.
 *
 *  Read off the server's own account of who holds what, never off the
 *  bytes. `get` is another live machine holding a file this one does not;
 *  `waiting` is a want this machine placed on a holder that is offline, and
 *  names the machine so the seller knows what to switch on; `held` says
 *  nothing, because the file is here. */
export type TransferLabel =
	| { kind: 'held' }
	| { kind: 'get'; from: string }
	| { kind: 'waiting'; on: string }
	| { kind: 'fetching'; from: string }
	| { kind: 'nowhere' };

export function transferLabel(
	hash: string,
	thisDevice: string | null,
	files: readonly LibraryFileView[],
	heldHere: ReadonlySet<string>
): TransferLabel {
	if (heldHere.has(hash)) {
		return { kind: 'held' };
	}
	const file = files.find((entry) => entry.hash === hash);
	const others = (file?.holders ?? []).filter((holder) => holder.device !== thisDevice);
	if (others.length === 0) {
		return { kind: 'nowhere' };
	}
	const wanted = thisDevice !== null && (file?.wanted_by ?? []).includes(thisDevice);
	const online = others.find((holder) => holder.online);
	if (wanted) {
		return online === undefined
			? { kind: 'waiting', on: others[0].name }
			: { kind: 'fetching', from: online.name };
	}
	return { kind: 'get', from: (online ?? others[0]).name };
}

/** The sentence a transfer label reads as, beside the button it may carry. */
export function transferSentence(label: TransferLabel): string | null {
	switch (label.kind) {
		case 'held':
		case 'nowhere':
			return null;
		case 'get':
			return `${label.from} holds this file.`;
		case 'waiting':
			return `Waiting for ${label.on}.`;
		case 'fetching':
			return `Copying from ${label.from}…`;
	}
}

/** The files the server lists that this machine does not hold, newest
 *  unknown so listed by name: what "Get on this machine" is offered for. */
export interface ElsewhereRow {
	hash: string;
	name: string;
	size: string;
	label: TransferLabel;
}

export function elsewhereRows(
	files: readonly LibraryFileView[],
	thisDevice: string | null,
	heldHere: ReadonlySet<string>
): ElsewhereRow[] {
	return files
		.filter((file) => !heldHere.has(file.hash))
		.map((file) => ({
			hash: file.hash,
			name: file.file_name ?? `${file.hash.slice(0, 12)}…`,
			size: formatBytes(file.byte_len),
			label: transferLabel(file.hash, thisDevice, files, heldHere)
		}))
		.filter((row) => row.label.kind !== 'nowhere')
		.sort((left, right) => left.name.localeCompare(right.name));
}

/** How many files one machine holds, for its row under Your machines. */
export function holdsSentence(device: string, files: readonly LibraryFileView[]): string | null {
	const count = files.filter((file) => file.holders.some((holder) => holder.device === device))
		.length;
	if (count === 0) {
		return null;
	}
	return count === 1 ? 'holds 1 file' : `holds ${count} files`;
}
