import { describe, expect, it } from 'vitest';
import { chosenOf, destinationsOf } from './destinations';
import type { VocabularyView } from '$lib/api';

function vocabulary(patch: Partial<VocabularyView> = {}): VocabularyView {
	return {
		inventory: 'Tpt',
		marketplace: 'Tpt',
		canonical: [],
		natives: [
			{
				name: 'subject_area',
				direction: 'written',
				required: true,
				vocabulary: 'closed',
				values: [{ id: 'math-elementary', label: 'Math > Basic Operations' }],
				delegation: { kind: 'by_opt_in' }
			}
		],
		axes: [
			{
				axis: 'subject',
				native: 'subject_area',
				cardinality: 'many',
				delegation: { kind: 'by_opt_in' },
				required: true
			}
		],
		absent_axes: [],
		authoring: {
			payload_files: 'exactly_one',
			body_wire: 'renders_to_html',
			body_formats: []
		},
		...patch
	} as VocabularyView;
}

describe('what a marketplace offers on an axis', () => {
	it('says unread while the vocabulary has not arrived, which is every page load', () => {
		expect(destinationsOf(undefined, 'subject')).toEqual({ kind: 'unread' });
	});

	it('says unbound when the marketplace lands this axis in no field of its own', () => {
		expect(destinationsOf(vocabulary(), 'phase')).toEqual({ kind: 'unbound' });
	});

	it('says open when the field is bound but publishes no list to choose from', () => {
		const open = vocabulary({
			natives: [
				{
					name: 'subject_area',
					direction: 'written',
					required: true,
					vocabulary: 'closed_uncaptured',
					delegation: { kind: 'by_opt_in' }
				}
			]
		});
		expect(destinationsOf(open, 'subject')).toEqual({ kind: 'open' });
	});

	it('carries the values when there are values', () => {
		const answer = destinationsOf(vocabulary(), 'subject');
		expect(answer.kind).toBe('values');
		expect(answer.kind === 'values' && answer.values[0].id).toBe('math-elementary');
	});

	it('never reports an unread vocabulary as an empty one', () => {
		// The whole point of the union: both used to be an empty array, and the
		// screen told the seller the marketplace publishes nothing.
		expect(destinationsOf(undefined, 'subject').kind).not.toBe(destinationsOf(vocabulary(), 'phase').kind);
	});
});

describe('the value a save may actually name', () => {
	const offered = destinationsOf(vocabulary(), 'subject');

	it('keeps a value that is on offer', () => {
		expect(chosenOf(offered, 'math-elementary')).toBe('math-elementary');
	});

	it('drops an identifier belonging to a field that is no longer selected', () => {
		// The S8 write: pick a subject destination, switch the axis to one that
		// lands in another field, and the old identifier is still a non-empty
		// string. Without this it satisfies completeness and is written as the
		// destination of a field it does not belong to.
		expect(chosenOf(offered, 'english-reading')).toBe('');
	});

	it('names nothing while the vocabulary is unread, so a save cannot run early', () => {
		expect(chosenOf({ kind: 'unread' }, 'math-elementary')).toBe('');
	});

	it('names nothing when the field offers no list at all', () => {
		expect(chosenOf({ kind: 'open' }, 'math-elementary')).toBe('');
		expect(chosenOf({ kind: 'unbound' }, 'math-elementary')).toBe('');
	});
});
