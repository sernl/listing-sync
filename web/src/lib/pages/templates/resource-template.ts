// What the New resource tab's create form holds, and how a saved template
// converts to and from it. Pure, so it tests without a component: the tab
// owns the rendering and this owns the shape.

import type { DraftInput } from '$lib/api';
import { agoLabel } from '$lib/elapsed';
import { majorUnitsOf, minorUnitsOf } from '$lib/tpt-form';
import type { TemplateHead, TemplateInput, TemplateView } from './api';

/** What the server bounds a name by, mirrored so the seller learns before the
 *  round trip rather than from a refusal. Kept in step with
 *  `NAME_MAX_CHARS` and `validated_name` in
 *  `crates/tam-api/src/resource_templates.rs`; the server remains the
 *  authority, and the two refusals it alone can decide — a name already used
 *  and the ceiling on how many an organisation may hold — arrive as its own
 *  words. */
export const NAME_MAX = 80;

/** Everything the create form holds.
 *
 *  Four starting points and a name, which are the four the empty state names:
 *  subject, year levels, licence and price. The price is the typed string
 *  rather than minor units, because a half-typed amount is a string and
 *  converting on every keystroke would refuse "4." while the seller is still
 *  typing "4.50". */
export interface TemplateForm {
	name: string;
	subjects: string[];
	grades: string[];
	licence: string;
	free: boolean;
	price: string;
}

export function emptyForm(): TemplateForm {
	return { name: '', subjects: [], grades: [], licence: '', free: false, price: '' };
}

/** A saved template loaded back into the form.
 *
 *  Editing is the same write as creating against a known identifier, so what
 *  this loads is what the next save sends: a field the stored draft leaves
 *  unanswered loads as unanswered rather than as a default the seller would
 *  then save as an answer. */
export function formOf(template: TemplateView): TemplateForm {
	const draft = template.draft;
	return {
		name: template.name,
		subjects: [...(draft.subject_areas ?? [])],
		grades: [...(draft.grades ?? [])],
		licence: draft.copyright_declaration_id == null ? '' : String(draft.copyright_declaration_id),
		free: draft.free === true,
		price:
			draft.price_minor_units == null || draft.free === true
				? ''
				: majorUnitsOf(draft.price_minor_units)
	};
}

/** Why this form cannot be saved, in the words the seller reads, or null when
 *  it can. One function rather than a boolean beside a message, so the
 *  disabled control and the reason it states cannot drift apart. */
export function refusalOf(form: TemplateForm): string | null {
	const name = form.name.trim();
	if (name.length === 0) {
		return 'Give the template a name, so you can tell it from the others.';
	}
	if ([...name].length > NAME_MAX) {
		return `A name is at most ${NAME_MAX} characters.`;
	}
	if (/\p{Cc}/u.test(name)) {
		return 'A name cannot carry a line break or other control character.';
	}
	if (!form.free && form.price.trim().length > 0 && minorUnitsOf(form.price) === null) {
		return 'A price is a number of dollars and cents, like 4.50.';
	}
	// A name and nothing else saves. `validated_draft` accepts `{}` and the
	// server's own test calls it the blank template a seller starts from
	// (`resource_templates.rs`), so refusing it here would be a second rule
	// about one fact, and the stricter of the two.
	return null;
}

export function isComplete(form: TemplateForm): boolean {
	return refusalOf(form) === null;
}

/** The form as the write names it.
 *
 *  Every unanswered field is left out rather than sent empty, because a
 *  template is a partial draft: an empty list or a zero price sent as an
 *  answer would pre-fill a new resource with a decision the seller never
 *  made. `free` is the exception and is sent whenever it is true, since
 *  free is itself an answer about the price. */
export function toInput(form: TemplateForm): TemplateInput {
	const draft: DraftInput = {};
	if (form.subjects.length > 0) {
		draft.subject_areas = [...form.subjects];
	}
	if (form.grades.length > 0) {
		draft.grades = [...form.grades];
	}
	if (form.licence.length > 0) {
		draft.copyright_declaration_id = Number(form.licence);
	}
	if (form.free) {
		draft.free = true;
	} else {
		const minor = minorUnitsOf(form.price);
		if (minor !== null) {
			draft.price_minor_units = minor;
		}
	}
	return { name: form.name.trim(), draft };
}

/** The one meta line a listed template shows.
 *
 *  Built from the two instants because they are the only facts the list has:
 *  `GET /v1/templates` serves heads without drafts on purpose, so a line
 *  naming subjects or a price would need a read per row of a list that exists
 *  to avoid exactly that. A template saved and never touched says so rather
 *  than reporting the same instant under two different words. */
export function savedLine(head: TemplateHead, now: number): string {
	return head.updated_at === head.created_at
		? `Saved ${agoLabel(head.created_at, now)}`
		: `Changed ${agoLabel(head.updated_at, now)}`;
}

/** Whether a request for a blank form is owed, and what is left after
 *  answering it.
 *
 *  The request lives on the page rather than in the tab because the tab is
 *  destroyed when the seller is on the other one, and the page's own action
 *  can raise a request from there — the landing path, and the first press a
 *  seller makes. A count passed as a prop cannot express it: the tab mounts
 *  with the count already raised and has no mark to compare it against, so a
 *  mount and a raised request look identical.
 *
 *  Answering exactly once is the property worth holding. The tab reads this
 *  from an effect that re-runs whenever anything it touches changes, and a
 *  request that stayed pending would blank the form again under whatever the
 *  seller had begun typing. */
export function consumeBlankRequest(pending: boolean): { open: boolean; pending: boolean } {
	return pending ? { open: true, pending: false } : { open: false, pending: false };
}
