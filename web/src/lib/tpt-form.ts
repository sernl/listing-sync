// The create form's own model, on the canonical TPT base: what the seller has
// typed, what each control refuses, and the two request bodies it composes.
// Pure, so it tests without a component, and driven by
// `GET /v1/authoring/vocabulary` rather than by a second copy of the capture
// here.
//
// The refusals below mirror `crates/tam-domain/src/product.rs` so a seller
// reads a message as they type. They are not the authority:
// `POST /v1/authoring/check` decides, and the page shows what it returns
// alongside these. Where the two could disagree the server wins, which is why
// nothing here invents a rule the model does not hold — the caps arrive in the
// vocabulary payload and an absent cap is unmeasured rather than unlimited.

import type {
	CreateProductBody,
	DraftInput,
	FacetView,
	FileHandle,
	FormCaps,
	FormVocabularyView,
	PathInput,
	PriceIntent,
	TptBaseInput
} from '$lib/api';
import type { FormGroup, InventoryId } from '$lib/generated/vocab';

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
	education_standards: 'Optional. Four frameworks, each searched on its own.',
	details: 'Nothing here is required.',
	product_status: 'Active listings are visible on the site and searchable. Inactive listings are only visible to you.'
};

/** One alignment the seller claimed. */
export interface StandardPick {
	framework: number;
	code: string;
	statement: string;
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

/** The label TPT's own form puts on each picker, and the cap it states. */
export const PICKER_LABEL: Record<PickerKey, string> = {
	grades: 'Grade Level',
	subjectAreas: 'Subject Area',
	tags: 'Tag',
	formats: 'Format'
};

const CAP_FIELD: Record<PickerKey, keyof FormCaps> = {
	grades: 'grades',
	subjectAreas: 'subject_areas',
	tags: 'tags',
	formats: 'formats'
};

/** TPT marks Grade Level, Subject Area and Tag required; Format carries no
 *  marker. Reported as the form has it and never invented. */
export const REQUIRED_PICKERS: readonly PickerKey[] = ['grades', 'subjectAreas', 'tags'];

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
 *  A mirror of the server's own rules so the seller sees a message as they
 *  type. `POST /v1/authoring/check` is the authority and the page shows its
 *  answer too; nothing here states a rule the model does not hold. */
export function refusalsOf(draft: TptDraft, vocabulary: FormVocabularyView | null): Refusal[] {
	const found: Refusal[] = [];
	const name = draft.name.trim();
	if (name.length === 0) {
		found.push({ group: 'name', control: 'Title', message: 'A product needs a title.' });
	} else if (vocabulary !== null && name.length > vocabulary.limits.title_max_utf16_units) {
		found.push({
			group: 'name',
			control: 'Title',
			message: `The title is ${name.length} characters and the form takes ${vocabulary.limits.title_max_utf16_units}.`
		});
	}
	if (draft.payload.length === 0) {
		found.push({
			group: 'files',
			control: 'Downloadable File',
			message: 'Upload the file buyers download; a product with none cannot be listed anywhere.'
		});
	}
	if (draft.thumbnails.length > 0 && draft.thumbnailMode !== '2') {
		found.push({
			group: 'files',
			control: 'Thumbnails',
			message: 'Thumbnails are attached under an option that shows no slots for them.'
		});
	}
	if (!draft.free) {
		if (minorUnitsOf(draft.price) === null) {
			found.push({
				group: 'price',
				control: 'Price',
				message: 'A paid listing needs a price, written in dollars and cents.'
			});
		} else if (
			vocabulary !== null &&
			(minorUnitsOf(draft.price) ?? 0) < vocabulary.limits.min_price_minor_units
		) {
			found.push({
				group: 'price',
				control: 'Price',
				message: `TPT refuses a price below $${majorUnitsOf(vocabulary.limits.min_price_minor_units)}.`
			});
		}
		if (draft.taxCode === null) {
			found.push({
				group: 'price',
				control: 'Tax Code',
				message:
					'Choose a tax code. It is never chosen for you: designating it is yours under TPT’s terms.'
			});
		}
	}
	if (vocabulary !== null) {
		for (const picker of REQUIRED_PICKERS) {
			if (draft[picker].length === 0) {
				found.push({
					group: 'categories',
					control: PICKER_LABEL[picker],
					message: `${PICKER_LABEL[picker]} is required; choose at least one.`
				});
			}
		}
		for (const picker of ['grades', 'subjectAreas', 'tags', 'formats'] as PickerKey[]) {
			const cap = capOf(vocabulary.caps, picker);
			if (cap !== null && draft[picker].length > cap) {
				found.push({
					group: 'categories',
					control: PICKER_LABEL[picker],
					message: `${PICKER_LABEL[picker]} takes up to ${cap}, and ${draft[picker].length} are chosen.`
				});
			}
		}
	}
	if (draft.copyright === null) {
		found.push({
			group: 'copyright',
			control: null,
			message:
				'Choose one of the two attestations. Nothing is pre-selected, because the statement is yours to make.'
		});
	}
	if (draft.inventories.length === 0) {
		found.push({
			group: 'product_status',
			control: null,
			message: 'Choose at least one marketplace; platforms cannot be added after the draft exists.'
		});
	}
	return found;
}

/** Something worth saying that blocks nothing. */
export interface Advisory {
	group: FormGroup;
	message: string;
}

export function advisoriesOf(draft: TptDraft, vocabulary: FormVocabularyView | null): Advisory[] {
	const found: Advisory[] = [];
	const pages = Number(draft.pagesOrSlides.trim());
	const guidance = vocabulary?.limits.free_resource_page_guidance ?? null;
	if (draft.free && guidance !== null && Number.isFinite(pages) && pages > guidance) {
		found.push({
			group: 'price',
			message: `TPT advises that free resources be ${guidance} pages or fewer, and this one states ${pages}. It is guidance rather than a rule, so nothing is blocked.`
		});
	}
	return found;
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
	decided_by: DecidedBy;
	/** What this platform drops whatever the seller picks, where the
	 *  projection says so. Null is "nothing recorded", never "nothing lost". */
	loss: string | null;
}

export interface MarketplaceProjection {
	inventory: InventoryId;
	rows: ProjectedRow[];
}

/** One marketplace's tab, built from what this client holds.
 *
 *  Field rows only, because no endpoint renders a product's projection field
 *  by field: `GET /v1/vocabulary/{inventory}` serves the registry's field
 *  table and `GET /v1/mappings` serves mapping heads with recorded losses, and
 *  neither answers "what will this listing's title be on Tes". Axis rows are
 *  the missing half and arrive from the server whole, which is why the row
 *  type already distinguishes them. */
export function projectionOf(draft: TptDraft, inventory: InventoryId): MarketplaceProjection {
	return {
		inventory,
		rows: OVERRIDABLE.map((entry) => ({
			key: entry.field,
			kind: 'field' as const,
			label: entry.label,
			values: [valueFor(draft, inventory, entry.field)],
			decided_by: diverges(draft, inventory, entry.field)
				? ({ by: 'listing_override' } as const)
				: ({ by: 'listing' } as const),
			loss: null
		}))
	};
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
