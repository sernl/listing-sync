// The create form's own model, on the canonical TPT base: what the seller has
// typed, what its controls hold, and the two request bodies it composes.
//
// The refusals are no longer stated here. They are decided by the compiled
// core in `$lib/core`, which is the same function `POST /v1/authoring/check`
// calls, so the message a seller reads as they type and the answer the server
// gives are one answer rather than two implementations that agreed when they
// were last compared (D28). Exactly one rule is added here — that a listing
// names at least one marketplace — because it is a decision about this listing
// and not a property of the product the domain describes.
//
// What remains is presentation and wire assembly: the counters, the picker
// toggles, the facet search, the grade grid, the price parsing and the two
// request bodies. None of those is a rule, and the core answers none of them.

import type {
	CreateProductBody,
	DraftInput,
	FacetView,
	FileHandle,
	FormCaps,
	FormVocabularyView,
	PathInput,
	PriceIntent,
	TptBaseInput,
	VocabularyView
} from '$lib/api';
import type { FormGroup, InventoryId, TermKind } from '$lib/generated/vocab';
import { core, loadCore } from '$lib/core';
import { MARKETPLACE_OF } from '$lib/listings-view';

// Started at module scope so the rules are ready before the seller has typed
// anything; guarded because there is no asset to fetch while prerendering.
if (typeof window !== 'undefined') {
	void loadCore();
}

/** TPT's create form is nine sections and every one of them is a heading the
 *  seller reads. Education Standards is lifted out of Categories, where TPT
 *  nests it, because a jurisdiction picker is not a category picker. */
export const GROUP_HEADINGS: Record<FormGroup, string> = {
	name: 'Name',
	files: 'Files',
	description: 'Description',
	price: 'Price',
	categories: 'Categories',
	education_standards: 'Education Standards',
	details: 'Details',
	copyright: 'Copyright',
	product_status: 'Product Status'
};

/** TPT's own helper text, quoted where the DOM carries it, so a seller who
 *  knows the TPT form reads the same sentence here. */
export const GROUP_HELP: Partial<Record<FormGroup, string>> = {
	files: 'The file buyers download, an optional preview, and the images that front the listing.',
	price: 'Free hides the price and the tax code, exactly as it does on TPT.',
	categories: 'How buyers find this. Each picker is its own vocabulary and its own limit.',
	details: 'Nothing here is required.',
	product_status: 'Active listings are visible on the site and searchable. Inactive listings are only visible to you.'
};

/** The Education Standards heading's helper text.
 *
 *  A function rather than an entry in `GROUP_HELP` because the number is the
 *  server's: the count was written out as "Four" and stood over a panel saying
 *  no framework was offered at all, whenever the vocabulary served none. */
export function standardsHelp(frameworks: number): string {
	if (frameworks <= 0) {
		return 'Optional, and no framework is offered here yet.';
	}
	if (frameworks === 1) {
		return 'Optional. One framework.';
	}
	return `Optional. ${SPELLED[frameworks] ?? frameworks} frameworks, each searched on its own.`;
}

/** Small counts read as words in a sentence, which is the register the rest of
 *  this helper text is written in. Above ten the digit reads better than the
 *  word, and the list stops there. */
const SPELLED: Record<number, string> = {
	2: 'Two',
	3: 'Three',
	4: 'Four',
	5: 'Five',
	6: 'Six',
	7: 'Seven',
	8: 'Eight',
	9: 'Nine',
	10: 'Ten'
};

/** One alignment the seller claimed. */
export interface StandardPick {
	framework: number;
	code: string;
	statement: string;
	/** The mirror's own identifier, and what identifies a pick.
	 *
	 *  Not the code: 814 TEKS codes name more than one addressable node with a
	 *  different statement, so comparing picks by code would tie four unrelated
	 *  standards together — ticking one would tick all four and removing one
	 *  would remove them all. */
	source_guid: string;
	tpt_node_id?: number;
}

/** What the seller has typed. Every scalar is the string its control holds, so
 *  a half-typed price is representable and the refusal names it rather than
 *  the field silently reading as zero. */
export interface TptDraft {
	name: string;
	description: string;
	payload: FileHandle[];
	cover: FileHandle | null;
	previews: FileHandle[];
	videoPreview: FileHandle | null;
	/** `1` auto-generate, `2` upload now, `3` upload later. */
	thumbnailMode: string;
	thumbnails: FileHandle[];
	free: boolean;
	price: string;
	additionalLicence: string;
	bundleDiscount: string;
	/** Never defaulted: the designation is the seller's under TPT's terms. */
	taxCode: string | null;
	grades: string[];
	subjectAreas: string[];
	tags: string[];
	formats: string[];
	customCategories: string[];
	/** `data[ItemsLocalization][country_id_flag]`. The label beside it names a
	 *  country and is served, never written here, so a seller outside the one
	 *  country we have measured does not read another country's name.
	 *
	 *  A boolean rather than the sidecar's three-state option, and the two
	 *  agree: the wire's absent case is a product nobody asked, and a seller
	 *  looking at this control has been asked, so a saved form states the
	 *  checkbox's own answer whichever way it is ticked. */
	appropriateForCountry: boolean;
	standards: StandardPick[];
	teachingDuration: string | null;
	pagesOrSlides: string;
	answerKey: string | null;
	/** Never pre-selected: TPT ticks its first attestation on a blank form and
	 *  ours must not, because the statement is the seller's. */
	copyright: string | null;
	/** `0` draft, `1` live. */
	status: string;
	inventories: InventoryId[];
	/** One marketplace's own value for one field, held only where the seller
	 *  edited it away from the canonical one. Keyed `inventory:field`. */
	overrides: Record<string, string>;
}

export function emptyTptDraft(): TptDraft {
	return {
		name: '',
		description: '',
		payload: [],
		cover: null,
		previews: [],
		videoPreview: null,
		thumbnailMode: '1',
		thumbnails: [],
		free: false,
		price: '',
		additionalLicence: '',
		bundleDiscount: '',
		taxCode: null,
		grades: [],
		subjectAreas: [],
		tags: [],
		formats: [],
		customCategories: [],
		appropriateForCountry: false,
		standards: [],
		teachingDuration: null,
		pagesOrSlides: '',
		answerKey: null,
		copyright: null,
		status: '0',
		inventories: [],
		overrides: {}
	};
}

// ------------------------------------------------------------- the pickers

/** Which draft field a picker holds, so one control serves all four. */
export type PickerKey = 'grades' | 'subjectAreas' | 'tags' | 'formats';

// The labels each picker carries and which of them TPT marks required are the
// core's, not this file's: `Picker::label` and the three required pickers live
// in `tam-domain`, and a refusal arrives here already naming its control.

const CAP_FIELD: Record<PickerKey, keyof FormCaps> = {
	grades: 'grades',
	subjectAreas: 'subject_areas',
	tags: 'tags',
	formats: 'formats'
};

/** The measured limit for one picker, or `null` where none is measured.
 *
 *  `null` is unmeasured, never unlimited: the subject-area cap of three was
 *  contradicted by a create TPT accepted, so enforcing it would refuse a set
 *  the platform took. */
export function capOf(caps: FormCaps, picker: PickerKey): number | null {
	return caps[CAP_FIELD[picker]] ?? null;
}

/** The counter beside a capped picker. Reads "2 of 4" against a measured cap
 *  and "2 chosen" without one, because a counter against a number nobody holds
 *  would be an invented ceiling. */
export function counterOf(chosen: number, cap: number | null): { text: string; over: boolean } {
	if (cap === null) {
		return { text: `${chosen} chosen`, over: false };
	}
	return { text: `${chosen} of ${cap}`, over: chosen > cap };
}

/** Whether one more value can be ticked. At the cap the remaining options are
 *  disabled rather than left to be refused on submit, which is the one place
 *  this form deliberately does more than TPT's. */
export function atCap(chosen: number, cap: number | null): boolean {
	return cap !== null && chosen >= cap;
}

/** The chosen values with one added or removed, keeping the seller's own order
 *  and refusing to add past the cap. Ticking a value already held is a no-op,
 *  so a repeated change event cannot write it twice. */
export function togglePick(
	chosen: readonly string[],
	value: string,
	on: boolean,
	cap: number | null
): string[] {
	if (!on) {
		return chosen.filter((held) => held !== value);
	}
	if (chosen.includes(value) || atCap(chosen.length, cap)) {
		return [...chosen];
	}
	return [...chosen, value];
}

/** The facets matching what the seller typed, case-insensitively, over both
 *  the label and the slug. An empty query matches everything. */
export function searchFacets(facets: readonly FacetView[], query: string): FacetView[] {
	const needle = query.trim().toLowerCase();
	const offered = facets.filter((facet) => facet.seller_writable);
	if (needle.length === 0) {
		return [...offered];
	}
	return offered.filter(
		(facet) =>
			facet.label.toLowerCase().includes(needle) || facet.slug.toLowerCase().includes(needle)
	);
}

/** The grade grid as columns, which is the arrangement that makes the four
 *  bands legible: primary grades, middle grades, high-school grades, then the
 *  three non-grade bands. The sizes come from the server, so a re-polled
 *  vocabulary re-shapes the grid rather than silently overflowing one column. */
export function gradeColumns(vocabulary: FormVocabularyView): FacetView[][] {
	const writable = vocabulary.grades.filter((grade) => grade.seller_writable);
	const columns: FacetView[][] = [];
	let cursor = 0;
	for (const size of vocabulary.grade_columns) {
		columns.push(writable.slice(cursor, cursor + size));
		cursor += size;
	}
	if (cursor < writable.length) {
		columns.push(writable.slice(cursor));
	}
	return columns;
}

/** The label for one option id, or the id itself where the list holds none.
 *  Never a reading invented here. */
export function labelOf(options: readonly { id: string; label: string }[], id: string | null) {
	if (id === null) {
		return null;
	}
	return options.find((option) => option.id === id)?.label ?? id;
}

// --------------------------------------------------------------- the price

/** A typed amount as minor units, or `null` where it is not a number this
 *  client will send. A fractional cent is refused rather than rounded: the
 *  seller typed a price and a silently altered one is a different price. */
export function minorUnitsOf(amount: string): number | null {
	const trimmed = amount.trim();
	if (trimmed.length === 0 || !/^\d+(\.\d{1,2})?$/.test(trimmed)) {
		return null;
	}
	return Math.round(Number(trimmed) * 100);
}

export function majorUnitsOf(minorUnits: number): string {
	return (minorUnits / 100).toFixed(2);
}

/** The Multiple Licenses pre-fill: the percentage the vocabulary states, of
 *  the price the seller typed, rounded down to the cent.
 *
 *  A pre-fill and nothing else. TPT's help centre says the seller may choose
 *  any discount, so this seeds an empty field once and never overwrites a
 *  figure the seller has entered — a projection that recomputed it would
 *  replace their own number on every sync. */
export function suggestedAdditionalLicence(price: string, percentage: number): string {
	const minor = minorUnitsOf(price);
	if (minor === null) {
		return '';
	}
	return majorUnitsOf(Math.floor((minor * percentage) / 100));
}

// ------------------------------------------------------------ the refusals

export interface Refusal {
	group: FormGroup;
	control: string | null;
	message: string;
}

/** Everything this form refuses, in the order the groups read.
 *
 *  Decided by the compiled core, which is the same function
 *  `POST /v1/authoring/check` calls, so the message a seller reads as they type
 *  is the server's answer rather than a second implementation of it. What used
 *  to be reproduced here is gone; only the one rule below is added, and it is
 *  added because the domain does not hold it.
 *
 *  `vocabulary` is still taken because it is what the page has and what the
 *  caller passes; the caps it carries are no longer read here, since the module
 *  holds the same numbers from the same capture. */
export function refusalsOf(draft: TptDraft, vocabulary: FormVocabularyView | null): Refusal[] {
	void vocabulary;
	const found: Refusal[] = [];
	const rules = core();
	if (rules === null) {
		// Fail closed. An unloaded module has not decided that there is nothing
		// to refuse, and treating it as though it had would let a blank form
		// submit in the moment before the rules arrive.
		found.push({
			group: 'name',
			control: null,
			message: 'The form’s rules are still loading; nothing can be submitted yet.'
		});
	} else {
		for (const refusal of rules.checkDraft(draftInputOf(draft)).refusals) {
			found.push({
				group: refusal.group,
				control: refusal.control ?? null,
				message: refusal.message
			});
		}
	}
	// The one rule the core does not hold, and the reason it does not: the
	// domain describes a product, and which marketplaces to publish it to is a
	// decision about this listing rather than a property of the product.
	if (draft.inventories.length === 0) {
		found.push({
			group: 'product_status',
			control: null,
			message: 'Choose at least one marketplace to create this draft on.'
		});
	}
	return found;
}

/** Something worth saying that blocks nothing. */
export interface Advisory {
	group: FormGroup;
	message: string;
}

/** The advisories the core raises. Nothing is added here: every one of them is
 *  guidance the model already states. */
export function advisoriesOf(draft: TptDraft, vocabulary: FormVocabularyView | null): Advisory[] {
	void vocabulary;
	const rules = core();
	if (rules === null) {
		return [];
	}
	return rules.checkDraft(draftInputOf(draft)).advisories.map((advisory) => ({
		group: advisory.group,
		message: advisory.message
	}));
}

export function submittable(refusals: readonly Refusal[]): boolean {
	return refusals.length === 0;
}

/** The refusals belonging to one group, so a section renders its own. */
export function refusalsIn(refusals: readonly Refusal[], group: FormGroup): Refusal[] {
	return refusals.filter((refusal) => refusal.group === group);
}

// ------------------------------------------------------- per-marketplace

/** The key one marketplace's own value for one field is held under. */
export function overrideKey(inventory: InventoryId, field: string): string {
	return `${inventory}:${field}`;
}

/** The fields a marketplace tab lets a seller diverge on. Deliberately the
 *  three every target carries: a field one platform lacks is a disclosed loss
 *  rather than an override. */
export const OVERRIDABLE: readonly { field: string; label: string }[] = [
	{ field: 'name', label: 'Title' },
	{ field: 'description', label: 'Description' },
	{ field: 'price', label: 'Price' }
];

/** What this marketplace will carry for one field: its own value where the
 *  seller set one, and the canonical value otherwise. */
export function valueFor(draft: TptDraft, inventory: InventoryId, field: string): string {
	const own = draft.overrides[overrideKey(inventory, field)];
	return own ?? canonicalValue(draft, field);
}

export function canonicalValue(draft: TptDraft, field: string): string {
	switch (field) {
		case 'name':
			return draft.name;
		case 'description':
			return draft.description;
		case 'price':
			return draft.free ? 'Free' : draft.price;
		default:
			return '';
	}
}

/** Whether this marketplace's value has been edited away from the canonical
 *  one. The Vendoo affordance hangs off exactly this: once a marketplace value
 *  differs, "Update all" appears on the shared form and "Reset" on the
 *  marketplace one, and neither fires on its own. */
export function diverges(draft: TptDraft, inventory: InventoryId, field: string): boolean {
	const own = draft.overrides[overrideKey(inventory, field)];
	return own !== undefined && own !== canonicalValue(draft, field);
}

/** Every marketplace whose value for this field differs from the canonical
 *  one, which is what decides whether "Update all" is offered at all. */
export function divergentOn(draft: TptDraft, field: string): InventoryId[] {
	return draft.inventories.filter((inventory) => diverges(draft, inventory, field));
}

/** Why one marketplace carries the value it carries.
 *
 *  Four provenances, of which the client can produce two today. `listing` and
 *  `listing_override` are this listing's own doing and are decided here.
 *  `relation` and `override` are the projection's, and the second is the
 *  per-organisation `ProjectionOverride` slice designed at the end of
 *  `docs/notes/mapping/tpt-base-residue.md`, whose `ResolvedBy` is exactly
 *  this discrimination: the crosswalk decided it, or a standing decision of
 *  yours did, with the instant it was taken.
 *
 *  The union is written now so that the tab renders a server-supplied row
 *  without a rewrite: when the projection endpoint lands it returns
 *  `MarketplaceProjection` and this file stops constructing one. */
export type DecidedBy =
	| { by: 'listing' }
	| { by: 'listing_override' }
	| { by: 'relation' }
	| { by: 'override'; decided_at: string };

/** One row of a marketplace tab: what this platform will carry for one
 *  canonical field or one equivalence axis, and what decided it.
 *
 *  `values` is a list because an axis resolves to a set — a term can broaden
 *  onto several of a platform's own values, and an override resolving to an
 *  empty set means "drop this term for me", which is a different thing from a
 *  term nobody has decided. A scalar field carries exactly one. */
export interface ProjectedRow {
	/** A canonical field name (`name`) or an equivalence axis (`subject`). */
	key: string;
	kind: 'field' | 'axis';
	label: string;
	values: string[];
	/** Null where nothing has decided this row yet, which is every axis row
	 *  today: the four cases above are the listing's doing or the projection's,
	 *  and an axis no projection has run over is neither. */
	decided_by: DecidedBy | null;
	/** What this platform drops whatever the seller picks, where the
	 *  projection says so. Null is "nothing recorded", never "nothing lost". */
	loss: string | null;
	/** Present on an axis row and absent on a field row. */
	axis?: AxisFacts;
}

/** How one axis will be settled on this marketplace.
 *
 *  Three modes, of which this client produces two. `seller_decides` is an axis
 *  no computation may ever answer, which the registry states as
 *  `Delegation::Never`; licence is the exemplar and the only one today.
 *  `best_fit` is an axis a seller may hand to a best-fit suggestion by opting
 *  in. `resolved` is an axis the projection settled on its own and shows
 *  values for, and no client can produce one, because the equivalence relation
 *  lives in Postgres and a mapping invented in the browser would be one nobody
 *  recorded. */
export type AxisMode = 'resolved' | 'best_fit' | 'seller_decides';

/** What the registry says about one axis, read off the served vocabulary
 *  rather than restated here. */
export interface AxisFacts {
	mode: AxisMode;
	/** Whether a seller may ever hand this axis to a computed answer.
	 *
	 *  False on a legal-content axis and not negotiable there. The domain
	 *  constructor refuses it, a database CHECK refuses it, and this row is the
	 *  third layer: a tab that offered an override here would undo both. */
	delegable: boolean;
	/** The platform field this axis lands in, which is what a seller would
	 *  recognise on the other site. */
	native: string;
	/** The listing's own terms for this axis, where the draft holds a set for
	 *  one. What the seller chose, never what the platform will carry. */
	stated: string[];
	/** How many terms this platform takes. Null is unmeasured, never
	 *  unlimited, so an absent cap discloses no loss. */
	cap: number | null;
}

export interface MarketplaceProjection {
	inventory: InventoryId;
	rows: ProjectedRow[];
}

/** One marketplace's tab, built from what this client holds, what the core
 *  decides and what the served vocabulary declares.
 *
 *  Field rows carry this listing's values — its own overrides where the seller
 *  set one — and the losses are the core's, read from the compiled-in field
 *  registry: a cap one platform declares and this listing's value exceeds is a
 *  real disclosed loss.
 *
 *  Axis rows carry no value, and the emptiness is the honest answer rather than
 *  a gap: the equivalence relation lives in Postgres, so a client that resolved
 *  an axis would be inventing a mapping nobody recorded. What they do carry is
 *  everything the registry already states — how the axis is settled, whether it
 *  may ever be delegated, the field it lands in, the seller's own terms and the
 *  cap those terms may exceed. When the projection endpoint lands it fills
 *  `values` and `decided_by` on these same rows rather than replacing them.
 *
 *  `vocabulary` null is a tab whose vocabulary has not arrived, which renders
 *  field rows and no axis rows rather than guessing at the axes. */
export function projectionOf(
	draft: TptDraft,
	inventory: InventoryId,
	vocabulary: VocabularyView | null = null
): MarketplaceProjection {
	const rules = core();
	const preview = rules === null ? null : rules.projectPreview(draftInputOf(draft), inventory);
	const declared = new Map((preview?.rows ?? []).map((row) => [row.key, row.loss] as const));
	const fields: ProjectedRow[] = OVERRIDABLE.map((entry) => ({
		key: entry.field,
		kind: 'field' as const,
		label: entry.label,
		values: [valueFor(draft, inventory, entry.field)],
		decided_by: diverges(draft, inventory, entry.field)
			? ({ by: 'listing_override' } as const)
			: ({ by: 'listing' } as const),
		// Null is "nothing recorded", never "nothing lost": a registry that
		// declares no cap for this field has not measured one.
		loss: declared.get(canonicalKey(entry.field)) ?? null
	}));
	// Beside the canonical fields rather than in a line of its own: a seller
	// reading this tab is already reading what this platform will do with each
	// part of the listing, and a separate notice is the thing that gets
	// scrolled past.
	const standards: ProjectedRow[] =
		carriesStandards(inventory) && draft.standards.length > 0
			? [
					{
						key: 'standards',
						kind: 'field' as const,
						label: 'Standards',
						values: draft.standards.map((pick) => pick.code),
						decided_by: { by: 'listing' } as const,
						loss: standardsLoss(draft.standards)
					}
				]
			: [];
	return {
		inventory,
		rows: [
			...fields,
			...standards,
			...axisRowsOf(draft, preview?.undecided_axes ?? [], vocabulary)
		]
	};
}

/** The words a seller reads for one axis. Presentation, and the only part of
 *  an axis row this file decides: the registry carries no label for an axis. */
const AXIS_LABELS: Record<TermKind, string> = {
	subject: 'Subject',
	topic: 'Topic',
	resource_type: 'Resource type',
	phase: 'Grade level',
	licence: 'Licence'
};

/** Which of the draft's own sets states an axis.
 *
 *  Two, and the omissions are the point. Tags, formats and the seller's own
 *  shelves reach `native_residue` with no axis claimed for them, because
 *  claiming one would be inventing a reading the registry does not hold. */
const AXIS_TERMS: Partial<Record<TermKind, 'grades' | 'subjectAreas'>> = {
	phase: 'grades',
	subject: 'subjectAreas'
};

/** One row per axis the core could not decide, joined to what the served
 *  vocabulary declares about it. An axis the core names and the vocabulary does
 *  not describe is skipped rather than rendered half-known. */
function axisRowsOf(
	draft: TptDraft,
	undecided: readonly string[],
	vocabulary: VocabularyView | null
): ProjectedRow[] {
	if (vocabulary === null) {
		return [];
	}
	const declared = new Map(vocabulary.axes.map((view) => [view.axis as string, view] as const));
	return undecided.flatMap((token) => {
		const view = declared.get(token);
		if (view === undefined) {
			return [];
		}
		const held = AXIS_TERMS[view.axis];
		const stated = held === undefined ? [] : draft[held];
		// A one-valued axis is a cap of one. Reading it that way rather than
		// as a separate case is what stops a set silently arriving as its
		// first element.
		const cap = view.cardinality === 'one' ? 1 : (view.cap ?? null);
		const delegable = view.delegation.kind !== 'never';
		return [
			{
				key: view.axis as string,
				kind: 'axis' as const,
				label: AXIS_LABELS[view.axis],
				// Never narrowed to fit the cap: a set cut to length is a
				// different listing from the one the seller described, and the
				// loss below is how they learn it before the write.
				values: [],
				decided_by: null,
				loss: capLoss(cap, stated.length),
				axis: {
					mode: delegable ? ('best_fit' as const) : ('seller_decides' as const),
					delegable,
					native: view.native,
					stated,
					cap
				}
			}
		];
	});
}

/** Whether this marketplace carries standards at all.
 *
 *  TPT alone. The whole standards stream exists because TPT posts
 *  `common_core_standard_id` parts, and no Tes or Etsy field takes a standard,
 *  so a standards row on another marketplace's tab would disclose a loss that
 *  is not one. Read through `MARKETPLACE_OF` rather than an inventory list, so
 *  a fourth Tes site inherits the answer without being named here. */
function carriesStandards(inventory: InventoryId): boolean {
	return MARKETPLACE_OF[inventory] === 'Tpt';
}

/** What this marketplace will not carry of the standards a seller picked.
 *
 *  Derived from the absent node id rather than read from the engine's own
 *  record: the server withholds `tpt_node_id` exactly when a standard cannot be
 *  posted, either because no crawl has bound it or because no current capture
 *  vouches for the binding it has. The two reasons are indistinguishable from
 *  here and the seller's situation is the same in both — the tag stays in their
 *  own catalogue and the listing does not carry it — so the sentence states the
 *  outcome and does not guess at the cause.
 *
 *  `StandardsProjection` and `NotCarried` in
 *  `crates/tam-marketplace-tpt/src/standards.rs` are the record proper, and
 *  nothing computes them yet. When something does, this derivation is replaced
 *  without what a seller reads changing. */
export function standardsLoss(picks: readonly StandardPick[]): string | null {
	const dropped = picks.filter((pick) => pick.tpt_node_id === undefined);
	if (dropped.length === 0) {
		return null;
	}
	const codes = dropped.map((pick) => pick.code).join(', ');
	const which = dropped.length === 1 ? 'it' : 'they';
	return `${dropped.length} of ${picks.length} will not reach this platform: ${codes}. ${
		dropped.length === 1 ? 'It stays' : 'They stay'
	} in your own catalogue, and ${which} can be sent once we have confirmed how this platform names ${
		dropped.length === 1 ? 'it' : 'them'
	}.`;
}

/** What this platform drops, where its declared cap and this listing's own set
 *  disagree. Null is "nothing recorded": an unmeasured cap discloses nothing,
 *  because absent is unmeasured and never unlimited. */
function capLoss(cap: number | null, chosen: number): string | null {
	if (cap === null || chosen <= cap) {
		return null;
	}
	return `This platform takes ${cap} and you have chosen ${chosen}; the rest will not reach it.`;
}

/** The registry's own name for one of this form's overridable fields. The two
 *  vocabularies agree except on the title, which the domain calls `title` and
 *  the create form calls `name`. */
function canonicalKey(field: string): string {
	return field === 'name' ? 'title' : field;
}

/** One marketplace's own value, set or cleared. Clearing is Reset: the field
 *  goes back to following the canonical one rather than being emptied. */
export function withOverride(
	draft: TptDraft,
	inventory: InventoryId,
	field: string,
	value: string | null
): TptDraft {
	const overrides = { ...draft.overrides };
	if (value === null) {
		delete overrides[overrideKey(inventory, field)];
	} else {
		overrides[overrideKey(inventory, field)] = value;
	}
	return { ...draft, overrides };
}

/** Update all: one marketplace's value becomes the canonical one, and every
 *  override of that field is dropped so nothing is left diverging silently. */
export function applyToAll(draft: TptDraft, field: string, value: string): TptDraft {
	const overrides = { ...draft.overrides };
	for (const inventory of draft.inventories) {
		delete overrides[overrideKey(inventory, field)];
	}
	const next = { ...draft, overrides };
	switch (field) {
		case 'name':
			return { ...next, name: value };
		case 'description':
			return { ...next, description: value };
		case 'price':
			return value === 'Free' ? { ...next, free: true } : { ...next, free: false, price: value };
		default:
			return next;
	}
}

// -------------------------------------------------------- the two bodies

/** The draft as `POST /v1/authoring/check` reads it. */
export function draftInputOf(draft: TptDraft): DraftInput {
	return {
		name: draft.name,
		payload_hash: draft.payload[0]?.hash ?? null,
		preview_hash: draft.previews[0]?.hash ?? null,
		video_preview_hash: draft.videoPreview?.hash ?? null,
		thumbnail_mode: Number(draft.thumbnailMode),
		thumbnail_hashes: draft.thumbnails.map((file) => file.hash),
		description: draft.description,
		free: draft.free,
		price_minor_units: draft.free ? null : minorUnitsOf(draft.price),
		additional_licence_minor_units: draft.free ? null : minorUnitsOf(draft.additionalLicence),
		bundle_discount_minor_units: draft.free ? null : minorUnitsOf(draft.bundleDiscount),
		tax_code_id: draft.free || draft.taxCode === null ? null : Number(draft.taxCode),
		grades: draft.grades,
		subject_areas: draft.subjectAreas,
		tags: draft.tags,
		formats: draft.formats,
		custom_categories: draft.customCategories,
		appropriate_for_country: draft.appropriateForCountry,
		standards: draft.standards.map((pick) => ({
			framework: pick.framework,
			code: pick.code,
			tpt_node_id: pick.tpt_node_id ?? null
		})),
		teaching_duration_id: draft.teachingDuration === null ? null : Number(draft.teachingDuration),
		pages_or_slides: pagesOf(draft),
		answer_key_id: draft.answerKey === null ? null : Number(draft.answerKey),
		copyright_declaration_id: draft.copyright === null ? null : Number(draft.copyright),
		status_user: Number(draft.status)
	};
}

function pagesOf(draft: TptDraft): number | null {
	const typed = draft.pagesOrSlides.trim();
	if (!/^\d+$/.test(typed)) {
		return null;
	}
	const pages = Number(typed);
	return pages > 0 ? pages : null;
}

export function priceIntentOf(draft: TptDraft): PriceIntent | null {
	if (draft.free) {
		return 'Free';
	}
	const minor = minorUnitsOf(draft.price);
	if (minor === null || minor <= 0) {
		return null;
	}
	// TPT sells in USD alone and offers no other denomination on its form, so
	// the currency is the marketplace's rather than a control the seller sees.
	return { Paid: { minor_units: minor, currency: 'Usd' } };
}

/** The grade declaration the catalogue stores: one verbatim TPT path per
 *  chosen slug, which is what the projection reads back. */
export function gradePathsOf(draft: TptDraft): PathInput[] {
	return draft.grades.map((slug) => ({
		inventory: 'Tpt' as InventoryId,
		kind: 'phase',
		segments: [slug],
		native_id: slug
	}));
}

/** The TPT-base block a create carries beside the fields `product` itself
 *  holds. A subset, so each field has one source rather than two copies that
 *  can disagree. */
export function tptBaseOf(draft: TptDraft): TptBaseInput {
	return {
		thumbnail_mode: Number(draft.thumbnailMode),
		thumbnail_hashes: draft.thumbnails.map((file) => file.hash),
		video_preview_hash: draft.videoPreview?.hash ?? null,
		additional_licence_minor_units: draft.free ? null : minorUnitsOf(draft.additionalLicence),
		bundle_discount_minor_units: draft.free ? null : minorUnitsOf(draft.bundleDiscount),
		tax_code_id: draft.free || draft.taxCode === null ? null : Number(draft.taxCode),
		subject_areas: draft.subjectAreas,
		tags: draft.tags,
		formats: draft.formats,
		custom_categories: draft.customCategories,
		appropriate_for_country: draft.appropriateForCountry,
		standards: draft.standards.map((pick) => ({
			framework: pick.framework,
			code: pick.code,
			tpt_node_id: pick.tpt_node_id ?? null
		})),
		teaching_duration_id: draft.teachingDuration === null ? null : Number(draft.teachingDuration),
		pages_or_slides: pagesOf(draft),
		answer_key_id: draft.answerKey === null ? null : Number(draft.answerKey),
		copyright_declaration_id: draft.copyright === null ? null : Number(draft.copyright),
		status_user: Number(draft.status)
	};
}

/** The draft as `POST /v1/products` reads it.
 *
 *  Every field the form collects travels: what `product` itself holds on the
 *  body, and everything else in the `tpt_base` block the sidecar stores. */
export function createBodyOf(draft: TptDraft): CreateProductBody | null {
	const price = priceIntentOf(draft);
	if (price === null || draft.payload.length === 0) {
		return null;
	}
	return {
		title: draft.name.trim(),
		body: draft.description,
		body_format: 'Markdown',
		price,
		payload: draft.payload,
		cover: draft.cover,
		previews: draft.previews,
		subjects: [],
		grades: gradePathsOf(draft),
		inventories: draft.inventories,
		elections: [],
		tpt_base: tptBaseOf(draft)
	};
}

/** What the form renders and does not collect, named so the page can say so
 *  rather than implying otherwise.
 *
 *  One entry. The four thumbnail slots are laid out as TPT lays them out, and
 *  attaching bytes to a named slot needs a per-slot upload `POST /v1/uploads`
 *  does not offer: it takes one file per request with no slot to name it. The
 *  thumbnail *mode* is collected and stored; the images are not. */
export const UNCOLLECTED_FIELDS: readonly string[] = [
	'the four thumbnail images, whose slots are rendered but collect no bytes yet'
];
