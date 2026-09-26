// The Files panel's own rules and state, kept out of the component so both
// test without a DOM. Every function here is pure; the component owns the
// requests and hands the results back as events.

import { ApiFailure, type FileView } from '$lib/api';
import { platformTitle } from '$lib/platforms';
import type { FileRole, InventoryId } from '$lib/generated/vocab';
import { sentenceFor } from './refusal';

/**
 * The word a seller reads for each file role.
 *
 * A total map rather than the wire value rendered directly, for the reason
 * `SESSION_LABEL` in `pages/marketplaces/machines.ts` gives for the same
 * pattern: the pill uppercases whatever it is handed, so `role` reaching it
 * unmapped put PAYLOAD — an identifier, and a word the founder's brief bans —
 * on a teacher's screen for every buyer-download row. The wire keeps
 * `payload` / `preview` / `cover`; only the words move.
 *
 * Totality is the point rather than a nicety: a role added in Rust fails this
 * type check instead of leaking its identifier onto the screen.
 */
export const ROLE_WORD: Record<FileRole, string> = {
	payload: 'File',
	preview: 'Preview',
	cover: 'Thumbnail'
};

export type FileVerb = 'add' | 'replace' | 'remove';

/**
 * What the panel is doing.
 *
 * `file` is the row an action belongs to, and `null` is the add, which belongs
 * to no row. One action at a time, so a seller cannot remove the file an
 * upload is in the middle of replacing.
 */
export type FileAction =
	| { kind: 'idle' }
	| { kind: 'confirming'; verb: 'replace' | 'remove'; file: string; chosen: string | null }
	| { kind: 'uploading'; verb: 'add' | 'replace'; file: string | null; fraction: number }
	| { kind: 'writing'; verb: FileVerb; file: string | null }
	| { kind: 'refused'; verb: FileVerb; file: string | null; sentence: string };

export type FileEvent =
	| { kind: 'ask'; verb: 'replace' | 'remove'; file: string; chosen: string | null }
	| { kind: 'cancel' }
	| { kind: 'send'; verb: FileVerb; file: string | null }
	| { kind: 'progress'; fraction: number }
	| { kind: 'stored' }
	| { kind: 'settled' }
	| { kind: 'failed'; sentence: string };

export const IDLE: FileAction = { kind: 'idle' };

/** An action with bytes or a write still in flight, which is the only shape
 *  carrying a verb and a row for a refusal to be attributed to. */
export type RunningAction = Extract<FileAction, { kind: 'uploading' | 'writing' }>;

/** Whether something is in flight, which is what disables every control. A
 *  predicate rather than a boolean so the narrowing it performs is available
 *  to every caller instead of being repeated at each one. */
export function busy(action: FileAction): action is RunningAction {
	return action.kind === 'uploading' || action.kind === 'writing';
}

/**
 * The next state, or the one given back unchanged.
 *
 * A machine rather than a handful of `$state` assignments because the two
 * faults it prevents are both invisible until a seller hits them: a second
 * action starting while one is running, which would remove the row an upload
 * is about to write to, and a refusal from one row surfacing on another.
 */
export function advance(action: FileAction, event: FileEvent): FileAction {
	switch (event.kind) {
		case 'ask':
			// A refusal is not a running action, so the seller can try again
			// from it without dismissing it first.
			return busy(action)
				? action
				: { kind: 'confirming', verb: event.verb, file: event.file, chosen: event.chosen };
		case 'cancel':
			return busy(action) ? action : IDLE;
		case 'send':
			if (busy(action)) {
				return action;
			}
			// A removal sends no bytes, so it starts at the write.
			return event.verb === 'remove'
				? { kind: 'writing', verb: 'remove', file: event.file }
				: { kind: 'uploading', verb: event.verb, file: event.file, fraction: 0 };
		case 'progress':
			return action.kind === 'uploading' ? { ...action, fraction: event.fraction } : action;
		case 'stored':
			return action.kind === 'uploading'
				? { kind: 'writing', verb: action.verb, file: action.file }
				: action;
		case 'settled':
			return busy(action) ? IDLE : action;
		case 'failed':
			return busy(action)
				? { kind: 'refused', verb: action.verb, file: action.file, sentence: event.sentence }
				: action;
	}
}

export type Removable = { ok: true } | { ok: false; reason: string };

/**
 * Whether a row's Replace is offered.
 *
 * The thumbnail is not replaceable, and the reason is the server's rather than
 * a preference: a `cover` row is the one role the console renders as an image,
 * so a client able to point that role at a hash of its choosing could make a
 * sellable file browser-readable. The server refuses it on both the add and
 * the replace; this is the console declining to ask for something it would be
 * told no to.
 */
export function replaceable(files: readonly FileView[], id: string): Removable {
	const file = files.find((one) => one.id === id);
	if (file === undefined) {
		return { ok: false, reason: 'This file isn’t part of this resource any more.' };
	}
	return file.role === 'cover' ? { ok: false, reason: COVER_STAYS } : { ok: true };
}

export const THUMBNAIL_STAYS =
	'You can’t remove the thumbnail. It comes from your first file and updates when that file changes.';

export const LAST_PAYLOAD =
	'A listed resource needs at least one file. Replace this one, or add another file first.';

export const LAST_FILE_LOSES_THUMBNAIL =
	'This is the last file, so the thumbnail made from it goes too.';

export const COVER_STAYS =
	'Your thumbnail comes from the first file. Replace that file to change it.';

/** What removing or replacing a file does to the bytes, which is nothing.
 *
 *  Stated in one place because two places state it: the Files panel, where a
 *  stored file is removed, and the create form, where an upload is taken back
 *  before the draft is made. One fact, so one wording. */
export const STORAGE_NOT_RECLAIMED =
	'The old file stays in your storage, so this won’t free up space.';

/**
 * Whether a row's Remove is offered, and the sentence to state on the control
 * when it is not.
 *
 * The thumbnail is held here rather than by the server, which allows its
 * removal because the database does: no readiness check reads it, so a
 * resource that lost one would fail at the adapter with nothing on the page
 * having said so. Replacing it is the action that was wanted anyway.
 */
export function removable(
	files: readonly FileView[],
	id: string,
	listed: boolean
): Removable {
	const file = files.find((one) => one.id === id);
	if (file === undefined) {
		return { ok: false, reason: 'This file isn’t part of this resource any more.' };
	}
	if (file.role === 'cover') {
		return { ok: false, reason: THUMBNAIL_STAYS };
	}
	if (file.role !== 'payload') {
		return { ok: true };
	}
	const payloads = files.filter((one) => one.role === 'payload').length;
	if (payloads > 1) {
		return { ok: true };
	}
	// The requirement belongs to the marketplace listing rather than to the
	// resource: a draft kept here alone is allowed to have no file yet, which
	// is what migration 0061 moved and what the server now asks. The console
	// asking the older, stricter question would decline a removal the server
	// would allow.
	return listed ? { ok: false, reason: LAST_PAYLOAD } : { ok: true };
}

export const UNNAMED = 'unnamed file';

/**
 * The heading a file row carries: its own name, or the honest absence of one.
 *
 * Files stored before the name column existed have none and none can be
 * invented for them — the bytes were sealed under a content hash and the
 * filename was never recorded — so the row says so rather than substituting
 * the kind, because "PDF" is not a name and three of them are not three names.
 * A thumbnail has none either, being generated rather than chosen.
 */
export function fileName(file: FileView): string {
	return file.name ?? UNNAMED;
}

/**
 * What one file is called in a sentence about it.
 *
 * The name where there is one, and the role, kind and size where there is not,
 * so a confirmation reads "Remove answer-key.pdf?" for a named file and
 * "Remove the file (PDF, 2.3 MB)?" for one nobody named.
 *
 * The role word comes from [`ROLE_WORD`] rather than a conditional of its own,
 * so a dialog cannot say "the cover" in one sentence and "thumbnail" in the
 * next about the same file — which is exactly what two spellings in two places
 * produced before this read from the one map.
 */
export function fileWords(file: FileView): string {
	if (file.name !== undefined) {
		return file.name;
	}
	const role = ROLE_WORD[file.role].toLowerCase();
	return `the ${role} (${file.kind.toUpperCase()}, ${megabytes(file.byte_len)})`;
}

/**
 * Whether replacing this file also redraws the thumbnail.
 *
 * The thumbnail is generated from the first payload file's bytes, so replacing
 * that file and not the thumbnail leaves a picture of a file the resource no
 * longer holds. The server decides and reports what it did; this is the same
 * rule read forward, so the confirmation can say it before the seller commits
 * rather than the page reporting it afterwards.
 */
export function redrawsCover(files: readonly FileView[], id: string): boolean {
	const first = files.find((one) => one.role === 'payload');
	return first?.id === id && files.some((one) => one.role === 'cover');
}

/**
 * Whether removing this file leaves the thumbnail depicting a file the
 * resource no longer holds.
 *
 * The same condition as [`redrawsCover`] and a different sentence, because the
 * two verbs do different things to the thumbnail: a replacement redraws it from
 * the bytes that arrive, while a removal has no new bytes and the next payload
 * file becomes the one it should have been drawn from. Said before the seller
 * confirms rather than discovered at the next send, which is the whole of what
 * a confirmation is for.
 */
export function stalesCover(files: readonly FileView[], id: string): boolean {
	return redrawsCover(files, id);
}

/**
 * Whether removing this file takes the thumbnail with it.
 *
 * The last file of an unlisted resource: there is nothing left to draw a new
 * thumbnail from, so the old one is retired rather than redrawn, and a picture
 * of a file the resource does not hold is worse than no picture. Said before
 * the seller confirms, because it is the one removal that loses two things.
 */
export function retiresCover(files: readonly FileView[], id: string): boolean {
	const payloads = files.filter((one) => one.role === 'payload');
	return payloads.length === 1 && payloads[0]?.id === id && files.some((one) => one.role === 'cover');
}

function megabytes(bytes: number): string {
	if (bytes < 1024) {
		return `${bytes} bytes`;
	}
	const mb = bytes / (1024 * 1024);
	return mb < 1 ? `${Math.round(bytes / 1024)} KB` : `${mb.toFixed(1)} MB`;
}

/**
 * What a file change does to the listings that already exist.
 *
 * Nothing on this path contacts a marketplace, so the copy already on one
 * stands until the next send lowers a revise onto it. Said in full rather
 * than implied, because a seller who reads "removed" and does not read this
 * believes the file is gone from their storefront.
 */
export function reachSentence(reaches: readonly InventoryId[]): string {
	if (reaches.length === 0) {
		return 'This resource isn’t on any marketplace yet, so nothing else changes.';
	}
	const named = reaches.map((inventory) => platformTitle(inventory)).join(', ');
	return `The copy on ${named} stays as it is until you send again.`;
}

const OWN: Record<FileVerb, string> = {
	add: 'The file was not added.',
	replace: 'The file was not replaced.',
	remove: 'The file was not removed.'
};

/** Why a file change was refused, in the seller's words. */
export function fileRefusal(failure: unknown, verb: FileVerb): string {
	if (!(failure instanceof ApiFailure)) {
		return OWN[verb];
	}
	const said = sentenceFor(failure, OWN[verb]);
	switch (failure.code()) {
		case 'uncaptured_transition':
			return 'This resource is live on a marketplace we can’t edit yet, so you can’t change its files there.';
		case 'payload_missing':
			return LAST_PAYLOAD;
		case 'upload_rejected':
			return `${said} Choose the file again.`;
		case 'resource_missing':
			return 'That file isn’t part of this resource any more. Reload to see its current files.';
		default:
			return said;
	}
}
