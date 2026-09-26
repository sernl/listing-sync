import { describe, expect, it } from 'vitest';
import { ApiFailure, type FileView } from '$lib/api';
import {
	COVER_STAYS,
	coverUrlOf,
	IDLE,
	LAST_PAYLOAD,
	advance,
	busy,
	ROLE_WORD,
	UNNAMED,
	fileName,
	fileRefusal,
	fileWords,
	reachSentence,
	THUMBNAIL_STAYS,
	redrawsCover,
	removable,
	replaceable,
	retiresCover,
	stalesCover,
	type FileAction
} from './files';

function file(
	id: string,
	role: FileView['role'],
	bytes = 2_400_000,
	name?: string
): FileView {
	return {
		id,
		role,
		kind: role === 'cover' ? 'image' : 'pdf',
		byte_len: bytes,
		// Distinct per row, as a digest is: nothing here reads it, and one
		// shared literal would make two files one file to anything that did.
		hash: id.repeat(64).slice(0, 64),
		scan: 'clean',
		name
	};
}

const TWO_PAYLOADS = [file('a', 'payload'), file('b', 'payload'), file('c', 'cover')];
const ONE_PAYLOAD = [file('a', 'payload'), file('c', 'cover')];

describe('the thumbnail’s URL', () => {
	// The founder's stale thumbnail: the cover route answered with a cache
	// lifetime, and the form asked for it under one constant URL, so a
	// replaced first file kept drawing the old picture.
	it('changes when the server redraws the cover after a replace', () => {
		const before = coverUrlOf('p', ONE_PAYLOAD);
		const redrawn = [file('d', 'payload'), file('e', 'cover')];
		const after = coverUrlOf('p', redrawn);
		expect(before).toBe(`/v1/products/p/cover?v=${'c'.repeat(64)}`);
		expect(after).toBe(`/v1/products/p/cover?v=${'e'.repeat(64)}`);
		expect(after).not.toBe(before);
	});

	it('is the same URL while the cover is the same bytes, so the browser keeps it', () => {
		expect(coverUrlOf('p', ONE_PAYLOAD)).toBe(coverUrlOf('p', [...TWO_PAYLOADS]));
	});

	it('is absent where the resource has no cover', () => {
		expect(coverUrlOf('p', [file('a', 'payload')])).toBeNull();
	});
});

describe('the panel’s state machine', () => {
	it('asks before it destroys, and the question names the row', () => {
		const asked = advance(IDLE, { kind: 'ask', verb: 'remove', file: 'a', chosen: null });
		expect(asked).toEqual({ kind: 'confirming', verb: 'remove', file: 'a', chosen: null });
	});

	it('starts a removal at the write, because a removal sends no bytes', () => {
		const writing = advance(IDLE, { kind: 'send', verb: 'remove', file: 'a' });
		expect(writing).toEqual({ kind: 'writing', verb: 'remove', file: 'a' });
	});

	it('starts a rename at the write, because a rename sends no bytes either', () => {
		const writing = advance(IDLE, { kind: 'send', verb: 'rename', file: 'a' });
		expect(writing).toEqual({ kind: 'writing', verb: 'rename', file: 'a' });
	});

	it('carries a replacement through upload, progress and write', () => {
		let state: FileAction = advance(IDLE, { kind: 'send', verb: 'replace', file: 'a' });
		expect(state).toEqual({ kind: 'uploading', verb: 'replace', file: 'a', fraction: 0 });
		state = advance(state, { kind: 'progress', fraction: 0.42 });
		expect(state).toEqual({ kind: 'uploading', verb: 'replace', file: 'a', fraction: 0.42 });
		state = advance(state, { kind: 'stored' });
		expect(state).toEqual({ kind: 'writing', verb: 'replace', file: 'a' });
		expect(advance(state, { kind: 'settled' })).toEqual(IDLE);
	});

	it('runs one action at a time', () => {
		// The fault: a second control starting while the first is in flight
		// removes the row the upload is about to write to.
		const uploading = advance(IDLE, { kind: 'send', verb: 'replace', file: 'a' });
		expect(advance(uploading, { kind: 'send', verb: 'remove', file: 'b' })).toBe(uploading);
		expect(advance(uploading, { kind: 'ask', verb: 'remove', file: 'b', chosen: null })).toBe(
			uploading
		);
		expect(advance(uploading, { kind: 'cancel' })).toBe(uploading);
	});

	it('keeps a refusal on the row that earned it', () => {
		const writing = advance(IDLE, { kind: 'send', verb: 'remove', file: 'b' });
		const refused = advance(writing, { kind: 'failed', sentence: 'nope' });
		expect(refused).toEqual({ kind: 'refused', verb: 'remove', file: 'b', sentence: 'nope' });
	});

	it('lets the seller try again from a refusal without dismissing it first', () => {
		const refused: FileAction = { kind: 'refused', verb: 'remove', file: 'b', sentence: 'nope' };
		expect(busy(refused)).toBe(false);
		expect(advance(refused, { kind: 'ask', verb: 'remove', file: 'b', chosen: null })).toEqual({
			kind: 'confirming',
			verb: 'remove',
			file: 'b',
			chosen: null
		});
		expect(advance(refused, { kind: 'cancel' })).toEqual(IDLE);
	});

	it('ignores progress and settlement that belong to no running action', () => {
		expect(advance(IDLE, { kind: 'progress', fraction: 0.5 })).toBe(IDLE);
		expect(advance(IDLE, { kind: 'stored' })).toBe(IDLE);
		expect(advance(IDLE, { kind: 'settled' })).toBe(IDLE);
		expect(advance(IDLE, { kind: 'failed', sentence: 'nope' })).toBe(IDLE);
	});
});

describe('which files a seller may remove', () => {
	it('offers a removal while another payload file remains', () => {
		expect(removable(TWO_PAYLOADS, 'a', true)).toEqual({ ok: true });
	});

	it('refuses the last file of a resource a marketplace carries', () => {
		expect(removable(ONE_PAYLOAD, 'a', true)).toEqual({ ok: false, reason: LAST_PAYLOAD });
	});

	it('allows the last file of a resource no marketplace carries', () => {
		// The requirement belongs to the listing rather than the resource, which
		// is what migration 0061 moved; asking the older question here would
		// decline a removal the server allows.
		expect(removable(ONE_PAYLOAD, 'a', false)).toEqual({ ok: true });
	});

	it('holds the thumbnail, which no readiness check would notice the loss of', () => {
		expect(removable(TWO_PAYLOADS, 'c', true)).toEqual({ ok: false, reason: THUMBNAIL_STAYS });
		expect(removable(TWO_PAYLOADS, 'c', false)).toEqual({ ok: false, reason: THUMBNAIL_STAYS });
	});

	it('refuses a row the resource no longer carries', () => {
		expect(removable(TWO_PAYLOADS, 'gone', true).ok).toBe(false);
	});
});

describe('which files a seller may replace', () => {
	it('offers a replacement on any file the seller chose', () => {
		const withPreview = [...TWO_PAYLOADS, file('d', 'preview')];
		expect(replaceable(withPreview, 'a')).toEqual({ ok: true });
		expect(replaceable(withPreview, 'b')).toEqual({ ok: true });
		expect(replaceable(withPreview, 'd')).toEqual({ ok: true });
	});

	it('declines the thumbnail, because the server refuses that write', () => {
		// A `cover` row is the one role the console renders as an image, so a
		// client able to point it at a hash of its choosing could make a
		// sellable file browser-readable. The server refuses it; the console
		// does not ask.
		expect(replaceable(TWO_PAYLOADS, 'c')).toEqual({ ok: false, reason: COVER_STAYS });
	});

	it('declines a row the resource no longer carries', () => {
		expect(replaceable(TWO_PAYLOADS, 'gone').ok).toBe(false);
	});

	it('never sends a seller to a control that would be refused', () => {
		// The two rules together: nothing the console offers on a thumbnail row
		// reaches a write the server declines.
		expect(replaceable(TWO_PAYLOADS, 'c').ok).toBe(false);
		expect(removable(TWO_PAYLOADS, 'c', true).ok).toBe(false);
	});
});

describe('what a file change says it does', () => {
	it('states that a listed marketplace keeps its copy until the next send', () => {
		expect(reachSentence(['Tpt', 'Tes'])).toBe(
			'The copy on TPT (Teachers Pay Teachers), TES (Tes.com) stays as it is until you send again.'
		);
	});

	it('says so plainly when the resource is on no marketplace', () => {
		expect(reachSentence([])).toBe(
			'This resource isn’t on any marketplace yet, so nothing else changes.'
		);
	});

	it('names a file by what the page actually knows about it', () => {
		expect(fileWords(file('a', 'payload'))).toBe('the file (PDF, 2.3 MB)');
		expect(fileWords(file('c', 'cover', 184_220))).toBe('the thumbnail (IMAGE, 180 KB)');
		expect(fileWords(file('d', 'preview', 900))).toBe('the preview (PDF, 900 bytes)');
	});

	it('prefers the seller’s own filename wherever one was recorded', () => {
		expect(fileWords(file('a', 'payload', 2_400_000, 'answer-key.pdf'))).toBe('answer-key.pdf');
		expect(fileName(file('a', 'payload', 2_400_000, 'answer-key.pdf'))).toBe('answer-key.pdf');
	});

	it('says a file is unnamed rather than showing its kind as a name', () => {
		// The fault this guards: three PDFs stored before names existed render
		// as three identical rows, which is the defect the name column closes.
		expect(fileName(file('a', 'payload'))).toBe(UNNAMED);
		expect(fileName(file('b', 'payload', 618_004))).toBe(UNNAMED);
	});
});

describe('the words a seller reads for a file role', () => {
	it('never puts a wire identifier on the screen', () => {
		// The fault this guards: the pill uppercases what it is handed, so
		// `role` reaching it unmapped rendered PAYLOAD — an identifier, and a
		// banned word — on every buyer-download row.
		for (const word of Object.values(ROLE_WORD)) {
			expect(word).not.toMatch(/^(payload|preview|cover)$/);
		}
		expect(ROLE_WORD.payload).toBe('File');
		expect(ROLE_WORD.cover).toBe('Thumbnail');
		expect(ROLE_WORD.preview).toBe('Preview');
	});

	it('says thumbnail in a sentence and in a pill, never cover in either', () => {
		// Two spellings for one thing landed in adjacent sentences of one
		// dialog before `fileWords` read from the same map the pill does.
		const words = fileWords(file('c', 'cover', 184_220));
		expect(words).toContain('thumbnail');
		expect(words).not.toContain('cover');
		expect(COVER_STAYS).toContain('thumbnail');
		expect(COVER_STAYS).not.toContain('cover');
		expect(THUMBNAIL_STAYS).toContain('thumbnail');
		expect(THUMBNAIL_STAYS).not.toContain('cover');
	});
});

describe('which replacement redraws the thumbnail', () => {
	it('redraws when the first payload file is the one being replaced', () => {
		expect(redrawsCover(TWO_PAYLOADS, 'a')).toBe(true);
	});

	it('leaves the thumbnail alone for any later payload file', () => {
		// The cover was drawn from the first file's bytes, so redrawing it from
		// the second would put the answer key on the storefront.
		expect(redrawsCover(TWO_PAYLOADS, 'b')).toBe(false);
	});

	it('redraws nothing where the resource has no cover to redraw', () => {
		expect(redrawsCover([file('a', 'payload')], 'a')).toBe(false);
	});

	it('says nothing is redrawn when the cover itself is replaced', () => {
		expect(redrawsCover(TWO_PAYLOADS, 'c')).toBe(false);
	});

	it('warns when the last file takes the thumbnail with it', () => {
		// The one removal that loses two things: nothing is left to draw a new
		// thumbnail from, so the old one is retired rather than redrawn.
		expect(retiresCover(ONE_PAYLOAD, 'a')).toBe(true);
		expect(retiresCover(TWO_PAYLOADS, 'a')).toBe(false);
		expect(retiresCover([file('a', 'payload')], 'a')).toBe(false);
	});

	it('warns on a removal that would leave the thumbnail behind', () => {
		// The finding this closes: removing the first of two payload files was
		// permitted and silent, and left the thumbnail holding the retired
		// file's bytes with nothing saying so.
		expect(stalesCover(TWO_PAYLOADS, 'a')).toBe(true);
		expect(stalesCover(TWO_PAYLOADS, 'b')).toBe(false);
		expect(stalesCover(TWO_PAYLOADS, 'c')).toBe(false);
		expect(stalesCover([file('a', 'payload')], 'a')).toBe(false);
	});
});

describe('a refused file change', () => {
	it('says a live listing’s files cannot be changed, rather than showing a code', () => {
		const refused = new ApiFailure(422, {
			status: 422,
			errors: [{ code: 'uncaptured_transition', message: 'uncaptured', detail: undefined }]
		} as never);
		expect(fileRefusal(refused, 'replace')).toBe(
			'This resource is live on a marketplace we can’t edit yet, so you can’t change its files there.'
		);
	});

	it('turns the server’s last-file refusal into the same sentence the control carries', () => {
		const refused = new ApiFailure(422, {
			status: 422,
			errors: [{ code: 'payload_missing', message: 'no payload', detail: undefined }]
		} as never);
		expect(fileRefusal(refused, 'remove')).toBe(LAST_PAYLOAD);
	});

	it('never hands the seller a status line', () => {
		expect(fileRefusal(new ApiFailure(502, null), 'add')).toBe('The file was not added.');
		expect(fileRefusal(new Error('offline'), 'remove')).toBe('The file was not removed.');
		expect(fileRefusal(undefined, 'replace')).toBe('The file was not replaced.');
	});
});
