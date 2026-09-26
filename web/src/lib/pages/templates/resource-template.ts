// What the New resource tab's create form holds, and how a saved template
// converts to and from it. Pure, so it tests without a component: the tab
// owns the rendering and this owns the shape.
//
// The form is the whole new-resource form now, not four starting points: a
// template is a partial `DraftInput` and the create form is the thing that
// composes one, so the same bands render over the same draft (§7). What the
// template does not hold is the three things that are made per resource
// rather than chosen once — the title, the files and the pictures — and the
// marketplace grid, which is where a resource goes rather than what it says.

import type { DraftInput } from '$lib/api';
import { agoLabel } from '$lib/elapsed';
import type { InventoryId } from '$lib/generated/vocab';
import { MARKETPLACE_WORD } from '$lib/platforms';
import {
	emptyTptDraft,
	majorUnitsOf,
	minorUnitsOf,
	storedPick,
	type TptDraft
} from '$lib/tpt-form';
import type { TemplateHead, TemplateInput, TemplateView } from './api';

/** What the server bounds a name by, mirrored so the seller learns before the
 *  round trip rather than from a refusal. Kept in step with
 *  `NAME_MAX_CHARS` and `validated_name` in
 *  `crates/tam-api/src/resource_templates.rs`; the server remains the
 *  authority, and the two refusals it alone can decide — a name already used
 *  and the ceiling on how many an organisation may hold — arrive as its own
 *  words. */
export const NAME_MAX = 80;

/** The same, for the description the template carries beside its name. */
export const DESCRIPTION_MAX = 1000;

/** Everything the create form holds.
 *
 *  Three answers about the template itself, and the draft the whole
 *  new-resource form writes into. The draft is a `TptDraft` rather than a
 *  `DraftInput` because that is what the form's bands bind to; the conversion
 *  to the wire is [`toInput`], and it is where unanswered becomes absent. */
export interface TemplateForm {
	name: string;
	description: string;
	/** Which marketplace this template is written for, or `''` for a template
	 *  that suits any of them. Empty string rather than `null` because it is a
	 *  `<select>` value and the browser has no other blank. */
	scope: InventoryId | '';
	draft: TptDraft;
}

/** The draft a blank template starts from.
 *
 *  Two fields differ from a blank create form, and both are the same fact: a
 *  create form's defaults are answers, and a template's are not. A resource is
 *  drafted or live and one of the two is always true of it, so the create form
 *  starts at "draft"; a template that answered it would decide the status of
 *  every resource started from it, so it starts unanswered. The localisation
 *  tick is the sidecar's third state for the same reason. */
export function blankTemplateDraft(): TptDraft {
	return { ...emptyTptDraft(), status: '', appropriateForCountry: null };
}

export function emptyForm(): TemplateForm {
	return { name: '', description: '', scope: '', draft: blankTemplateDraft() };
}

/** A saved template loaded back into the form.
 *
 *  Editing is the same write as creating against a known identifier, so what
 *  this loads is what the next save sends: a field the stored draft leaves
 *  unanswered loads as unanswered rather than as a default the seller would
 *  then save as an answer. */
export function formOf(template: TemplateView): TemplateForm {
	return {
		name: template.name,
		description: template.description ?? '',
		scope: template.scope ?? '',
		draft: draftFormOf(template.draft)
	};
}

/** A stored partial draft as the form's bands hold it. Separate from
 *  [`formOf`] because starting a new resource from a template reads exactly
 *  this half and none of the three answers about the template itself. */
export function draftFormOf(draft: DraftInput): TptDraft {
	const blank = blankTemplateDraft();
	const free = draft.free === true;
	return {
		...blank,
		description: draft.description ?? '',
		free,
		price: free || draft.price_minor_units == null ? '' : majorUnitsOf(draft.price_minor_units),
		additionalLicence:
			free || draft.additional_licence_minor_units == null
				? ''
				: majorUnitsOf(draft.additional_licence_minor_units),
		bundleDiscount:
			free || draft.bundle_discount_minor_units == null
				? ''
				: majorUnitsOf(draft.bundle_discount_minor_units),
		taxCode: draft.tax_code_id == null ? null : String(draft.tax_code_id),
		grades: [...(draft.grades ?? [])],
		subjectAreas: [...(draft.subject_areas ?? [])],
		tags: [...(draft.tags ?? [])],
		formats: [...(draft.formats ?? [])],
		customCategories: [...(draft.custom_categories ?? [])],
		appropriateForCountry: draft.appropriate_for_country ?? null,
		standards: (draft.standards ?? []).map((standard) =>
			storedPick({
				framework: standard.framework,
				code: standard.code,
				tpt_node_id: standard.tpt_node_id ?? null
			})
		),
		teachingDuration:
			draft.teaching_duration_id == null ? null : String(draft.teaching_duration_id),
		pagesOrSlides: draft.pages_or_slides == null ? '' : String(draft.pages_or_slides),
		answerKey: draft.answer_key_id == null ? null : String(draft.answer_key_id),
		copyright:
			draft.copyright_declaration_id == null ? null : String(draft.copyright_declaration_id),
		status: draft.status_user == null ? '' : String(draft.status_user)
	};
}

/** Why this form cannot be saved, in the words the seller reads, or null when
 *  it can. One function rather than a boolean beside a message, so the
 *  disabled control and the reason it states cannot drift apart. */
export function refusalOf(form: TemplateForm): string | null {
	const name = form.name.trim();
	if (name.length === 0) {
		return 'Give the template a name.';
	}
	if ([...name].length > NAME_MAX) {
		return `Keep the name to ${NAME_MAX} characters or fewer.`;
	}
	if (/\p{Cc}/u.test(name)) {
		return 'Remove the line break or hidden character from the name.';
	}
	if ([...form.description.trim()].length > DESCRIPTION_MAX) {
		return `Keep the template note to ${DESCRIPTION_MAX} characters or fewer.`;
	}
	const draft = form.draft;
	if (!draft.free && draft.price.trim().length > 0 && minorUnitsOf(draft.price) === null) {
		return 'Enter the price in dollars and cents, like 4.50.';
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
	const held = form.draft;
	const draft: DraftInput = {};
	// The resource's own description, which the form's Description band holds.
	// The template's own note is `form.description` and is a different field
	// on a different row; the two are never the same string.
	const body = held.description.trim();
	if (body.length > 0) {
		draft.description = body;
	}
	if (held.free) {
		draft.free = true;
	} else {
		const price = minorUnitsOf(held.price);
		if (price !== null) {
			draft.price_minor_units = price;
		}
		const additional = minorUnitsOf(held.additionalLicence);
		if (additional !== null) {
			draft.additional_licence_minor_units = additional;
		}
		const bundle = minorUnitsOf(held.bundleDiscount);
		if (bundle !== null) {
			draft.bundle_discount_minor_units = bundle;
		}
		if (held.taxCode !== null) {
			draft.tax_code_id = Number(held.taxCode);
		}
	}
	if (held.grades.length > 0) {
		draft.grades = [...held.grades];
	}
	if (held.subjectAreas.length > 0) {
		draft.subject_areas = [...held.subjectAreas];
	}
	if (held.tags.length > 0) {
		draft.tags = [...held.tags];
	}
	if (held.formats.length > 0) {
		draft.formats = [...held.formats];
	}
	if (held.customCategories.length > 0) {
		draft.custom_categories = [...held.customCategories];
	}
	if (held.appropriateForCountry !== null) {
		draft.appropriate_for_country = held.appropriateForCountry;
	}
	if (held.standards.length > 0) {
		draft.standards = held.standards.map((pick) => ({
			framework: pick.framework,
			code: pick.code,
			tpt_node_id: pick.tpt_node_id ?? null
		}));
	}
	if (held.teachingDuration !== null) {
		draft.teaching_duration_id = Number(held.teachingDuration);
	}
	const pages = pagesOf(held.pagesOrSlides);
	if (pages !== null) {
		draft.pages_or_slides = pages;
	}
	if (held.answerKey !== null) {
		draft.answer_key_id = Number(held.answerKey);
	}
	if (held.copyright !== null) {
		draft.copyright_declaration_id = Number(held.copyright);
	}
	if (held.status !== '') {
		draft.status_user = Number(held.status);
	}
	const note = form.description.trim();
	return {
		name: form.name.trim(),
		description: note.length > 0 ? note : null,
		scope: form.scope === '' ? null : form.scope,
		draft
	};
}

function pagesOf(typed: string): number | null {
	const trimmed = typed.trim();
	if (!/^\d+$/.test(trimmed)) {
		return null;
	}
	const pages = Number(trimmed);
	return pages > 0 ? pages : null;
}

// ------------------------------------------------------------------- scope

/** The scopes a template is written for, in the order the select shows them.
 *
 *  Etsy is offered and disabled rather than left out: a seller who sells there
 *  is answered, and the reason is the same one the create form's tile gives. */
export const SCOPE_CHOICES: readonly {
	value: InventoryId | '';
	label: string;
	reason: string | null;
}[] = [
	{ value: '', label: 'Any marketplace', reason: null },
	{ value: 'Tes', label: 'Tes', reason: null },
	{ value: 'Tpt', label: 'TPT', reason: null },
	{ value: 'Etsy', label: 'Etsy', reason: 'Not connected yet.' }
];

/** How a listed template says what it is written for. */
export function scopeLabel(scope: InventoryId | null): string {
	return scope === null ? 'Any marketplace' : MARKETPLACE_WORD[scope];
}

/** Why this template cannot be the one a pull rule fills from, or null.
 *
 *  A rule publishes a pulled resource to the marketplaces it names, so a
 *  template written for a marketplace the rule does not publish to would fill
 *  fields nothing carries. A generic template suits every rule. */
export function scopeRefusal(
	scope: InventoryId | null,
	publishTo: readonly InventoryId[]
): string | null {
	if (scope === null || publishTo.includes(scope)) {
		return null;
	}
	return `Written for ${MARKETPLACE_WORD[scope]}, which this rule does not publish to.`;
}

/** What the apply plan's own field names are called on screen.
 *
 *  The server answers in the draft's own vocabulary — `price_minor_units`,
 *  `copyright_declaration_id` — because that is what it writes; the seller
 *  reads the words the form asks in. An unlisted name falls back to itself
 *  with the underscores taken out, so a field the server adds tomorrow reads
 *  as a field rather than as a blank. */
const WIRE_WORDS: Record<string, string> = {
	description: 'description',
	free: 'price',
	price_minor_units: 'price',
	additional_licence_minor_units: 'multiple licenses',
	bundle_discount_minor_units: 'bundle discount',
	tax_code_id: 'tax code',
	grades: 'year levels',
	subject_areas: 'subjects',
	tags: 'tags',
	formats: 'formats',
	custom_categories: 'custom categories',
	appropriate_for_country: 'localization',
	standards: 'standards',
	teaching_duration_id: 'teaching duration',
	pages_or_slides: 'pages or slides',
	answer_key_id: 'answer key',
	copyright_declaration_id: 'copyright',
	status_user: 'status',
	rights: 'licence'
};

export function fieldWordsOf(field: string): string {
	return WIRE_WORDS[field] ?? field.replace(/_/g, ' ');
}

/** The fields a saved draft answers, in the words the form asks them in.
 *
 *  Read off the wire draft rather than a form, so the Templates page can say
 *  what a saved template sets without loading it into the editor. Only the
 *  fields a template can hold are named (`WIRE_WORDS`): the title, files and
 *  pictures are per resource. A field counts as answered when it carries a
 *  value — a number, a non-empty text or list, `free` ticked, or the
 *  localisation tick answered either way — and each word is named once, so
 *  the price and its free tick read as one "price". */
export function fieldsSet(draft: DraftInput): string[] {
	const words: string[] = [];
	for (const [field, value] of Object.entries(draft)) {
		if (!Object.hasOwn(WIRE_WORDS, field)) {
			continue;
		}
		const answered =
			field === 'free'
				? value === true
				: Array.isArray(value)
					? value.length > 0
					: typeof value === 'string'
						? value.trim().length > 0
						: value !== null && value !== undefined;
		const word = WIRE_WORDS[field];
		if (answered && !words.includes(word)) {
			words.push(word);
		}
	}
	return words;
}

/** What a verdict is called on screen, in the three words the seller reads. */
export const VERDICT_WORDS: Record<string, string> = {
	will_change: 'Will change',
	unchanged: 'Unchanged',
	blocked: 'Blocked'
};

// --------------------------------------------------- starting from one

/** The words a filled field is named by in the note the form shows. Keyed by
 *  the draft's own field so the note cannot name a field the merge did not
 *  write. */
const FILL_WORDS: Partial<Record<keyof TptDraft, string>> = {
	description: 'description',
	free: 'price',
	price: 'price',
	additionalLicence: 'multiple licenses',
	bundleDiscount: 'bundle discount',
	taxCode: 'tax code',
	grades: 'year levels',
	subjectAreas: 'subjects',
	tags: 'tags',
	formats: 'formats',
	customCategories: 'custom categories',
	appropriateForCountry: 'localization',
	standards: 'standards',
	teachingDuration: 'teaching duration',
	pagesOrSlides: 'pages or slides',
	answerKey: 'answer key',
	copyright: 'copyright',
	status: 'status'
};

/** What the merge changed: the draft to hold, and the fields it filled.
 *
 *  The draft before the merge travels too, so Undo puts back exactly what the
 *  seller had rather than a blank form — a seller who had typed a title and
 *  then tried a template must not lose the title by changing their mind. */
export interface Merged {
	draft: TptDraft;
	filled: string[];
	before: TptDraft;
}

/** A template merged into a draft, filling only what the draft has not
 *  answered.
 *
 *  "Not answered" is "still what a blank form starts with", which is why the
 *  status and the localisation tick can be filled at all: their blank-form
 *  values are defaults rather than answers, and a seller who chose one keeps
 *  it. The title, the files and the pictures are not merged because a template
 *  does not hold them. */
export function mergeIntoEmpty(draft: TptDraft, template: DraftInput): Merged {
	const blank = emptyTptDraft();
	const from = draftFormOf(template);
	const merged: TptDraft = { ...draft };
	const filled: string[] = [];

	function take<K extends keyof TptDraft>(field: K, stated: boolean) {
		if (!stated || !isBlank(draft, blank, field)) {
			return;
		}
		merged[field] = from[field];
		// Named only where the value actually moved. A template whose status is
		// the blank form's own "draft" fills a field that already read that
		// way, and a note claiming it filled the status would be reporting a
		// change the seller cannot see.
		const word = FILL_WORDS[field];
		if (word === undefined || filled.includes(word) || same(draft[field], from[field])) {
			return;
		}
		filled.push(word);
	}

	take('description', from.description !== '');
	// The pricing branch travels whole or not at all: a template that says
	// "free" and a draft that took only its price would carry a price under a
	// free resource, which is the one combination the form refuses.
	if (from.free && isBlank(draft, blank, 'free') && isBlank(draft, blank, 'price')) {
		merged.free = true;
		filled.push('price');
	} else if (!from.free) {
		take('price', from.price !== '');
		take('additionalLicence', from.additionalLicence !== '');
		take('bundleDiscount', from.bundleDiscount !== '');
		take('taxCode', from.taxCode !== null);
	}
	take('grades', from.grades.length > 0);
	take('subjectAreas', from.subjectAreas.length > 0);
	take('tags', from.tags.length > 0);
	take('formats', from.formats.length > 0);
	take('customCategories', from.customCategories.length > 0);
	take('appropriateForCountry', from.appropriateForCountry !== null);
	take('standards', from.standards.length > 0);
	take('teachingDuration', from.teachingDuration !== null);
	take('pagesOrSlides', from.pagesOrSlides !== '');
	take('answerKey', from.answerKey !== null);
	take('copyright', from.copyright !== null);
	take('status', from.status !== '');

	return { draft: merged, filled, before: draft };
}

/** Whether one field of a draft still holds what a blank form starts with. */
function isBlank<K extends keyof TptDraft>(draft: TptDraft, blank: TptDraft, field: K): boolean {
	const held = draft[field];
	if (Array.isArray(held)) {
		return held.length === 0;
	}
	return held === blank[field];
}

/** Whether two of a draft's values read the same. Lists compare by their
 *  members, because a filled list and the list it replaced are two arrays and
 *  never the same reference. */
function same(held: unknown, taken: unknown): boolean {
	if (Array.isArray(held) && Array.isArray(taken)) {
		return held.length === taken.length && held.every((one, at) => one === taken[at]);
	}
	return held === taken;
}

/** The one line the form says after a template was taken. Names the fields so
 *  a seller can see what changed without hunting the form for it, and says so
 *  plainly when a template had nothing this draft was missing. */
export function filledLine(name: string, filled: readonly string[]): string {
	if (filled.length === 0) {
		return `“${name}” had nothing new to fill in.`;
	}
	return `“${name}” filled in ${filled.join(', ')}. Your own answers were kept.`;
}

// ------------------------------------------------------- the two examples

/** One worked example a seller can start a template from.
 *
 *  Held as data rather than as markup so the same two cards can be rendered,
 *  merged and tested without a component, and so what an example states is
 *  reviewable in one place.
 *
 *  What an example states is one field of the draft — the resource's own
 *  description — beside the three answers about the template itself. That
 *  restraint is the point. Every other field the form holds is either chosen
 *  from a served vocabulary (the year levels, the subjects, the formats, the
 *  teaching duration, the answer key), or an answer that is the seller's alone
 *  to make (the price, the licence, the copyright declaration, the status, the
 *  localisation tick). An example that filled one of those would either carry
 *  an identifier no vocabulary serves or decide something on the seller's
 *  behalf, and a template's whole job is to be a starting point rather than a
 *  decision. The status and the localisation tick stay unanswered here for
 *  exactly the reason [`blankTemplateDraft`] leaves them unanswered.
 *
 *  The body is Markdown, which is what a blank draft's `bodyFormat` already
 *  declares and what every adapter renders; it holds no substitution of any
 *  kind, because the renderer supports none. */
export interface TemplateExample {
	/** Which example this is, and the key its card is rendered under. */
	id: 'tes' | 'tpt';
	/** What its control says, and the name the filled-fields note uses. */
	label: string;
	/** One line under the label, saying what this example suits. */
	summary: string;
	/** The name the template starts with. The seller renames it; nothing here
	 *  is saved until they press Save. */
	name: string;
	/** The template's own note — when to reach for it — which is a different
	 *  field from the resource description below. */
	note: string;
	scope: InventoryId;
	/** The resource description, in the words a teacher reads. */
	body: string;
}

const TES_BODY = `**What this resource is for**
Pupils practise one skill in short guided steps and then apply it on their own. Say here which skill it is and which year group it suits.

**What is included**
- A teaching page to work through together
- Two pages of independent practice
- An answer sheet for marking

**How to use it in class**
1. Work the first example through together on the board.
2. Set the practice pages for independent or paired work.
3. Mark from the answer sheet and pick up the misconceptions it shows.

**Also useful for** homework, cover lessons and small intervention groups.`;

const TPT_BODY = `**What students will learn**
Students practise one skill in short guided steps and then apply it independently. Name the skill and the grade level it fits.

**What's included**
- A teaching page to work through together
- Two independent practice pages
- An answer key

**How to use it in your classroom**
1. Work the first example together as a warm-up.
2. Assign the practice pages for independent or partner work.
3. Review with the answer key and reteach what it surfaces.

**Also works for** homework, sub plans and small-group intervention.`;

export const EXAMPLES: readonly TemplateExample[] = [
	{
		id: 'tes',
		label: 'Tes example',
		summary: 'Written for Tes, in British wording: pupils, year groups, marking.',
		name: 'Tes lesson starter',
		note: 'A starting point for a Tes resource: what it is for, what is included, and how to use it in class.',
		scope: 'Tes',
		body: TES_BODY
	},
	{
		id: 'tpt',
		label: 'TPT example',
		summary: 'Written for TPT, in American wording: students, grade levels, answer keys.',
		name: 'TPT lesson starter',
		note: "A starting point for a TPT resource: what students will learn, what's included, and how to use it.",
		scope: 'Tpt',
		body: TPT_BODY
	}
];

/** What starting from an example changed: the form to hold, the fields it
 *  filled, and the form as it was.
 *
 *  The form before the start travels for the reason [`Merged`] carries the
 *  draft before a merge — Undo puts back exactly what the seller had, so
 *  trying an example is never a way to lose typing. */
export interface Started {
	form: TemplateForm;
	filled: string[];
	before: TemplateForm;
}

/** An example started from, filling only what the form has not answered.
 *
 *  The draft half is [`mergeIntoEmpty`], which is the same fill-the-empty-ones
 *  rule the create form's template picker and the apply route both follow, so
 *  there is one merge here rather than a second one written for examples. The
 *  three answers about the template itself are filled on the same rule: a name
 *  the seller has typed, a note they have written and a marketplace they have
 *  chosen are all left alone.
 *
 *  Nothing is written anywhere. An example is an unsaved form until the seller
 *  presses Save, so opening this page or pressing a card costs no template and
 *  no allowance. */
export function startFromExample(form: TemplateForm, example: TemplateExample): Started {
	const merged = mergeIntoEmpty(form.draft, { description: example.body });
	const next: TemplateForm = { ...form, draft: merged.draft };
	const filled: string[] = [];
	if (form.name.trim() === '') {
		next.name = example.name;
		filled.push('name');
	}
	filled.push(...merged.filled);
	if (form.description.trim() === '') {
		next.description = example.note;
		filled.push('note');
	}
	if (form.scope === '') {
		next.scope = example.scope;
		filled.push('written for');
	}
	return { form: next, filled, before: form };
}

/** Whether this form still holds exactly what a blank one starts with, which
 *  is what decides whether clearing it is something to offer an Undo for.
 *  Compared field by field against a blank rather than by a dirty flag, so a
 *  seller who typed a word and deleted it again is not warned about nothing. */
export function isUntouched(form: TemplateForm): boolean {
	if (form.name !== '' || form.description !== '' || form.scope !== '') {
		return false;
	}
	const blank = blankTemplateDraft();
	// `overrides` is the one field a blank draft starts as an object, and two
	// empty objects are never the same reference, so it is compared by its
	// entries and every other field by the reading `same` already holds.
	if (Object.keys(form.draft.overrides).length > 0) {
		return false;
	}
	return (Object.keys(blank) as (keyof TptDraft)[]).every(
		(field) => field === 'overrides' || same(form.draft[field], blank[field])
	);
}

/** Whether two forms read the same, field by field.
 *
 *  What Undo asks before it acts: an example or a clear travels with the form
 *  it produced, and a form that no longer reads that way is one the seller has
 *  written in since. Compared by value rather than by reference because every
 *  write to the form makes a new object, so two forms holding the same answers
 *  are never the same one.
 *
 *  `overrides` is compared by its entries for the reason [`isUntouched`]
 *  gives: it is the one field a draft holds as an object. */
export function sameForm(held: TemplateForm, taken: TemplateForm): boolean {
	if (
		held.name !== taken.name ||
		held.description !== taken.description ||
		held.scope !== taken.scope
	) {
		return false;
	}
	const keys = Object.keys(held.draft.overrides);
	if (
		keys.length !== Object.keys(taken.draft.overrides).length ||
		!keys.every((key) => held.draft.overrides[key] === taken.draft.overrides[key])
	) {
		return false;
	}
	return (Object.keys(blankTemplateDraft()) as (keyof TptDraft)[]).every(
		(field) => field === 'overrides' || same(held.draft[field], taken.draft[field])
	);
}

/** The line the editor says after it was cleared with something in it. Its own
 *  sentence rather than [`filledLine`]'s empty case, which reports a template
 *  that filled nothing — the opposite fact. */
export const CLEARED_LINE = 'You cleared the form.';

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

/** Whether a request to clear the editor is owed, and what is left after
 *  answering it.
 *
 *  The request lives on the page because the page's own header action raises
 *  it, including from the mapping tab, while the form it clears is the tab's
 *  own state. A count passed as a prop cannot express it: one already-raised
 *  count is indistinguishable from the next, so the tab would have no mark to
 *  compare against.
 *
 *  Answering exactly once is the property worth holding. The tab reads this
 *  from an effect that re-runs whenever anything it touches changes, and a
 *  request that stayed pending would clear the form again under whatever the
 *  seller had begun typing. */
export function consumeBlankRequest(pending: boolean): { open: boolean; pending: boolean } {
	return pending ? { open: true, pending: false } : { open: false, pending: false };
}
