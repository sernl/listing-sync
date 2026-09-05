import { describe, expect, it } from 'vitest';
import {
	NAME_MAX,
	consumeBlankRequest,
	emptyForm,
	formOf,
	isComplete,
	refusalOf,
	savedLine,
	toInput,
	type TemplateForm
} from './resource-template';
import type { TemplateHead, TemplateView } from './api';

function form(patch: Partial<TemplateForm> = {}): TemplateForm {
	return { ...emptyForm(), name: 'Year 4 maths starter', ...patch };
}

function saved(draft: TemplateView['draft'] = {}): TemplateView {
	return { id: 'tpl-1', name: 'Year 4 maths starter', created_at: 5, updated_at: 7, draft };
}

function head(patch: Partial<TemplateHead> = {}): TemplateHead {
	return { id: 'tpl-1', name: 'Year 4 maths starter', created_at: 1000, updated_at: 1000, ...patch };
}

describe('what a template may be saved with', () => {
	it('refuses a template with no name, because a list of them needs telling apart', () => {
		expect(refusalOf(form({ name: '   ', subjects: ['maths'] }))).toMatch(/name/);
	});

	it('refuses a name past the length the server stores, before the round trip', () => {
		const long = 'a'.repeat(NAME_MAX + 1);
		expect(refusalOf(form({ name: long, subjects: ['maths'] }))).toMatch(/80/);
		expect(isComplete(form({ name: 'a'.repeat(NAME_MAX), subjects: ['maths'] }))).toBe(true);
	});

	it('counts a name in characters rather than code units, as the server does', () => {
		// Eighty astral characters are eighty characters and 160 code units;
		// `.length` would refuse a name the server stores.
		expect(isComplete(form({ name: '\u{1D400}'.repeat(NAME_MAX), subjects: ['maths'] }))).toBe(
			true
		);
	});

	it('refuses a control character in the name, which the column cannot hold', () => {
		expect(refusalOf(form({ name: 'Year 4\u0000maths', subjects: ['maths'] }))).toMatch(
			/hidden character/
		);
	});

	it('saves a name and nothing else, which the server calls the blank template', () => {
		// `validated_draft(&json!({})).is_ok()` in resource_templates.rs, under
		// the sentence naming it the blank template a seller starts from. A
		// stricter client rule would be a second rule about one fact.
		expect(refusalOf(form())).toBeNull();
		expect(isComplete(form())).toBe(true);
		expect(toInput(form())).toEqual({ name: 'Year 4 maths starter', draft: {} });
	});

	it('accepts any one of the four starting points', () => {
		expect(isComplete(form({ subjects: ['maths'] }))).toBe(true);
		expect(isComplete(form({ grades: ['year-4'] }))).toBe(true);
		expect(isComplete(form({ licence: '3' }))).toBe(true);
		expect(isComplete(form({ price: '4.50' }))).toBe(true);
		expect(isComplete(form({ free: true }))).toBe(true);
	});

	it('refuses a price that is not a number, rather than dropping what was typed', () => {
		expect(refusalOf(form({ subjects: ['maths'], price: 'four fifty' }))).toMatch(/4\.50/);
	});

	it('ignores an unparseable price once the resource is free, which answers the price', () => {
		expect(isComplete(form({ free: true, price: 'four fifty' }))).toBe(true);
	});

	it('still refuses a form with no name at all, which is the one bound left', () => {
		expect(isComplete(emptyForm())).toBe(false);
	});
});

describe('the write a form names', () => {
	it('leaves every unanswered field out, so a template stays a partial draft', () => {
		expect(toInput(form({ subjects: ['maths'] }))).toEqual({
			name: 'Year 4 maths starter',
			draft: { subject_areas: ['maths'] }
		});
	});

	it('trims the name and carries every answered field', () => {
		expect(
			toInput(
				form({
					name: '  Year 4 maths starter  ',
					subjects: ['maths'],
					grades: ['year-4', 'year-5'],
					licence: '3',
					price: '4.50'
				})
			)
		).toEqual({
			name: 'Year 4 maths starter',
			draft: {
				subject_areas: ['maths'],
				grades: ['year-4', 'year-5'],
				copyright_declaration_id: 3,
				price_minor_units: 450
			}
		});
	});

	it('sends free instead of a price, and never both', () => {
		const input = toInput(form({ free: true, price: '4.50' }));
		expect(input.draft.free).toBe(true);
		expect(input.draft.price_minor_units).toBeUndefined();
	});

	it('sends no price at all when one was typed badly, which the refusal already blocks', () => {
		expect(toInput(form({ subjects: ['maths'], price: 'nonsense' })).draft.price_minor_units)
			.toBeUndefined();
	});
});

describe('loading a saved template back into the form', () => {
	it('round-trips every field through the write it came from', () => {
		const original = form({
			subjects: ['maths'],
			grades: ['year-4'],
			licence: '3',
			price: '4.50'
		});
		expect(formOf(saved(toInput(original).draft))).toEqual(original);
	});

	it('loads a free template with no price rather than a zero', () => {
		expect(formOf(saved({ free: true }))).toEqual(form({ free: true }));
	});

	it('reads a draft that answers nothing as an empty form under its name', () => {
		expect(formOf(saved())).toEqual(form());
	});

	it('treats a null licence as unanswered rather than as the number zero', () => {
		expect(formOf(saved({ copyright_declaration_id: null })).licence).toBe('');
	});
});

describe('the line a listed template shows', () => {
	const now = 1000 + 3 * 24 * 60 * 60 * 1000;

	it('says saved when the template has never been changed', () => {
		expect(savedLine(head(), now)).toBe('Saved 3 days ago');
	});

	it('says changed once the two instants differ, and reads the later one', () => {
		expect(savedLine(head({ updated_at: now - 60_000 }), now)).toBe('Changed 1 min ago');
	});

	it('needs nothing the list route withholds: a head carries no draft', () => {
		// The compile-time half of this is the point — `savedLine` takes a
		// `TemplateHead`, so a future edit cannot reach for `draft` here and
		// have it type-check, which is how the first version of this tab broke.
		expect(savedLine(head(), 1000)).toBe('Saved just now');
	});
});

describe('a request for a blank form', () => {
	it('is answered when one is pending, which is the state a mount can arrive in', () => {
		// The page raises it from the mapping tab, where this component does not
		// exist; it is answered when the component appears, not before.
		expect(consumeBlankRequest(true)).toEqual({ open: true, pending: false });
	});

	it('opens nothing when none is pending, so arriving on the tab opens no form', () => {
		expect(consumeBlankRequest(false)).toEqual({ open: false, pending: false });
	});

	it('is answered once, so an effect re-run cannot blank a form being typed into', () => {
		const first = consumeBlankRequest(true);
		expect(first.open).toBe(true);
		expect(consumeBlankRequest(first.pending).open).toBe(false);
	});
});
