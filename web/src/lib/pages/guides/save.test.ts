import { describe, expect, it } from 'vitest';
import type { GuideEdit } from './editor';
import {
	READBACK_TRIES,
	SETTLED,
	landingOf,
	previewAnswered,
	previewAsked,
	previewCurrent,
	previewFailed,
	readFailed,
	refusedIdentity,
	resolution,
	sending,
	standingOf,
	type Pipeline,
	type Preview,
	type Seen,
	type Write
} from './save';

/** The guide under test, and the one that takes its address after a delete
 *  and a recreate. Different guides, and a recreated guide's revisions start
 *  again at one, so the two can stand at the same revision. */
const MINE = '4f1d0a3e-0000-4000-8000-000000000001';
const THEIRS = '9c2b7e51-0000-4000-8000-000000000002';

const edit = (over: Partial<GuideEdit> = {}): GuideEdit => ({
	title: 'Connecting a marketplace',
	body: 'Open the marketplaces page.',
	topic_id: 'topic-1',
	tag_ids: ['tag-1'],
	...over
});

const seen = (over: Partial<Seen> = {}): Seen => ({
	id: MINE,
	revision: 8,
	edit: edit(),
	status: 'draft',
	published_from: null,
	...over
});

const save: Write = { kind: 'save', expected_id: MINE, expected_revision: 7, edit: edit() };

describe('what may be sent', () => {
	it('sends one write at a time', () => {
		const flying = sending(SETTLED, save, true);
		expect(flying).not.toBeNull();
		expect(sending(flying as Pipeline, save, true)).toBeNull();
	});

	it('sends nothing at all while a write is unresolved', () => {
		const halted: Pipeline = {
			flight: null,
			halt: { why: 'conflict', write: save, at: 9, applied: 'refused' }
		};
		expect(sending(halted, save, true)).toBeNull();
	});

	it('refuses a publication while the buffer is ahead of what was saved', () => {
		// The whole of "publish awaits the exact saved edit": a publish sent
		// with an edit outstanding would publish text nobody stored, and
		// dropping the edit to publish would lose it.
		const publish: Write = { kind: 'publish', expected_id: MINE, expected_revision: 7, edit: null };
		expect(sending(SETTLED, publish, true)).toBeNull();
		expect(sending(SETTLED, publish, false)).not.toBeNull();
	});
});

describe('what became of a write whose answer was lost', () => {
	it('reads a revision one on with the sent content as this write landing', () => {
		expect(landingOf(save, seen({ revision: 8, edit: edit() }))).toBe('landed');
	});

	it('reads an unmoved revision as a write that never arrived', () => {
		expect(landingOf(save, seen({ revision: 7, edit: edit({ body: 'older' }) }))).toBe('missed');
	});

	it('reads somebody else at this address as an overtaken write', () => {
		// One on, but not what was sent: another operator's save.
		expect(landingOf(save, seen({ revision: 8, edit: edit({ body: 'theirs' }) }))).toBe(
			'overtaken'
		);
		// This write followed by theirs, which is two revisions on. Telling the
		// two apart is not needed: either way the server holds text this editor
		// did not send.
		expect(landingOf(save, seen({ revision: 9, edit: edit() }))).toBe('overtaken');
	});

	it('reads a publication by the snapshot it left behind', () => {
		const publish: Write = { kind: 'publish', expected_id: MINE, expected_revision: 7, edit: null };
		expect(
			landingOf(publish, seen({ revision: 8, status: 'published', published_from: 7 }))
		).toBe('landed');
		// Published, but from a revision this publish did not send: somebody
		// else published a different draft.
		expect(
			landingOf(publish, seen({ revision: 8, status: 'published', published_from: 5 }))
		).toBe('overtaken');
		expect(landingOf(publish, seen({ revision: 7, status: 'draft' }))).toBe('missed');
	});

	it('reads an unpublish by the snapshot it cleared', () => {
		const unpublish: Write = {
			kind: 'unpublish',
			expected_id: MINE,
			expected_revision: 7,
			edit: null
		};
		expect(landingOf(unpublish, seen({ revision: 8, status: 'draft' }))).toBe('landed');
		expect(
			landingOf(unpublish, seen({ revision: 8, status: 'published', published_from: 8 }))
		).toBe('overtaken');
	});
});

describe('what is done about it', () => {
	it('adopts the server copy wherever nothing local is at risk', () => {
		// Including an overtaken write: another operator's text with nothing of
		// this operator's outstanding is not a conflict, and calling it one
		// would make every second editor shout about a guide they had not
		// touched.
		for (const landing of ['landed', 'missed', 'overtaken'] as const) {
			expect(resolution(landing, false)).toBe('adopt');
		}
	});

	it('keeps the buffer and carries on where the write itself is settled', () => {
		// Typed during a delayed save: the save landed, the newer text is
		// simply the next save, and adopting the answer would delete it.
		expect(resolution('landed', true)).toBe('resume');
		expect(resolution('missed', true)).toBe('resume');
	});

	it('stops on an overtaken write with unsent text', () => {
		expect(resolution('overtaken', true)).toBe('conflict');
	});
});

describe('reading the guide back', () => {
	it('gives up after the bound rather than spinning', () => {
		let pipeline: Pipeline = { flight: null, halt: { why: 'unconfirmed', write: save, reads: 0 } };
		for (let attempt = 1; attempt < READBACK_TRIES; attempt += 1) {
			pipeline = readFailed(pipeline);
			expect(pipeline.halt?.why).toBe('unconfirmed');
		}
		pipeline = readFailed(pipeline);
		expect(pipeline.halt).toEqual({ why: 'unreachable', write: save });
		// And the write is still named, because what happened to it is still
		// the question.
		expect(readFailed(pipeline).halt).toEqual({ why: 'unreachable', write: save });
	});
});

describe('where a reply stands against what is held', () => {
	it('takes this guide at or past what is acknowledged', () => {
		expect(standingOf({ id: MINE, revision: 7 }, { id: MINE, revision: 8 })).toBe('current');
		// A re-read of the same state says the same thing, and the read after
		// a lost answer is exactly that read.
		expect(standingOf({ id: MINE, revision: 7 }, { id: MINE, revision: 7 })).toBe('current');
	});

	it('drops a reply about a revision already superseded', () => {
		// Replies do not come back in the order they were sent: a slow read
		// landing after a save's answer would otherwise walk the editor back
		// onto text the server has moved past.
		expect(standingOf({ id: MINE, revision: 9 }, { id: MINE, revision: 8 })).toBe('behind');
	});

	it('reads another guide at the same revision as a replacement, not as current', () => {
		// The whole of the hole a revision alone leaves: the recreated guide
		// is at revision 1, and so was the deleted one.
		expect(standingOf({ id: MINE, revision: 1 }, { id: THEIRS, revision: 1 })).toBe('replaced');
		// And a lower revision on another guide is a replacement rather than
		// something to drop as stale.
		expect(standingOf({ id: MINE, revision: 9 }, { id: THEIRS, revision: 1 })).toBe('replaced');
	});
});

describe('a write whose guide was deleted underneath it', () => {
	it('reads another guide at the address as a replacement, whatever its revision says', () => {
		// Each of these would otherwise be read as an answer about this write:
		// the revision one on, carrying what was sent; the revision unmoved,
		// which is the "it never arrived" arm.
		expect(landingOf(save, seen({ id: THEIRS, revision: 8, edit: edit() }))).toBe('replaced');
		expect(landingOf(save, seen({ id: THEIRS, revision: 7 }))).toBe('replaced');
		const publish: Write = { kind: 'publish', expected_id: MINE, expected_revision: 7, edit: null };
		expect(
			landingOf(publish, seen({ id: THEIRS, revision: 8, status: 'published', published_from: 7 }))
		).toBe('replaced');
	});

	it('never adopts and never resumes, whatever the buffer says', () => {
		// Including where the buffer happens to read the same as the guide now
		// standing there: that is still not this operator's guide, so there is
		// nothing to reconcile with and nothing to send.
		expect(resolution('replaced', true)).toBe('replaced');
		expect(resolution('replaced', false)).toBe('replaced');
	});
});

describe('what a refusal names', () => {
	it('reads the documented shape', () => {
		expect(
			refusedIdentity({
				status: 409,
				errors: [
					{
						kind: 'validation',
						message: 'stale',
						detail: { expected_id: THEIRS, expected_revision: 12 }
					}
				]
			})
		).toEqual({ id: THEIRS, revision: 12 });
	});

	it('reads a revision with no guide on it as unknown', () => {
		// The revision alone cannot be written against: which guide it counts
		// is the question being asked, and guessing it is the overwrite this
		// whole check exists to stop. Unknown falls back to a read.
		expect(
			refusedIdentity({
				status: 409,
				errors: [{ message: 'stale', detail: { expected_revision: 12 } }]
			})
		).toBeNull();
	});

	it('reads anything else as unknown rather than as something to write against', () => {
		expect(refusedIdentity(null)).toBeNull();
		expect(refusedIdentity({ status: 409, errors: [{ message: 'stale' }] })).toBeNull();
		expect(
			refusedIdentity({ status: 409, errors: [{ message: 'stale', detail: { revision: 12 } }] })
		).toBeNull();
		expect(
			refusedIdentity({
				status: 409,
				errors: [{ message: 'stale', detail: { expected_id: THEIRS, expected_revision: '12' } }]
			})
		).toBeNull();
		expect(
			refusedIdentity({
				status: 409,
				errors: [{ message: 'stale', detail: { expected_id: '', expected_revision: 12 } }]
			})
		).toBeNull();
	});
});

describe('the live preview', () => {
	const blank: Preview = { generation: 0, html: null, from: null, state: 'ready' };

	it('draws the newest answer and drops an older one that arrives after it', () => {
		// The race this exists for: two renders out, the slow one is of the
		// older body, and it answers last.
		const first = previewAsked(blank, 'one');
		const second = previewAsked(first, 'one two');
		const drawn = previewAnswered(second, second.generation, '<p>one two</p>', 'one two');
		const late = previewAnswered(drawn, first.generation, '<p>one</p>', 'one');
		expect(late.html).toBe('<p>one two</p>');
		expect(previewCurrent(late, 'one two')).toBe(true);
	});

	it('lets an older failure neither blank nor mark the current rendering', () => {
		const first = previewAsked(blank, 'one');
		const second = previewAsked(first, 'one two');
		const drawn = previewAnswered(second, second.generation, '<p>one two</p>', 'one two');
		expect(previewFailed(drawn, first.generation)).toEqual(drawn);
		expect(previewFailed(drawn, drawn.generation).state).toBe('failed');
	});

	it('is not current while a render of the body is still out', () => {
		const asked = previewAsked(
			{ generation: 3, html: '<p>old</p>', from: 'old', state: 'ready' },
			'new'
		);
		// The old html is kept — an operator scrolling a long guide keeps their
		// place — but it is not the rendering of what is on screen.
		expect(asked.html).toBe('<p>old</p>');
		expect(previewCurrent(asked, 'new')).toBe(false);
	});

	it('answers an emptied body itself rather than asking', () => {
		const emptied = previewAsked({ generation: 1, html: '<p>x</p>', from: 'x', state: 'ready' }, '');
		expect(emptied).toEqual({ generation: 2, html: '', from: '', state: 'ready' });
		expect(previewCurrent(emptied, '')).toBe(true);
	});
});
