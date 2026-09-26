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
	FileView,
	FormCaps,
	FormVocabularyView,
	PatchProductBody,
	PathInput,
	PathView,
	PriceIntent,
	ProductView,
	TptBaseInput,
	VocabularyView
} from '$lib/api';
import type {
	CopyFormat,
	FileRole,
	FormGroup,
	InventoryId,
	Marketplace,
	TermKind
} from '$lib/generated/vocab';
import { core, loadCore } from '$lib/core';
import { licenceElections, licenceGated, rightsOf, type LicenceIntent } from '$lib/authoring';
import { MARKETPLACE_OF } from '$lib/listings-view';
import { MARKETPLACE_TILES, MARKETPLACE_WORD, platformTitle } from '$lib/platforms';

// Started at module scope so the rules are ready before the seller has typed
// anything; guarded because there is no asset to fetch while prerendering.
if (typeof window !== 'undefined') {
	void loadCore();
}

/** Where a section of the form sits, and what a refusal about it scrolls to.
 *
 *  A superset of the nine groups the domain names. Three of the form's bands
 *  are the console's own — the marketplace grid at the top, the preview file
 *  and the thumbnails — and two more are the per-marketplace panels; none of
 *  them is a group the core refuses against, so they are added here rather
 *  than asked of the generated union. Every refusal the core raises carries a
 *  `FormGroup`, which is one of these by construction. */
export type FormAnchor =
	| FormGroup
	| 'marketplaces'
	| 'preview'
	| 'thumbnails'
	| 'tpt_options'
	| 'tes_options';

/** The heading over each band, in the order the form reads. */
export const GROUP_HEADINGS: Record<FormAnchor, string> = {
	marketplaces: 'Marketplaces',
	name: 'Name',
	files: 'Files',
	preview: 'Preview',
	thumbnails: 'Thumbnails (TPT layout)',
	description: 'Description',
	price: 'Price',
	categories: 'Categories',
	education_standards: 'Education Standards',
	details: 'Details',
	copyright: 'Copyright',
	tpt_options: 'TPT only',
	tes_options: 'Tes only',
	product_status: 'Product Status'
};

/** One short sentence under a heading, telling the teacher what to do there,
 *  and nothing at all where the heading already says it. */
export const GROUP_HELP: Partial<Record<FormAnchor, string>> = {
	marketplaces: 'Choose where this goes.',
	files: 'The files buyers download.',
	preview: 'A free sample buyers can look at before they buy.',
	description: 'Tell buyers what this is and how it helps.',
	price: 'Free hides the price and tax code.',
	categories: 'Help buyers find this.',
	details: 'Optional.',
	product_status: 'Active listings show up in search; a draft only you can see.'
};

/** The AI auto-fill placement, beside the Files heading on a new resource.
 *
 * A notice, not a control: the feature is not built, so there is nothing to
 * press. The charter allows a model only to propose (`decisions.md`,
 * 2026-09-12), and the promise says exactly that and nothing else — no date,
 * no accuracy figure, and no claim about writing to a marketplace. */
export const AI_FILL_SOON = 'AI fill — coming soon';
export const AI_FILL_SOON_HINT =
	'Soon Teachouse will fill this form from your file, for you to check.';

/** The Education Standards heading's helper text, or nothing where the server
 *  serves no framework: with nothing to search the panel below says so at more
 *  length, in the place the seller is already looking. */
export const STANDARDS_HELP = 'Optional. Search a framework by code or words.';

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
	/** How `description` is written.
	 *
	 *  Carried rather than asserted at the wire. A create writes Markdown and
	 *  no control changes it, but an edit is seeded from a product that may
	 *  already be `Html` — the Tes import writes those — and a save that
	 *  relabelled it would leave the markup unchanged under a declaration that
	 *  is now wrong, which is how a listing acquires escaped markup on its next
	 *  send. The format only ever travels beside the body it describes. */
	bodyFormat: CopyFormat;
	payload: FileHandle[];
	cover: FileHandle | null;
	previews: FileHandle[];
	/** The video preview's handle, as the sidecar stores it: a bare digest.
	 *  No control writes one yet, and the read carries the hash alone, so a
	 *  whole `FileHandle` here would be a kind and a length nothing states. */
	videoPreview: string | null;
	/** `1` auto-generate, `2` upload now, `3` upload later. */
	thumbnailMode: string;
	/** One digest per filled slot, in slot order, as the sidecar stores them.
	 *  Bare hashes rather than handles for the same reason `videoPreview` is:
	 *  the read carries the hash alone, so a kind and a length here would be
	 *  values nothing states. */
	thumbnails: string[];
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
	 *  Three states, as the sidecar has: `true` and `false` are the ticked and
	 *  unticked answers of a seller who was shown the control, and `null` is a
	 *  product that was never asked. A create shows the control, so it starts
	 *  at `false` and an unticked box is an answer. An edit is seeded from a
	 *  product that may hold none, and collapsing that to `false` on the way in
	 *  would turn "not stated" into "explicitly no" on the first save, since
	 *  the sidecar is sent whole every time. */
	appropriateForCountry: boolean | null;
	standards: StandardPick[];
	teachingDuration: string | null;
	pagesOrSlides: string;
	answerKey: string | null;
	/** Never pre-selected: TPT ticks its first attestation on a blank form and
	 *  ours must not, because the statement is the seller's. */
	copyright: string | null;
	/** `0` draft, `1` live. */
	status: string;
	/** The rights grant the seller states, where a chosen marketplace gates one.
	 *
	 *  One value for the whole listing rather than one per marketplace. Never
	 *  defaulted — a rights grant is the seller's to make. */
	licence: string | null;
	/** The tiles the seller ticked. One Tes, never three, because that is what
	 *  the form shows. */
	marketplaces: Marketplace[];
	/** The answer above, resolved to what the request carries. Held on the
	 *  draft rather than recomputed at every reader because the refusals, the
	 *  rail, the licence gate and both request bodies all ask for it; every
	 *  write goes through [`withMarketplaces`], so the two cannot disagree. */
	inventories: InventoryId[];
	/** One marketplace's own value for one field, held only where the seller
	 *  edited it away from the canonical one. Keyed `inventory:field`. */
	overrides: Record<string, string>;
}

/** Which of the two things this form is doing.
 *
 *  One component renders both, because the create form and the edit form are
 *  one model: TPT's own create and edit post near-identical bodies, and a
 *  second component would be a second set of seventeen controls to keep in
 *  step. The mode decides the seed, the request the submit makes, and whether
 *  a marketplace tick can be taken back — nothing else. */
export type FormMode =
	| { kind: 'create' }
	| {
			kind: 'edit';
			product: ProductView;
			/** The marketplaces this resource already reaches. Add-only: no
			 *  route unmaps one, so these render ticked and disabled. */
			mapped: readonly InventoryId[];
			/** The live listings that make this whole form read-only, or empty.
			 *  A published listing on a platform whose edit transition we have
			 *  not captured cannot be edited through us, and the server refuses
			 *  the request, so the fields are shown as stored and held back. */
			blockedBy: readonly InventoryId[];
	  };

export function emptyTptDraft(): TptDraft {
	return {
		name: '',
		description: '',
		bodyFormat: 'Markdown',
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
		licence: null,
		marketplaces: [],
		inventories: [],
		overrides: {}
	};
}

// ------------------------------------------------------- the marketplaces

/** What the request carries, from the one answer the form asks for.
 *
 *  One tile is one marketplace is one inventory (`decisions.md`, 2026-09-12),
 *  so ticking a tile is the whole answer to where a listing goes.
 *
 *  Ordered by the tile grid rather than by the order the seller ticked, so two
 *  identical listings compose one identical body. */
export function inventoriesOf(marketplaces: readonly Marketplace[]): InventoryId[] {
	return MARKETPLACE_TILES.filter(
		(tile) => tile.authorable && marketplaces.includes(tile.marketplace)
	).map((tile) => tile.inventory);
}

/** The draft with its marketplace answer replaced and the request's own list
 *  resolved from it, which is the only way those two fields are written. */
export function withMarketplaces(draft: TptDraft, marketplaces: readonly Marketplace[]): TptDraft {
	return {
		...draft,
		marketplaces: [...marketplaces],
		inventories: inventoriesOf(marketplaces)
	};
}

/** The tiles a stored listing's inventories stand for: the inverse of
 *  [`inventoriesOf`], so an edit form opens with the tiles the listing's own
 *  mappings imply. */
export function tilesOf(inventories: readonly InventoryId[]): { marketplaces: Marketplace[] } {
	return {
		marketplaces: MARKETPLACE_TILES.filter((tile) =>
			inventories.includes(tile.inventory)
		).map((tile) => tile.marketplace)
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

/** Which words the grade grid is written in.
 *
 *  One selection underneath either way: the teacher picks in the system they
 *  teach in and Teachouse writes the other one for the marketplaces that
 *  expect it, which is the founder's rule of 2026-09-11. */
export type GradeLabels = 'american' | 'british';

/** Where the choice is remembered, so a teacher who works in years is not
 *  asked to switch on every listing. */
export const GRADE_LABELS_KEY = 'teachouse.grade-labels';

/** One grade in the words the teacher chose, falling back to the American
 *  label where no British one is declared.
 *
 *  The fall-back is the crosswalk's own shape rather than a defensive default:
 *  Preschool, Higher Education, Adult Education and Not Grade Specific have no
 *  year group, and inventing one would be the inference the founder's table
 *  exists to avoid. */
export function gradeLabel(facet: FacetView, labels: GradeLabels): string {
	if (labels === 'american') {
		return facet.label;
	}
	return facet.british_label ?? facet.label;
}

/** The heading over one column of the grade grid, in the chosen words, or the
 *  empty string where the server headed no such column. */
export function gradeBandLabel(
	vocabulary: FormVocabularyView,
	column: number,
	labels: GradeLabels
): string {
	const band = vocabulary.grade_bands[column];
	if (band === undefined) {
		return '';
	}
	return labels === 'american' ? band.american : band.british;
}

/** The label for one option id, or the id itself where the list holds none.
 *  Never a reading invented here. */
export function labelOf(options: readonly { id: string; label: string }[], id: string | null) {
	if (id === null) {
		return null;
	}
	return options.find((option) => option.id === id)?.label ?? id;
}

// --------------------------------------------------------- the description

/** What one button on the description toolbar does. */
export type MarkKind = 'bold' | 'italic' | 'bullets' | 'numbers';

/** The description after a toolbar button, and where the caret goes.
 *
 *  The caret matters as much as the text: a teacher who presses Bold with
 *  nothing selected expects to type between the two pairs of asterisks rather
 *  than after them, and one who bolded a phrase expects that phrase still
 *  selected so a second press is undoable by eye. */
export interface MarkedUp {
	text: string;
	start: number;
	end: number;
}

const WRAP: Record<'bold' | 'italic', string> = { bold: '**', italic: '*' };

/** The description with Markdown written around what the teacher selected.
 *
 *  Markdown rather than the marketplace's own markup: the body already travels
 *  as `CopyFormat::Markdown` and each adapter renders it in its own way, so a
 *  toolbar that wrote a platform's HTML would write it for every platform. */
export function markUp(text: string, start: number, end: number, kind: MarkKind): MarkedUp {
	if (kind === 'bold' || kind === 'italic') {
		const mark = WRAP[kind];
		const selected = text.slice(start, end);
		return {
			text: `${text.slice(0, start)}${mark}${selected}${mark}${text.slice(end)}`,
			start: start + mark.length,
			end: end + mark.length
		};
	}
	// A list is written per line, and the lines are the whole lines the
	// selection touches: prefixing from the middle of a line would put the
	// bullet inside a sentence.
	const from = text.lastIndexOf('\n', Math.max(start - 1, 0)) + 1;
	const to = text.indexOf('\n', end) === -1 ? text.length : text.indexOf('\n', end);
	const lines = text.slice(from, to).split('\n');
	const marked = lines
		.map((line, index) => `${kind === 'bullets' ? '- ' : `${index + 1}. `}${line}`)
		.join('\n');
	return {
		text: `${text.slice(0, from)}${marked}${text.slice(to)}`,
		start: from,
		end: from + marked.length
	};
}

/** A byte limit in the unit a teacher would say it in. Shared by the three
 *  places that state one, so the file section and a thumbnail slot cannot
 *  round the same number two ways. */
export function sizeWords(bytes: number): string {
	if (bytes >= 1024 ** 3) {
		return `${Math.round(bytes / 1024 ** 3)} GB`;
	}
	return `${Math.round(bytes / 1024 ** 2)} MB`;
}

/** One file the teacher added, and the handles one upload landed it as.
 *
 *  A ZIP uploaded to be unpacked becomes several payload handles under one
 *  name, so the row a teacher sees is the file they chose rather than the
 *  handles it became, and removing it removes all of them. `cover` is the
 *  thumbnail that upload drew; only the first file's is used, because the
 *  thumbnail is drawn from the file buyers see first. */
export interface StoredFile {
	name: string;
	bytes: number;
	payload: FileHandle[];
	cover: FileHandle;
	storedBytes: number;
	storageBytesMax: number;
}

/** Every handle the added files came to, in order, with the first file's
 *  thumbnail as the listing's cover. */
export function payloadOf(files: readonly StoredFile[]): {
	payload: FileHandle[];
	cover: FileHandle | null;
} {
	return {
		payload: files.flatMap((file) => file.payload),
		cover: files[0]?.cover ?? null
	};
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
	group: FormAnchor;
	control: string | null;
	message: string;
}

/** Which band of this form a refusal scrolls to.
 *
 *  The core answers in the domain's own nine groups, and this form no longer
 *  draws two of them where the domain puts them: the copyright attestation and
 *  the tax code are questions only TPT asks, so they live in the TPT-only
 *  panel and a refusal about either has to land there rather than on a
 *  Copyright heading that is not on the page or in the middle of Price. Every
 *  other group is its own band and is left alone. */
export function anchorOf(group: FormGroup, control: string | null): FormAnchor {
	if (group === 'copyright' || control === 'Tax Code') {
		return 'tpt_options';
	}
	return group;
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
export function refusalsOf(
	draft: TptDraft,
	vocabulary: FormVocabularyView | null,
	known: ReadonlyMap<InventoryId, VocabularyView> = new Map()
): Refusal[] {
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
			message: 'The form is still loading. Wait a moment, then try again.'
		});
	} else {
		for (const refusal of rules.checkDraft(draftInputOf(draft)).refusals) {
			found.push({
				group: anchorOf(refusal.group, refusal.control ?? null),
				control: refusal.control ?? null,
				message: refusal.message
			});
		}
	}
	// The rules the core does not hold, and the reason it does not: the domain
	// describes a product, and which marketplaces to publish it to is a
	// decision about this listing rather than a property of the product.
	//
	// A resource with no marketplace is a draft kept here, which is a thing a
	// seller is allowed to want, so no marketplace is no longer refused. What
	// is refused is a marketplace chosen without a file, because that is the
	// combination the marketplace itself will not take.
	if (needsFileBeforeMarketplace(draft)) {
		found.push({
			group: 'marketplaces',
			control: null,
			message: 'Add your file before you choose a marketplace.'
		});
	}
	// Refused rather than left to the server, because the server refuses it
	// either way: `required_fields_answered` in `crates/tam-api/src/catalogue.rs`
	// rejects a create naming a marketplace whose registry declares a required
	// field it cannot see answered. Sending it and reading the refusal back was
	// how every Tes create from this form failed.
	for (const marketplace of new Set(
		unlicensed(draft, known).map((inventory) => MARKETPLACE_OF[inventory])
	)) {
		found.push({
			group: marketplace === 'Tpt' ? 'tpt_options' : 'tes_options',
			control: 'Licence',
			message: `Choose a licence. ${MARKETPLACE_WORD[marketplace]} needs one to list this.`
		});
	}
	return found;
}

/** Every chosen marketplace that gates a licence and has not been given one. */
export function unlicensed(
	draft: TptDraft,
	known: ReadonlyMap<InventoryId, VocabularyView>
): InventoryId[] {
	if (draft.licence !== null && draft.licence.length > 0) {
		return [];
	}
	return licenceGated(draft.inventories, known);
}

/** The pricing branch this listing's licence is gated on. Tes refuses a
 *  Creative Commons licence with a price and refuses `TES-PAID` without one,
 *  so the free tick decides which values are offered. */
export function licenceIntentOf(draft: TptDraft): LicenceIntent {
	return {
		licence: draft.licence,
		inventories: draft.inventories,
		branch: draft.free ? 'free' : 'paid'
	};
}

/** Whether the seller has asked for a marketplace listing without the file it
 *  would carry, which is what opens the warning on this form.
 *
 *  A function rather than a condition written into the markup, so one
 *  assertion holds it: a rule reachable only by rendering the page is a rule
 *  nothing cheap can check. */
export function needsFileBeforeMarketplace(draft: TptDraft): boolean {
	return draft.inventories.length > 0 && draft.payload.length === 0;
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
export function refusalsIn(refusals: readonly Refusal[], group: FormAnchor): Refusal[] {
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
	return `${dropped.length} of ${picks.length} will not go to this marketplace: ${codes}. ${
		dropped.length === 1 ? 'It stays' : 'They stay'
	} in your Resources, and ${which} can be sent once we know how this marketplace names ${
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
	return `This marketplace takes ${cap} and you chose ${chosen}; the rest will not go to it.`;
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
		// D32: the file becomes necessary only where a marketplace is named, and
		// the core cannot know the destination, so the form states it.
		for_marketplace: draft.inventories.length > 0,
		payload_hash: draft.payload[0]?.hash ?? null,
		preview_hash: draft.previews[0]?.hash ?? null,
		video_preview_hash: draft.videoPreview,
		thumbnail_mode: Number(draft.thumbnailMode),
		thumbnail_hashes: draft.thumbnails,
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
		thumbnail_hashes: draft.thumbnails,
		video_preview_hash: draft.videoPreview,
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
 *  body, and everything else in the `tpt_base` block the sidecar stores.
 *
 *  `known` is what each chosen marketplace declares, and it is what turns the
 *  seller's licence into the two shapes the server reads it in — the product's
 *  own rights declaration and one already-answered election per gating
 *  marketplace. Both are composed by `$lib/authoring`, which held the tested
 *  answer while this function sent `elections: []` and no `rights` at all: the
 *  server's `required_fields_answered` then refused every Tes create, and no
 *  Tes listing could be made from this form. Passing no map composes neither,
 *  which is correct for a listing that names no marketplace. */
export function createBodyOf(
	draft: TptDraft,
	known: ReadonlyMap<InventoryId, VocabularyView> = new Map()
): CreateProductBody | null {
	const price = priceIntentOf(draft);
	// No file is a resource kept here (D32), so only an unsendable price stops
	// the body being composed. The marketplace case is a refusal the form
	// already carries, not a body it declines to build.
	if (price === null) {
		return null;
	}
	const intent = licenceIntentOf(draft);
	return {
		title: draft.name.trim(),
		body: draft.description,
		body_format: draft.bodyFormat,
		price,
		payload: draft.payload,
		cover: draft.cover,
		previews: draft.previews,
		subjects: [],
		grades: gradePathsOf(draft),
		rights: rightsOf(intent, known),
		inventories: draft.inventories,
		elections: licenceElections(intent, known),
		tpt_base: tptBaseOf(draft)
	};
}

/** The draft as `PATCH /v1/products/{product}` reads it, or `null` where the
 *  price is one this client will not send.
 *
 *  Three fields the create carries are deliberately absent, and each for the
 *  same reason: on this route an absent field is left as stored, so sending
 *  one the form does not collect would clear it. `payload`, `cover` and
 *  `previews` belong to the file sub-resource. `subjects` is the canonical
 *  taxonomy an import writes and this form has no control for, so the create's
 *  own empty list would wipe it here. `inventories` and `elections` have no
 *  place on this body at all: a marketplace is added through its own route,
 *  and edit mode never removes one.
 *
 *  `tpt_base` travels whole because the server replaces the row rather than
 *  merging it, which is what makes clearing a control expressible. */
export function patchBodyOf(
	draft: TptDraft,
	known: ReadonlyMap<InventoryId, VocabularyView> = new Map()
): PatchProductBody | null {
	const price = priceIntentOf(draft);
	if (price === null) {
		return null;
	}
	const rights = rightsOf(licenceIntentOf(draft), known);
	return {
		title: draft.name.trim(),
		body: draft.description,
		// The format the product already carries, not a constant. Always beside
		// the body it describes, because the server refuses a format on its own
		// — and a format that changed without its text is the same fault seen
		// from the other side: an `Html` body relabelled Markdown is unchanged
		// markup under a declaration that is now wrong, which the next send
		// renders as escaped markup.
		body_format: draft.bodyFormat,
		price,
		grades: gradePathsOf(draft),
		...(rights === null ? {} : { rights }),
		tpt_base: tptBaseOf(draft)
	};
}

/** The stored product as this form's own state: the inverse of
 *  [`createBodyOf`] and [`tptBaseOf`] together.
 *
 *  A product with no sidecar row keeps [`emptyTptDraft`]'s values for the
 *  seventeen fields it holds, and in particular leaves the copyright
 *  attestation and the tax code unstated: neither is ours to invent, and a
 *  form that arrived with one pre-selected would make the statement ours
 *  rather than the seller's (D7).
 *
 *  `licence` and `inventories` come from the product and its mappings rather
 *  than from the sidecar, which holds neither. */
export function draftOf(
	product: ProductView,
	inventories: readonly InventoryId[] = []
): TptDraft {
	const base = product.tpt_base;
	const files = (role: FileRole) => product.files.filter((file) => file.role === role);
	const empty = emptyTptDraft();
	const paid = paidMinorUnits(product.price);
	return {
		...empty,
		name: product.title,
		description: product.body,
		bodyFormat: product.body_format,
		payload: files('payload').map(handleOf),
		cover: files('cover').map(handleOf)[0] ?? null,
		previews: files('preview').map(handleOf),
		videoPreview: base?.video_preview_hash ?? null,
		thumbnailMode: base === undefined ? empty.thumbnailMode : String(base.thumbnail_mode),
		thumbnails: base?.thumbnail_hashes ?? [],
		free: paid === null,
		price: paid === null ? '' : majorUnitsOf(paid),
		additionalLicence: optionalMajorUnits(base?.additional_licence_minor_units),
		bundleDiscount: optionalMajorUnits(base?.bundle_discount_minor_units),
		taxCode: optionalId(base?.tax_code_id),
		grades: product.grades.raw.map(pathSlug),
		subjectAreas: base?.subject_areas ?? [],
		tags: base?.tags ?? [],
		formats: base?.formats ?? [],
		customCategories: base?.custom_categories ?? [],
		// `?? null` rather than `?? false`: a product with no sidecar and a
		// sidecar that answered nothing are both unanswered, and neither is a
		// seller saying no.
		appropriateForCountry: base?.appropriate_for_country ?? null,
		standards: (base?.standards ?? []).map(storedPick),
		teachingDuration: optionalId(base?.teaching_duration_id),
		pagesOrSlides: base?.pages_or_slides === null || base === undefined ? '' : String(base.pages_or_slides),
		answerKey: optionalId(base?.answer_key_id),
		copyright: optionalId(base?.copyright_declaration_id),
		status: base === undefined ? empty.status : String(base.status_user),
		licence: product.rights?.native_id ?? product.rights?.segments[0] ?? null,
		...tilesOf(inventories),
		inventories: [...inventories]
	};
}

/** Everything the draft carries about a file, dropped.
 *
 *  The bytes are untouched: they reached `POST /v1/uploads` when the file was
 *  chosen and they stay in the organisation's storage either way. This drops
 *  the handles so the create does not carry them, and nothing else. */
export function withoutPayload(draft: TptDraft): TptDraft {
	return { ...draft, payload: [], cover: null, previews: [] };
}

function handleOf(file: FileView): FileHandle {
	return { hash: file.hash, kind: file.kind, byte_len: file.byte_len, name: file.name };
}

/** The stored price in minor units, or `null` for a free listing and for a
 *  shape this client cannot read. Both read back as free, because guessing an
 *  amount is worse than showing none. */
function paidMinorUnits(price: unknown): number | null {
	if (typeof price !== 'object' || price === null) {
		return null;
	}
	const paid = (price as { Paid?: unknown }).Paid;
	if (typeof paid !== 'object' || paid === null) {
		return null;
	}
	const minor = (paid as { minor_units?: unknown }).minor_units;
	return typeof minor === 'number' ? minor : null;
}

function optionalMajorUnits(minorUnits: number | null | undefined): string {
	return minorUnits === null || minorUnits === undefined ? '' : majorUnitsOf(minorUnits);
}

function optionalId(id: number | null | undefined): string | null {
	return id === null || id === undefined ? null : String(id);
}

function pathSlug(path: PathView): string {
	return path.native_id ?? path.segments[0] ?? '';
}

/** One stored alignment as a pick.
 *
 *  The sidecar holds the framework, the code and TPT's node id, and neither
 *  the statement the mirror published nor the mirror's own identifier. So the
 *  identifier is synthesised from what is stored and marked as such: the chips
 *  render the code and remove by this key, which works, and a seller who finds
 *  the same standard again in the search adds a second pick rather than seeing
 *  the stored one already ticked. Storing the mirror's guid would fix that and
 *  is a migration, so it is stated here rather than papered over.
 *
 *  Exported because a saved template's draft carries the same three fields
 *  the sidecar does, so reading one back into the form is this conversion. */
export function storedPick(alignment: {
	framework: number;
	code: string;
	tpt_node_id: number | null;
}): StandardPick {
	return {
		framework: alignment.framework,
		code: alignment.code,
		statement: '',
		source_guid: `stored:${alignment.framework}:${alignment.code}:${alignment.tpt_node_id ?? ''}`,
		...(alignment.tpt_node_id === null ? {} : { tpt_node_id: alignment.tpt_node_id })
	};
}

/** One thumbnail slot's state, in the order TPT lays the four out.
 *
 *  `local` is where the picture can be drawn from: the browser's own object
 *  URL for bytes the seller has just chosen, held from the instant they choose
 *  so the picture appears before the upload finishes, or the stored blob's own
 *  route for a slot seeded from a saved listing. `handle` is the digest the
 *  server answered with, and only a slot that has one travels. The two are
 *  separate because the gap between them is exactly the interval the seller is
 *  told about. */
export interface ThumbnailSlot {
	local: string | null;
	handle: string | null;
	sending: boolean;
	refusal: string | null;
}

export function emptySlots(): ThumbnailSlot[] {
	return [0, 1, 2, 3].map(() => ({
		local: null,
		handle: null,
		sending: false,
		refusal: null
	}));
}

/** The four slots as a saved listing left them: one filled per stored digest,
 *  in the order the sidecar holds them, and the rest empty.
 *
 *  A stored slot draws from `GET /{version}/uploads/{handle}`, which serves any
 *  blob this organisation sealed and therefore needs no route of its own. That
 *  route answers PNG, JPEG, GIF and WebP, each decided from the bytes' own
 *  signature by `image_type` in `crates/tam-api/src/resources.rs`, and refuses
 *  anything else, so a slot holding some other format shows as stored without
 *  drawing; the digest still travels, so an edit that does not touch the slot
 *  does not drop the thumbnail. */
export function slotsFrom(hashes: readonly string[]): ThumbnailSlot[] {
	return emptySlots().map((slot, index) => {
		const hash = hashes[index];
		return hash === undefined
			? slot
			: { local: `/v1/uploads/${hash}`, handle: hash, sending: false, refusal: null };
	});
}

/** The digests the create carries, in slot order and skipping the empty ones.
 *
 *  Skipping rather than padding: the sidecar stores a list and a slot nobody
 *  filled is not a thumbnail, so a placeholder would be a hash standing for no
 *  bytes — which is the thing `create_product`'s own check refuses. */
export function thumbnailHashes(slots: readonly ThumbnailSlot[]): string[] {
	return slots.flatMap((slot) => (slot.handle === null ? [] : [slot.handle]));
}

/** Whether any slot is still being sent, which is what the form waits on
 *  before it will submit: a create composed mid-upload would carry fewer
 *  thumbnails than the seller chose and say nothing about it. */
export function slotsSettling(slots: readonly ThumbnailSlot[]): boolean {
	return slots.some((slot) => slot.sending);
}

/** What the seller is told once the listing exists.
 *
 *  Counted in marketplaces rather than in mappings: Tes is one tile and three
 *  catalogues, so a listing the teacher sent to TPT and Tes writes four
 *  mappings and "four marketplaces" would be a number they never chose.
 *
 *  Three arms, and the zero one is the reason this is a function rather than a
 *  ternary in the handler: an empty list fell through to the plural and read
 *  "on 0 marketplaces", which miscounts and then tells a seller to publish
 *  something they deliberately kept here. */
export function createdToast(marketplaces: number): string {
	if (marketplaces === 0) {
		return 'Listing created.';
	}
	if (marketplaces === 1) {
		return 'Listing created on one marketplace.';
	}
	return `Listing created on ${marketplaces} marketplaces.`;
}

/** How many marketplaces a set of inventories reaches, which is the figure a
 *  teacher counted when they ticked the tiles. */
export function marketplacesReached(inventories: readonly InventoryId[]): number {
	return new Set(inventories.map((inventory) => MARKETPLACE_OF[inventory])).size;
}

/** Whether the create should navigate to the resource it just made.
 *
 *  No, where the seller has left the create form while the request was in
 *  flight. A create takes seconds against a real server, and `goto` after the
 *  response yanks a seller who has moved on to some other page onto a detail
 *  page they did not ask for — reproduced six times in six. The toast still
 *  fires, because the draft really was created and saying so is the point;
 *  only the navigation is abandoned.
 *
 *  Compares the path the submit was made from with the path now, rather than a
 *  boolean set on unmount: the form is not unmounted by every navigation that
 *  matters, and a path is the thing a test can hold. */
export function shouldLandOnCreated(submittedFrom: string, nowAt: string): boolean {
	return submittedFrom === nowAt;
}

/** Why this file cannot be a thumbnail, or `null` where it can.
 *
 *  Two refusals the browser can make before a byte is sent. `accept="image/*"`
 *  on the input is a picker hint only — it does not survive a drag-and-drop —
 *  and the create's own held-bytes check proves a blob exists without proving
 *  it is an image or that it is small, because `blob` records a size but no
 *  kind. So a seller who drops a PDF or a 200 MB photo on a slot learns here
 *  rather than from a thumbnail that is permanently broken on the listing.
 *
 *  This does not close the same gap for a crafted API call, which is a
 *  server-side check against bytes the server would have to read. */
export function thumbnailRefusal(
	kind: string,
	bytes: number,
	maxBytes: number
): string | null {
	if (!kind.startsWith('image/')) {
		return 'Thumbnails have to be pictures. Choose a JPEG, PNG or GIF.';
	}
	if (bytes > maxBytes) {
		return `That picture is ${Math.round(bytes / 1024 / 1024)} MB and the limit is ${Math.round(maxBytes / 1024 / 1024)} MB.`;
	}
	return null;
}
