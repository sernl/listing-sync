import { describe, expect, it } from 'vitest';
import { AXES, AXIS_LABEL, OVERRIDABLE, draftOf, isComplete } from './templates';
import type { OverrideView } from './api';

function row(patch: Partial<OverrideView> = {}): OverrideView {
	return {
		inventory: 'Tpt',
		axis: 'subject',
		from_term: 't-1',
		segments: ['Maths'],
		native_id: 'math-elementary',
		kind: 'exact',
		decided_at: 5,
		...patch
	};
}

describe('the overridable axes', () => {
	it('offers every axis but licence, which both the domain and the database refuse', () => {
		expect(AXES).toEqual(['subject', 'topic', 'resource_type', 'phase']);
		expect(OVERRIDABLE.licence).toBe(false);
	});

	it('names every axis, including the one it does not offer', () => {
		for (const axis of Object.keys(OVERRIDABLE)) {
			expect(AXIS_LABEL[axis as keyof typeof AXIS_LABEL]).toBeTruthy();
		}
	});
});

describe('loading a saved override back into the form', () => {
	it('carries every field the row holds, so a change starts from what is stored', () => {
		expect(draftOf(row())).toEqual({
			inventory: 'Tpt',
			axis: 'subject',
			from: 't-1',
			toNative: 'math-elementary',
			kind: 'exact'
		});
	});

	it('round-trips a broader override on another marketplace and axis', () => {
		const loaded = draftOf(
			row({ inventory: 'TesGb', axis: 'phase', kind: 'broader', from_term: 't-9' })
		);
		expect(loaded.inventory).toBe('TesGb');
		expect(loaded.axis).toBe('phase');
		expect(loaded.kind).toBe('broader');
		expect(loaded.from).toBe('t-9');
	});

	it('reads a row with no identifier as an empty destination rather than undefined', () => {
		const loaded = draftOf(row({ native_id: undefined }));
		expect(loaded.toNative).toBe('');
		expect(isComplete(loaded)).toBe(false);
	});

	it('a loaded row with an identifier is ready to save again', () => {
		expect(isComplete(draftOf(row()))).toBe(true);
	});
});
