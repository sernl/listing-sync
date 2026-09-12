// What the server's answers mean while the operator keeps typing.
//
// Three separate problems live here, and all three are about time rather than
// about Markdown:
//
//  - A save is in flight and the operator types. The answer that comes back is
//    the server's copy of an older edit, so adopting its text wholesale would
//    delete what was typed in between.
//  - A save's answer never arrives. Whether the server stored it is unknown,
//    and both guesses are wrong: assuming it landed loses the edit, assuming it
//    did not can overwrite a second operator. The only honest move is to read
//    the guide back and compare, and to write nothing until that read says
//    what happened.
//  - The guide at this address is deleted and another is created in its
//    place. Its revisions start again at one, so the revision an outstanding
//    write was sent against matches a guide that write never read. Nothing
//    further may be sent, and what became of the write is not knowable from
//    the guide now standing there.
//  - A preview answers after the body has moved on. Drawing it would present
//    stale HTML as the current rendering.
//
// The decisions are here, as functions over plain values, because each one is a
// race the component cannot demonstrate by hand.

import type { APIErrorBody, GuideStatus } from '$lib/api';
import { sameEdit, type GuideEdit } from './editor';

/** How many times an authoritative read is attempted after a lost
 *  acknowledgement before the editor says it cannot reach the server.
 *
 * Bounded rather than persistent: the point of the read is to tell the
 * operator what happened to their edit, and a read that keeps retrying behind
 * a spinner tells them nothing while their text sits unsaved. */
export const READBACK_TRIES = 3;

/** Which guide, and which revision of it, something is about.
 *
 * A revision on its own is not an address into the world: it counts writes
 * within one guide, and two guides at the same slug can both be at revision
 * one. The pair is what identifies a stored state. */
export interface Identity {
	id: string;
	revision: number;
}

/** What one write asked the server to do. */
export type WriteKind = 'save' | 'publish' | 'unpublish';

/** A write that has been sent, with everything needed to work out afterwards
 *  whether it landed. */
export interface Write {
	kind: WriteKind;
	/** The guide the write named, which the server checks alongside the
	 *  revision. An answer or a read-back naming another id is another guide,
	 *  whatever its revision says. */
	expected_id: string;
	/** The revision the write was sent against, which is the revision the
	 *  server checks and the one it moves off. */
	expected_revision: number;
	/** The edit a save carried. Null on a publication: it writes visibility
	 *  and no text. */
	edit: GuideEdit | null;
}

/** Where a reply stands against the guide the editor has acknowledged.
 *
 * Replies do not come back in the order they were sent, and an address can
 * change hands underneath them: arriving last is not the same as being newest,
 * and naming this slug is not the same as being this guide. */
export type Standing =
	/** This guide, no older than what is already acknowledged. */
	| 'current'
	/** This guide, at a revision already superseded. Taking it would walk the
	 *  editor and the cache backwards onto text the server has moved past. */
	| 'behind'
	/** A different guide at this address. */
	| 'replaced';

/** Whether a reply may be taken as what this editor holds.
 *
 * Equal revisions are `current` rather than `behind`: a re-read of the same
 * state says the same thing, and refusing it would drop the authoritative
 * read a lost answer is settled by. */
export function standingOf(held: Identity, answered: Identity): Standing {
	if (answered.id !== held.id) {
		return 'replaced';
	}
	return answered.revision < held.revision ? 'behind' : 'current';
}

/** Why writes are paused. While one of these stands, nothing is sent: an
 *  autosave on top of an unresolved write is how an edit gets lost. */
export type Halt =
	/** The answer was lost. The write may or may not have landed, and the
	 *  editor is reading the guide back to find out. */
	| { why: 'unconfirmed'; write: Write; reads: number }
	/** The server holds a different revision. The buffer stays; the operator
	 *  decides what happens to it.
	 *
	 *  `applied` is what is actually known about the write itself, and the two
	 *  cases are not the same thing to say out loud. A 409 is the server
	 *  refusing before it wrote, so nothing happened. A conflict worked out
	 *  from a read-back after a lost answer is a guide that has moved on in
	 *  some way this editor cannot attribute — the write may well have landed
	 *  and been written over — so telling the operator it did not happen would
	 *  be a claim nobody can make. */
	| { why: 'conflict'; write: Write; at: number | null; applied: 'refused' | 'unknown' }
	/** The server refused the write on its merits. Retrying it unchanged would
	 *  be refused again. */
	| { why: 'refused'; write: Write; message: string }
	/** The authoritative read could not be made either, so what happened to
	 *  the write is still unknown. */
	| { why: 'unreachable'; write: Write }
	/** The guide this editor was writing is gone, and another guide answers
	 *  to its address.
	 *
	 *  Nothing more is sent: every write this editor could make names a guide
	 *  the server no longer holds, and re-aiming one at the guide now standing
	 *  there would overwrite a guide nobody here has read. `write` is whatever
	 *  was outstanding when this came to light, where there was one — its fate
	 *  is not knowable from here and is not guessed at — and `now` is the
	 *  guide at the address, where the server named it. */
	| { why: 'replaced'; write: Write | null; now: Identity | null };

/** The one write in flight and the reason writes are paused.
 *
 * There is no queue of edits, deliberately. The buffer is the queue: the
 * latest text is whatever the operator has typed, and the next write sends
 * that. A stored queue would be a second copy of the draft, and the only thing
 * it could ever hold is an edit already superseded by the one on screen. */
export interface Pipeline {
	flight: Write | null;
	halt: Halt | null;
}

export const SETTLED: Pipeline = { flight: null, halt: null };

/** Whether a write may leave now. One at a time, and none at all while a halt
 *  stands. */
export function open(pipeline: Pipeline): boolean {
	return pipeline.flight === null && pipeline.halt === null;
}

/** The pipeline with a write in flight, or null where this one may not leave.
 *
 * A publication is refused while the buffer differs from what was saved:
 * publishing has to mean "publish exactly what I saved", so an unsaved edit is
 * saved first rather than published silently or dropped. */
export function sending(pipeline: Pipeline, write: Write, unsaved: boolean): Pipeline | null {
	if (!open(pipeline)) {
		return null;
	}
	if (write.kind !== 'save' && unsaved) {
		return null;
	}
	return { flight: write, halt: null };
}

/** One failed authoritative read. Counted, and at the bound the editor stops
 *  reading and says so rather than spinning. */
export function readFailed(pipeline: Pipeline): Pipeline {
	const halt = pipeline.halt;
	if (halt === null || halt.why !== 'unconfirmed') {
		return pipeline;
	}
	const reads = halt.reads + 1;
	return reads >= READBACK_TRIES
		? { flight: null, halt: { why: 'unreachable', write: halt.write } }
		: { flight: null, halt: { why: 'unconfirmed', write: halt.write, reads } };
}

/** What the guide looked like when it was read back. */
export interface Seen {
	id: string;
	revision: number;
	edit: GuideEdit;
	status: GuideStatus;
	/** The published snapshot's source revision, where one is published. */
	published_from: number | null;
}

/** What became of a write whose answer was lost. */
export type Landing =
	/** The server stored it. */
	| 'landed'
	/** The server never stored it: the revision has not moved. */
	| 'missed'
	/** Something else happened at this address — another operator's write, or
	 *  this write followed by theirs. Which of the two cannot be told apart
	 *  from a revision, and does not need to be: either way the server holds
	 *  text this editor did not send. */
	| 'overtaken'
	/** The guide the write named is not the guide at this address any more.
	 *  What became of the write cannot be read off a guide it was never
	 *  applied to: the revisions standing there count somebody else's writes. */
	| 'replaced';

/** Whether the write landed, read off the guide as it now stands.
 *
 * A revision rather than a timestamp, because a revision moves exactly once
 * per write: one step on from what was sent against, carrying what was sent,
 * is this write and nothing else. */
export function landingOf(write: Write, seen: Seen): Landing {
	if (seen.id !== write.expected_id) {
		return 'replaced';
	}
	if (seen.revision === write.expected_revision) {
		return 'missed';
	}
	const moved = seen.revision === write.expected_revision + 1;
	if (write.kind === 'save') {
		const sent = write.edit;
		return moved && sent !== null && sameEdit(seen.edit, sent) ? 'landed' : 'overtaken';
	}
	if (write.kind === 'publish') {
		// The snapshot names the draft revision it was taken from, which is
		// exactly the revision this publish was sent against. A publication
		// that named another revision is somebody else's.
		return moved && seen.status === 'published' && seen.published_from === write.expected_revision
			? 'landed'
			: 'overtaken';
	}
	return moved && seen.status === 'draft' && seen.published_from === null ? 'landed' : 'overtaken';
}

/** What the editor does with the guide it read back. */
export type Resolution =
	/** Take the server's copy: text, revision and visibility. Nothing local is
	 *  at risk, so the two are simply reconciled. */
	| 'adopt'
	/** Keep the local text and let writes flow again. The server's revision is
	 *  adopted so the next save is sent against the right one, and the buffer
	 *  goes out as that next write. */
	| 'resume'
	/** Stop. The server holds text this editor did not send and the operator
	 *  holds text the server does not have, so which survives is their call. */
	| 'conflict'
	/** Stop, and differently. There is no stored copy of this guide to
	 *  reconcile with any more, so neither taking the server's text nor
	 *  sending the buffer over it is a move this editor may make by itself. */
	| 'replaced';

/** What to do about a landing, given whether the buffer still differs from
 *  what the server now holds.
 *
 *  `atRisk` is the whole of the question: adopting the server's copy overwrites
 *  the buffer, so it is only ever safe where the buffer says the same thing.
 *  A save that landed while the operator carried on typing is not a conflict —
 *  their newer text is simply the next save. */
export function resolution(landing: Landing, atRisk: boolean): Resolution {
	if (landing === 'replaced') {
		// Not a function of what is at risk. A buffer that happens to read the
		// same as the guide now at this address is still not that guide's
		// text, so there is no case where the two are simply reconciled.
		return 'replaced';
	}
	if (!atRisk) {
		return 'adopt';
	}
	return landing === 'overtaken' ? 'conflict' : 'resume';
}

/** The guide and revision a refusal named, or null where it named neither.
 *
 * The 409 carries what the server holds now — `errors[].detail`'s
 * `expected_id` and `expected_revision` — and it carries the same shape
 * whether the revision was stale or the guide itself has been replaced.
 * Comparing the id it names against the id that was sent is what tells those
 * two apart, so both halves are required: a detail with only a revision on it
 * cannot be written against, because which guide that revision counts is
 * exactly the question. Anything else reads as unknown and falls back to an
 * authoritative read. */
export function refusedIdentity(body: APIErrorBody | null): Identity | null {
	for (const entry of body?.errors ?? []) {
		const detail = entry.detail;
		if (typeof detail !== 'object' || detail === null) {
			continue;
		}
		const named = detail as Record<string, unknown>;
		const id = named.expected_id;
		const revision = named.expected_revision;
		if (
			typeof id === 'string' &&
			id.length > 0 &&
			typeof revision === 'number' &&
			Number.isInteger(revision)
		) {
			return { id, revision };
		}
	}
	return null;
}

// ----------------------------------------------------------------- preview

/** The live preview, and which body it is the rendering of.
 *
 * The generation is the fence. Every request takes the next one, and an answer
 * is drawn only where it carries the newest: a slow render of an older body
 * would otherwise land after a fast render of the current one and replace it
 * with the past. */
export interface Preview {
	generation: number;
	html: string | null;
	/** The body the held html was rendered from, so "is this current" is
	 *  answered by comparison rather than by a flag that can be wrong. */
	from: string | null;
	state: 'reading' | 'ready' | 'failed';
}

/** The next request's generation and the state to draw while it is out.
 *
 * An empty body is answered here rather than asked about: the rendering of
 * nothing is nothing, and a round trip to learn that is a round trip that can
 * fail. */
export function previewAsked(preview: Preview, body: string): Preview {
	const generation = preview.generation + 1;
	if (body.length === 0) {
		return { generation, html: '', from: '', state: 'ready' };
	}
	return { generation, html: preview.html, from: preview.from, state: 'reading' };
}

export function previewAnswered(
	preview: Preview,
	generation: number,
	html: string,
	body: string
): Preview {
	if (generation !== preview.generation) {
		return preview;
	}
	return { generation, html, from: body, state: 'ready' };
}

export function previewFailed(preview: Preview, generation: number): Preview {
	if (generation !== preview.generation) {
		return preview;
	}
	return { ...preview, state: 'failed' };
}

/** Whether what is held is the rendering of this body. A false here is what
 *  makes the panel say so rather than pass an older rendering off as the
 *  current one. */
export function previewCurrent(preview: Preview, body: string): boolean {
	return preview.state === 'ready' && preview.from === body;
}
