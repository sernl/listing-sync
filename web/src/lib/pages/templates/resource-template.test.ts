import { describe, expect, it } from 'vitest';
import { emptyTptDraft, storedPick, type TptDraft } from '$lib/tpt-form';
import {
	DESCRIPTION_MAX,
	NAME_MAX,
	blankTemplateDraft,
	consumeBlankRequest,
	emptyForm,
	fieldWordsOf,
	filledLine,
	formOf,
	isComplete,
	mergeIntoEmpty,
	refusalOf,
	savedLine,
	scopeRefusal,
	toInput,
	type TemplateForm
} from './resource-template';
import type { TemplateHead, TemplateView } from './api';

function draftWith(patch: Partial<TptDraft> = {}): TptDraft {
	return { ...blankTemplateDraft(), ...patch };
}

function form(patch: Partial<TemplateForm> = {}): TemplateForm {
	return { ...emptyForm(), name: 'Year 4 maths starter', ...patch };
}

/** A template as the read serves it, from the write the form composed. Named
 *  off the input so a round-trip test cannot quietly compare a hand-written
 *  view against a form it never came from. */
function saved(input = toInput(form())): TemplateView {
	return {
		id: 'tpl-1',
		name: input.name,
		description: input.description,
		scope: input.scope,
		created_at: 5,
		updated_at: 7,
		draft: input.draft
	};
}

function head(patch: Partial<TemplateHead> = {}): TemplateHead {
	return {
		id: 'tpl-1',
		name: 'Year 4 maths starter',
		description: null,
		scope: null,
		created_at: 1000,
		updated_at: 1000,
		...patch
	};
}

describe('what a template may be saved with', () => {
	it('refuses a template with no name, because a list of them needs telling apart', () => {
		expect(refusalOf(form({ name: '   ' }))).toMatch(/name/);
	});

	it('refuses a name past the length the server stores, before the round trip', () => {
		expect(refusalOf(form({ name: 'a'.repeat(NAME_MAX + 1) }))).toMatch(/80/);
		expect(isComplete(form({ name: 'a'.repeat(NAME_MAX) }))).toBe(true);
	});

	it('counts a name in characters rather than code units, as the server does', () => {
		// Eighty astral characters are eighty characters and 160 code units;
		// `.length` would refuse a name the server stores.
		expect(isComplete(form({ name: '\u{1D400}'.repeat(NAME_MAX) }))).toBe(true);
	});

	it('refuses a control character in the name, which the column cannot hold', () => {
		expect(refusalOf(form({ name: 'Year 4\u0000maths' }))).toMatch(/hidden character/);
	});

	it('refuses a description past the length the column holds', () => {
		expect(refusalOf(form({ description: 'a'.repeat(DESCRIPTION_MAX + 1) }))).toMatch(/1000/);
		expect(isComplete(form({ description: 'a'.repeat(DESCRIPTION_MAX) }))).toBe(true);
	});

	it('saves a name and nothing else, which the server calls the blank template', () => {
		// `validated_draft(&json!({})).is_ok()` in resource_templates.rs, under
		// the sentence naming it the blank template a seller starts from. A
		// stricter client rule would be a second rule about one fact.
		expect(isComplete(form())).toBe(true);
		expect(toInput(form())).toEqual({
			name: 'Year 4 maths starter',
			description: null,
			scope: null,
			draft: {}
		});
	});

	it('refuses a price that is not a number, rather than dropping what was typed', () => {
		expect(refusalOf(form({ draft: draftWith({ price: 'four fifty' }) }))).toMatch(/4\.50/);
	});

	it('ignores an unparseable price once the resource is free, which answers the price', () => {
		expect(isComplete(form({ draft: draftWith({ free: true, price: 'four fifty' }) }))).toBe(
			true
		);
	});

	it('still refuses a form with no name at all, which is the one bound left', () => {
		expect(isComplete(emptyForm())).toBe(false);
	});
});

describe('the write a form names', () => {
	it('leaves every unanswered field out, so a template stays a partial draft', () => {
		expect(toInput(form({ draft: draftWith({ subjectAreas: ['maths'] }) })).draft).toEqual({
			subject_areas: ['maths']
		});
	});

	it('sends free instead of a price, and never both', () => {
		const input = toInput(form({ draft: draftWith({ free: true, price: '4.50' }) }));
		expect(input.draft.free).toBe(true);
		expect(input.draft.price_minor_units).toBeUndefined();
	});

	it('sends no price at all when one was typed badly, which the refusal already blocks', () => {
		const input = toInput(form({ draft: draftWith({ price: 'nonsense' }) }));
		expect(input.draft.price_minor_units).toBeUndefined();
	});

	it('sends no status until one is chosen, so a template does not decide it', () => {
		expect(toInput(form()).draft.status_user).toBeUndefined();
		expect(toInput(form({ draft: draftWith({ status: '1' }) })).draft.status_user).toBe(1);
	});

	it('sends an unticked localization box only once it has been answered', () => {
		expect(toInput(form()).draft.appropriate_for_country).toBeUndefined();
		const answered = form({ draft: draftWith({ appropriateForCountry: false }) });
		expect(toInput(answered).draft.appropriate_for_country).toBe(false);
	});

	it('carries the scope and the description beside the draft', () => {
		const input = toInput(form({ description: '  For Tes uploads  ', scope: 'Tes' }));
		expect(input.description).toBe('For Tes uploads');
		expect(input.scope).toBe('Tes');
	});
});

describe('loading a saved template back into the form', () => {
	it('round-trips the whole new-resource form through the write it came from', () => {
		const original = form({
			name: 'Year 4 maths starter',
			description: 'The one I use for worksheets',
			scope: 'Tpt',
			draft: draftWith({
				description: 'A worksheet for year 4.',
				price: '4.50',
				additionalLicence: '3.15',
				bundleDiscount: '9.00',
				taxCode: '7',
				grades: ['year-4', 'year-5'],
				subjectAreas: ['maths'],
				tags: ['worksheets', 'printable'],
				formats: ['pdf'],
				customCategories: ['My best sellers'],
				appropriateForCountry: true,
				standards: [storedPick({ framework: 2, code: '4.NBT.A.1', tpt_node_id: 91 })],
				teachingDuration: '3',
				pagesOrSlides: '12',
				answerKey: '1',
				copyright: '2',
				status: '1'
			})
		});
		expect(formOf(saved(toInput(original)))).toEqual(original);
	});

	it('loads a free template with no price rather than a zero', () => {
		const free = form({ draft: draftWith({ free: true }) });
		expect(formOf(saved(toInput(free)))).toEqual(free);
	});

	it('reads a draft that answers nothing as an empty form under its name', () => {
		expect(formOf(saved())).toEqual(form());
	});

	it('treats a null licence as unanswered rather than as the number zero', () => {
		const view = saved();
		expect(formOf({ ...view, draft: { copyright_declaration_id: null } }).draft.copyright).toBe(
			null
		);
	});

	it('reads a generic template as the blank scope the select shows', () => {
		expect(formOf(saved()).scope).toBe('');
	});
});

describe('starting a new resource from a template', () => {
	it('fills a field the draft has not answered', () => {
		const merged = mergeIntoEmpty(emptyTptDraft(), { subject_areas: ['maths'] });
		expect(merged.draft.subjectAreas).toEqual(['maths']);
		expect(merged.filled).toEqual(['subjects']);
	});

	it('leaves a field the seller has already answered alone', () => {
		const typed = { ...emptyTptDraft(), subjectAreas: ['science'], description: 'Mine.' };
		const merged = mergeIntoEmpty(typed, {
			subject_areas: ['maths'],
			description: 'The template’s.'
		});
		expect(merged.draft.subjectAreas).toEqual(['science']);
		expect(merged.draft.description).toBe('Mine.');
		expect(merged.filled).toEqual([]);
	});

	it('never touches the title, which no template holds', () => {
		const typed = { ...emptyTptDraft(), name: 'Fractions pack' };
		expect(mergeIntoEmpty(typed, { name: 'A template title' }).draft.name).toBe('Fractions pack');
	});

	it('takes free without also taking a price, which the form would refuse', () => {
		const merged = mergeIntoEmpty(emptyTptDraft(), { free: true });
		expect(merged.draft.free).toBe(true);
		expect(merged.draft.price).toBe('');
		expect(merged.filled).toEqual(['price']);
	});

	it('does not make a priced draft free, because the price is already answered', () => {
		const priced = { ...emptyTptDraft(), price: '9.00' };
		const merged = mergeIntoEmpty(priced, { free: true });
		expect(merged.draft.free).toBe(false);
		expect(merged.draft.price).toBe('9.00');
	});

	it('fills the status and the localization box, whose blank values are defaults', () => {
		const merged = mergeIntoEmpty(emptyTptDraft(), {
			status_user: 1,
			appropriate_for_country: true
		});
		expect(merged.draft.status).toBe('1');
		expect(merged.draft.appropriateForCountry).toBe(true);
	});

	it('names no field whose value did not move, so the note reports only changes', () => {
		// The blank form starts at "draft" and this template answers "draft",
		// so the field is written and nothing about it changed.
		const merged = mergeIntoEmpty(emptyTptDraft(), { status_user: 0, subject_areas: ['maths'] });
		expect(merged.draft.status).toBe('0');
		expect(merged.filled).toEqual(['subjects']);
	});

	it('keeps what the draft held, so an undo puts it back exactly', () => {
		const typed = { ...emptyTptDraft(), name: 'Fractions pack' };
		const merged = mergeIntoEmpty(typed, { subject_areas: ['maths'] });
		expect(merged.before).toBe(typed);
		expect(merged.before.subjectAreas).toEqual([]);
	});

	it('says so plainly when a template had nothing this draft was missing', () => {
		expect(filledLine('Worksheets', [])).toMatch(/filled nothing/);
		expect(filledLine('Worksheets', ['subjects', 'price'])).toMatch(/subjects, price/);
	});
});

describe('which template a pull rule may fill from', () => {
	it('accepts a generic template under any rule', () => {
		expect(scopeRefusal(null, [])).toBeNull();
		expect(scopeRefusal(null, ['Tpt'])).toBeNull();
	});

	it('accepts a scoped template where the rule publishes to that marketplace', () => {
		expect(scopeRefusal('Tpt', ['Tpt'])).toBeNull();
	});

	it('refuses a scoped template the rule does not publish to, as the server does', () => {
		expect(scopeRefusal('Tpt', ['Tes'])).toMatch(/TPT/);
		expect(scopeRefusal('Tes', [])).toMatch(/does not publish to/);
	});
});

describe('the words a plan row is read in', () => {
	it('names a wire field in the words the form asks it in', () => {
		expect(fieldWordsOf('copyright_declaration_id')).toBe('copyright');
		expect(fieldWordsOf('price_minor_units')).toBe('price');
	});

	it('reads a field this bundle does not know as a field rather than as a blank', () => {
		expect(fieldWordsOf('some_new_field')).toBe('some new field');
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
