// The profile picture's pure half: the check on a chosen file before a byte
// is sent, and the sentence a refused save reads as. Pure so it tests without
// a component; the panel itself is read by `avatar-panel.test.ts`.

import { ApiFailure } from '$lib/api';
import { quotaSentence } from '$lib/authoring';

export const PICTURE_ONLY = 'Choose a JPEG, PNG or GIF picture.';
export const NOT_SAVED = 'The picture was not saved.';
export const NOT_REMOVED = 'The picture was not removed.';

/**
 * Refused before a byte is sent where the browser can already tell.
 *
 * `File.type` is derived from the extension, so this is a hint rather than
 * the gate: the server reads the bytes, at the upload and again before the
 * profile write, and its sentence is the one shown when the two disagree.
 */
export function pictureRefusal(kind: string): string | null {
	return kind.startsWith('image/') ? null : PICTURE_ONLY;
}

/**
 * Why a picture was not saved, in the seller's words.
 *
 * The upload and the profile write share one sentence per code because a
 * seller who meets the refusal twice about one file should not read two
 * answers. A body that did not parse falls back to the page's own sentence:
 * `ApiFailure.message` is then a status line, which is not something a
 * teacher can act on.
 */
export function avatarRefusal(failure: unknown): string {
	if (!(failure instanceof ApiFailure)) {
		return NOT_SAVED;
	}
	if (failure.status === 0) {
		return 'The upload did not reach us, so nothing was saved. Try again.';
	}
	const entry = failure.body?.errors?.[0];
	switch (failure.code()) {
		case 'quota_exceeded':
			return quotaSentence(entry?.detail) ?? entry?.message ?? NOT_SAVED;
		case 'blob_store_unavailable':
			return 'Picture uploads are not available yet, so nothing was saved.';
		case 'upload_rejected':
			return entry?.message ?? PICTURE_ONLY;
		default:
			return failure.body === null ? NOT_SAVED : failure.message;
	}
}
