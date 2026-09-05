// The create form's own model: how a platform is named, which payload shapes
// each one accepts, what the registry says a seller must answer, and the
// request body all of that composes into. Pure, so it tests without a
// component, and driven by `GET /v1/vocabulary/{inventory}` rather than by a
// second copy of the registry here.

import type {
	AuthoringView,
	CanonicalFieldView,
	CreateProductBody,
	ElectionInput,
	FileHandle,
	LicenceGateView,
	MappingHead,
	NativeValueView,
	PathInput,
	PriceIntent,
	ProductView,
	RightsInput,
	VocabularyView
} from '$lib/api';
import { CURRENCIES, MARKETPLACE_OF } from '$lib/listings-view';
import { platformTitle } from '$lib/platforms';
import type {
	CopyFormat,
	InventoryId,
	LengthUnit,
	NativeVocabularyKind,
	PayloadFileRule,
	TermKind
} from '$lib/generated/vocab';

// ------------------------------------------------------------------- pricing

/** The branch a platform's write is gated on. Tes refuses a Creative Commons
 *  licence with a price and refuses `TES-PAID` without one, so the licence
 *  offered depends on this and on nothing else. */
export type PricingBranch = 'free' | 'paid';

/** The denominations this client can write out, taken from the same table the
 *  listings page reads a price back through, so a price it composes is a price
 *  it can render. */
export const CURRENCY_OPTIONS: readonly { value: string; code: string }[] = Object.entries(
	CURRENCIES
).map(([value, { code }]) => ({ value, code }));

/** A typed amount as minor units, or `null` where it is not a number this
 *  client will send. Rejects a fractional minor unit rather than rounding it:
 *  the seller typed a price and a silently altered one is a different price. */
export function toMinorUnits(amount: string, currency: string): number | null {
	const denomination = CURRENCIES[currency];
	if (denomination === undefined) {
		return null;
	}
	const trimmed = amount.trim();
	if (trimmed.length === 0 || !/^\d+(\.\d+)?$/.test(trimmed)) {
		return null;
	}
	const scaled = Number(trimmed) * 10 ** denomination.exponent;
	const rounded = Math.round(scaled);
	return Math.abs(scaled - rounded) < 1e-6 ? rounded : null;
}

/** Minor units written back as the major-unit string a field is seeded with. */
export function toMajorUnits(minorUnits: number, currency: string): string {
	const denomination = CURRENCIES[currency];
	if (denomination === undefined) {
		return '';
	}
	return (minorUnits / 10 ** denomination.exponent).toFixed(denomination.exponent);
}

// ------------------------------------------------------------------ measuring

/** The length of a string in the unit a cap is counted in.
 *
 * Grapheme clusters are counted as codepoints, which is exactly what the
 * server's own `truncate` does: N codepoints can never exceed N clusters, so
 * counting codepoints under-approximates the budget and the two sides cannot
 * disagree about whether a value fits. */
const MEASURE: Record<LengthUnit, (text: string) => number> = {
	Bytes: (text) => new TextEncoder().encode(text).length,
	Utf16CodeUnits: (text) => text.length,
	Codepoints: (text) => [...text].length,
	GraphemeClusters: (text) => [...text].length
};

export function measure(text: string, unit: LengthUnit): number {
	return MEASURE[unit](text);
}

// ---------------------------------------------------------------- the payload

/** Why a platform will not carry this many payload files, or `null` where it
 *  will.
 *
 * A TPT create takes exactly one file into the product slot and refuses any
 * other count before a request is made, so an exploded multi-entry ZIP is
 * TES-only. A seller who wants a bundle on TPT keeps the ZIP whole. */
export function payloadRefusal(rule: PayloadFileRule, files: number): string | null {
	if (rule === 'every_payload_file') {
		return null;
	}
	if (files === 1) {
		return null;
	}
	return files === 0
		? 'takes exactly one file, and this product carries none yet'
		: `takes exactly one file, and this product carries ${files} — upload one file, or keep the ZIP whole`;
}

// ---------------------------------------------------------------- the licence

/** The licence values a create may carry on this platform under this pricing
 *  branch, or an empty list where the platform holds no licence field. */
export function licenceValues(
	gate: LicenceGateView | undefined,
	branch: PricingBranch
): readonly string[] {
	if (gate === undefined) {
		return [];
	}
	return branch === 'free' ? gate.free : gate.paid;
}

/** The same values as a seller reads them: each gate id carrying the words the
 *  captured vocabulary holds for it.
 *
 * The gate names ids alone, and the words are read off the native field it
 * points at, so the label has one source rather than a second copy beside the
 * gate that nothing keeps in step. An id that vocabulary carries no words for
 * stands in for itself, which is what the server does with an uncaptured label
 * too. */
export function licenceOptions(
	view: VocabularyView | undefined,
	branch: PricingBranch
): readonly NativeValueView[] {
	const gate = view?.authoring.licence;
	if (view === undefined || gate === undefined) {
		return [];
	}
	const named = view.natives.find((native) => native.name === gate.native);
	const labels = new Map((named?.values ?? []).map((value) => [value.id, value.label]));
	return licenceValues(gate, branch).map((id) => ({ id, label: labels.get(id) ?? id }));
}

// --------------------------------------------------------------- the subjects

/** The chosen subject ids with one term added or removed.
 *
 * The seller's own order is kept rather than sorted: `subjects` reaches the
 * wire as a list, and reordering it on every tick would send something other
 * than what was done. Ticking a term already held is a no-op, so a repeated
 * change event cannot write it twice. */
export function toggleSubject(chosen: readonly string[], term: string, on: boolean): string[] {
	if (!on) {
		return chosen.filter((held) => held !== term);
	}
	return chosen.includes(term) ? [...chosen] : [...chosen, term];
}

// ------------------------------------------------------------ platform fields

/** One control a platform's section renders, read off the vocabulary rather
 *  than named here. */
export interface AxisControl {
	axis: TermKind;
	/** The platform's own field name, which is what its API calls it. */
	native: string;
	multiple: boolean;
	/** The platform's measured selection limit, where one is measured. */
	cap: number | null;
	required: boolean;
	/** Present only for a captured closed set, each value carrying the words the
	 *  capture holds for it. Absent means the form renders a disclosure rather
	 *  than a select with no options. */
	values: readonly NativeValueView[] | null;
	vocabulary: NativeVocabularyKind;
	/** Why the seller may not hand this axis to a best-fit computation, or
	 *  `null` where the registry permits delegation. */
	nonDelegableReason: string | null;
	/** Why an answer here would reach nothing, or `null` where the create body
	 *  carries it. Stated rather than hidden: an axis with no control and no
	 *  reason reads as an axis the platform does not have. */
	unwritableReason: string | null;
}

/** Whether a create can carry an answer to this axis at all.
 *
 * Two shapes reach the wire: the phase axis, which lands in the create's own
 * `grades`, and a cardinality-one axis, which lands as an `elect_one` answer
 * against this product. A many-valued axis outside phase has no field in the
 * create body and no election shape that describes it honestly — `over_cap`
 * asserts a cap was exceeded — so the form discloses it rather than
 * collecting an answer nothing would carry. */
function unwritableReason(axis: TermKind, multiple: boolean): string | null {
	if (axis === 'phase' || !multiple) {
		return null;
	}
	return 'a create carries no field for this axis, so the projection fills it from your taxonomy';
}

const NON_DELEGABLE_REASON: Record<'legal_content', string> = {
	legal_content: 'issuing a rights grant is the seller’s to make, never ours'
};

/** Every axis a seller can answer on this platform.
 *
 * A read-only native is excluded because nothing this form writes reaches it.
 * The licence axis is excluded too: it is answered once for the whole product
 * through the price-gated selector, not per axis alongside the others. */
export function axisControls(view: VocabularyView): AxisControl[] {
	const byName = new Map(view.natives.map((native) => [native.name, native]));
	return view.axes
		.filter((axis) => axis.axis !== 'licence')
		.flatMap((axis) => {
			const native = byName.get(axis.native);
			if (native === undefined || native.direction === 'read_only') {
				return [];
			}
			const multiple = axis.cardinality === 'many';
			return [
				{
					axis: axis.axis,
					native: axis.native,
					multiple,
					cap: axis.cap ?? null,
					required: axis.required,
					values: native.values ?? null,
					vocabulary: native.vocabulary,
					nonDelegableReason:
						native.delegation.reason === undefined
							? null
							: NON_DELEGABLE_REASON[native.delegation.reason],
					unwritableReason: unwritableReason(axis.axis, multiple)
				}
			];
		});
}

// ------------------------------------------------------------------ the draft

/** What the seller has typed, before it becomes a request body.
 *
 * `licence` is one value for the whole product rather than one per platform:
 * the three Tes inventories serve the identical refdata set, and a licence
 * that differed between them would be the same grant issued twice. */
export interface Draft {
	title: string;
	body: string;
	bodyFormat: CopyFormat;
	branch: PricingBranch;
	amount: string;
	currency: string;
	payload: FileHandle[];
	cover: FileHandle | null;
	previews: FileHandle[];
	inventories: InventoryId[];
	licence: string | null;
	/** Canonical term ids, which is what the create body's `subjects` takes.
	 *  Ours rather than any one marketplace's, so it is answered once for the
	 *  whole product. */
	subjects: string[];
	/** Native ids chosen per inventory, keyed by the native's own name. */
	axes: Record<string, string[]>;
}

/** The key one platform's answer to one axis is held under. */
export function axisKey(inventory: InventoryId, native: string): string {
	return `${inventory}:${native}`;
}

export function emptyDraft(): Draft {
	return {
		title: '',
		body: '',
		bodyFormat: 'Markdown',
		branch: 'free',
		amount: '',
		currency: 'Gbp',
		payload: [],
		cover: null,
		previews: [],
		inventories: [],
		licence: null,
		subjects: [],
		axes: {}
	};
}

/** A refusal the seller can act on. `blocking` refusals mirror something the
 *  server will refuse; the rest are what a platform will do to the value
 *  quietly, which is worth saying before it happens rather than after. */
export interface Refusal {
	field: 'title' | 'description' | 'price' | 'files' | 'platforms' | 'licence';
	message: string;
	blocking: boolean;
}

/** The price a draft states, or `null` where it states none this client will
 *  send. */
export function priceOf(draft: Draft): PriceIntent | null {
	if (draft.branch === 'free') {
		return 'Free';
	}
	const minorUnits = toMinorUnits(draft.amount, draft.currency);
	if (minorUnits === null || minorUnits <= 0) {
		return null;
	}
	return { Paid: { minor_units: minorUnits, currency: draft.currency } };
}

function capRefusal(
	spec: CanonicalFieldView | undefined,
	text: string,
	field: 'title' | 'description',
	platform: string
): Refusal | null {
	if (spec?.cap === undefined) {
		return null;
	}
	const used = measure(text, spec.cap.unit);
	if (used <= spec.cap.limit) {
		return null;
	}
	return {
		field,
		message: `${platform} caps the ${field} at ${spec.cap.limit} and will shorten it to fit; this one is ${used}.`,
		blocking: false
	};
}

function floorRefusal(authoring: AuthoringView, price: PriceIntent | null, platform: string) {
	const floor = authoring.price_floor_minor_units;
	if (floor === undefined || price === null || price === 'Free') {
		return null;
	}
	if (price.Paid.minor_units >= floor) {
		return null;
	}
	return {
		field: 'price' as const,
		message: `${platform} refuses a price below ${floor} minor units when the listing is written.`,
		blocking: false
	};
}

/** Whether this product answers every field a selected platform declares
 *  required.
 *
 * Requiredness is reported as the registry has it and never invented: the Tes
 * licence is the only field declared required anywhere, because it is the
 * only refusal anyone has measured. A required field this form has not been
 * taught reads as unanswered rather than as silently satisfied, which is what
 * the server does too. */
export function unansweredRequired(
	draft: Draft,
	vocabularies: ReadonlyMap<InventoryId, VocabularyView>
): { inventory: InventoryId; native: string }[] {
	const unmet: { inventory: InventoryId; native: string }[] = [];
	for (const inventory of draft.inventories) {
		const view = vocabularies.get(inventory);
		if (view === undefined) {
			continue;
		}
		for (const native of view.natives) {
			if (!native.required) {
				continue;
			}
			const axis = view.axes.find((binding) => binding.native === native.name);
			const answered =
				axis?.axis === 'licence' ? draft.licence !== null && draft.licence.length > 0 : false;
			if (!answered) {
				unmet.push({ inventory, native: native.name });
			}
		}
	}
	return unmet;
}

/** Everything the seller should see before submitting, in the order the form
 *  reads.
 *
 * The blocking entries mirror what `POST /v1/products` refuses, so the seller
 * sees them without a round trip; the server's refusals stay the authority
 * and surface on their own if one is hit anyway. */
export function refusalsOf(
	draft: Draft,
	vocabularies: ReadonlyMap<InventoryId, VocabularyView>
): Refusal[] {
	const found: Refusal[] = [];
	if (draft.title.trim().length === 0) {
		found.push({ field: 'title', message: 'A product needs a title.', blocking: true });
	}
	if (draft.payload.length === 0) {
		found.push({
			field: 'files',
			message: 'A product needs at least one payload file; upload the bytes first.',
			blocking: true
		});
	}
	const price = priceOf(draft);
	if (price === null) {
		found.push({
			field: 'price',
			message: 'A paid price is a positive amount, written in the currency’s own units.',
			blocking: true
		});
	}
	if (draft.inventories.length === 0) {
		found.push({
			field: 'platforms',
			message: 'Choose at least one marketplace to create this draft on.',
			blocking: true
		});
	}
	for (const { inventory, native } of unansweredRequired(draft, vocabularies)) {
		found.push({
			field: native === 'licence' ? 'licence' : 'platforms',
			message: `${platformTitle(inventory)} requires ${native}, and this product does not carry one.`,
			blocking: true
		});
	}
	for (const inventory of draft.inventories) {
		const view = vocabularies.get(inventory);
		if (view === undefined) {
			continue;
		}
		const platform = platformTitle(inventory);
		const refusal = payloadRefusal(view.authoring.payload_files, draft.payload.length);
		if (refusal !== null) {
			found.push({ field: 'files', message: `${platform} ${refusal}.`, blocking: true });
		}
		const spec = (field: string) => view.canonical.find((entry) => entry.field === field);
		const title = capRefusal(spec('title'), draft.title, 'title', platform);
		if (title !== null) {
			found.push(title);
		}
		const body = capRefusal(spec('description'), draft.body, 'description', platform);
		if (body !== null) {
			found.push(body);
		}
		const floor = floorRefusal(view.authoring, price, platform);
		if (floor !== null) {
			found.push(floor);
		}
	}
	return found;
}

export function submittable(refusals: readonly Refusal[]): boolean {
	return !refusals.some((refusal) => refusal.blocking);
}

// ------------------------------------------------------------- the request

/** The licence answers a create carries.
 *
 * One already-answered election per selected platform that gates a licence,
 * under the pricing branch the write is gated on: `election_item.raised_by` is
 * nullable precisely so an answer authored on a create form can precede every
 * mapping. The product's own rights declaration is written alongside, because
 * that is the field the product view reads back and the edit form writes. */
export function licenceElections(
	draft: Draft,
	vocabularies: ReadonlyMap<InventoryId, VocabularyView>
): ElectionInput[] {
	if (draft.licence === null || draft.licence.length === 0) {
		return [];
	}
	const answer = { segments: [draft.licence], native_id: draft.licence };
	return draft.inventories
		.filter((inventory) => vocabularies.get(inventory)?.authoring.licence !== undefined)
		.map((inventory) => ({
			inventory,
			axis: 'licence' as TermKind,
			trigger: 'supply' as const,
			trigger_key: draft.branch,
			answers: [answer]
		}));
}

/** The rights grant, named against the first selected platform that holds a
 *  licence field. The three Tes inventories serve the identical refdata set,
 *  so which of them names the vocabulary does not change the grant. */
export function rightsOf(
	draft: Draft,
	vocabularies: ReadonlyMap<InventoryId, VocabularyView>
): RightsInput | null {
	if (draft.licence === null || draft.licence.length === 0) {
		return null;
	}
	const holder = draft.inventories.find(
		(inventory) => vocabularies.get(inventory)?.authoring.licence !== undefined
	);
	return holder === undefined
		? null
		: { inventory: holder, segments: [draft.licence], native_id: draft.licence };
}

/** The grade declaration, read off whichever selected platform binds the
 *  phase axis to a captured closed vocabulary. Every other axis answer travels
 *  as an election, because the create body carries no field for it. */
export function gradesOf(
	draft: Draft,
	vocabularies: ReadonlyMap<InventoryId, VocabularyView>
): PathInput[] {
	const paths: PathInput[] = [];
	for (const inventory of draft.inventories) {
		const view = vocabularies.get(inventory);
		if (view === undefined) {
			continue;
		}
		for (const control of axisControls(view)) {
			if (control.axis !== 'phase') {
				continue;
			}
			for (const value of draft.axes[axisKey(inventory, control.native)] ?? []) {
				paths.push({ inventory, kind: 'phase', segments: [value], native_id: value });
			}
		}
	}
	return paths;
}

/** Every writable non-phase, non-licence axis answer, as an already-answered
 *  election.
 *
 * `elect_one` generalises to nothing and the server refuses a trigger key on
 * it, so the answer is recorded against this product alone. An axis the
 * projection never raises a question about leaves its answer unused rather
 * than wrong. */
export function axisElections(
	draft: Draft,
	vocabularies: ReadonlyMap<InventoryId, VocabularyView>
): ElectionInput[] {
	const elections: ElectionInput[] = [];
	for (const inventory of draft.inventories) {
		const view = vocabularies.get(inventory);
		if (view === undefined) {
			continue;
		}
		for (const control of axisControls(view)) {
			if (control.axis === 'phase' || control.unwritableReason !== null) {
				continue;
			}
			const chosen = draft.axes[axisKey(inventory, control.native)] ?? [];
			if (chosen.length === 0) {
				continue;
			}
			elections.push({
				inventory,
				axis: control.axis,
				trigger: 'elect_one',
				answers: chosen.map((value) => ({ segments: [value], native_id: value }))
			});
		}
	}
	return elections;
}

export function createBodyOf(
	draft: Draft,
	vocabularies: ReadonlyMap<InventoryId, VocabularyView>
): CreateProductBody | null {
	const price = priceOf(draft);
	if (price === null) {
		return null;
	}
	const rights = rightsOf(draft, vocabularies);
	return {
		title: draft.title.trim(),
		body: draft.body,
		body_format: draft.bodyFormat,
		price,
		payload: draft.payload,
		cover: draft.cover,
		previews: draft.previews,
		subjects: draft.subjects,
		grades: gradesOf(draft, vocabularies),
		rights,
		inventories: draft.inventories,
		elections: [...licenceElections(draft, vocabularies), ...axisElections(draft, vocabularies)]
	};
}


// ---------------------------------------------------------------- the edit

/** What the edit form holds. The platform set is deliberately absent: no
 *  endpoint adds a mapping to an existing product, so an edit changes the
 *  canonical fields and never which marketplaces carry them. */
export interface EditSeed {
	title: string;
	body: string;
	bodyFormat: CopyFormat;
	branch: PricingBranch;
	amount: string;
	currency: string;
	licence: string | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null;
}

/** The price a stored product carries, read back into the fields that wrote
 *  it. A shape this client cannot read seeds a free listing and says nothing,
 *  because guessing an amount is worse than asking for one. */
export function priceSeedOf(price: unknown): Pick<EditSeed, 'branch' | 'amount' | 'currency'> {
	const free = { branch: 'free' as const, amount: '', currency: 'Gbp' };
	if (!isRecord(price) || !isRecord(price.Paid)) {
		return free;
	}
	const { minor_units: minorUnits, currency } = price.Paid;
	if (typeof minorUnits !== 'number' || typeof currency !== 'string') {
		return free;
	}
	if (CURRENCIES[currency] === undefined) {
		return free;
	}
	return { branch: 'paid', amount: toMajorUnits(minorUnits, currency), currency };
}

/** The stored product as the edit form's starting state. */
export function editSeedOf(product: ProductView): EditSeed {
	return {
		title: product.title,
		body: product.body,
		bodyFormat: product.body_format,
		licence: product.rights?.native_id ?? product.rights?.segments[0] ?? null,
		...priceSeedOf(product.price)
	};
}

/** The edit as a request body, or `null` where the price is one this client
 *  will not send.
 *
 * `body_format` always travels with the body it describes, because the server
 * refuses a format on its own: a format that moved without its text is how a
 * Markdown listing acquires escaped markup. A licence cannot be cleared here —
 * the edit carries a stated grant or leaves the stored one alone — because the
 * wire has no way to say "unstated". */
export function patchBodyOf(seed: EditSeed, licenceVocabulary: InventoryId | null) {
	const price = priceOf({ ...emptyDraft(), ...seed });
	if (price === null) {
		return null;
	}
	const rights =
		seed.licence !== null && seed.licence.length > 0 && licenceVocabulary !== null
			? { inventory: licenceVocabulary, segments: [seed.licence], native_id: seed.licence }
			: undefined;
	return {
		title: seed.title.trim(),
		body: seed.body,
		body_format: seed.bodyFormat,
		price,
		rights
	};
}

/** Why this product cannot be edited through us, or `null` where it can.
 *
 * Mirrors the server's own refusal: a bound mapping lowers a revise, and Tes
 * serves neither live-to-live nor live-to-draft, so a listing already live
 * there cannot be edited by us today. The refusal is whole-product rather than
 * per-platform because the edit is on the canonical product and the server
 * refuses the whole request. */
export function editBlockedBy(mappings: readonly MappingHead[]): InventoryId[] {
	return mappings
		.filter(
			(mapping) =>
				mapping.binding_state === 'bound' &&
				mapping.lifecycle_state === 'live' &&
				MARKETPLACE_OF[mapping.inventory] === 'Tes'
		)
		.map((mapping) => mapping.inventory);
}

// ------------------------------------------------------------------- refusals

/** The quota refusal's detail, rendered as the sentence the server composed it
 *  to allow. Returns `null` for a detail this client does not recognise, so a
 *  shape it cannot read falls back to the server's own message. */
export function quotaSentence(detail: unknown): string | null {
	if (typeof detail !== 'object' || detail === null) {
		return null;
	}
	const { quota, used, limit } = detail as Record<string, unknown>;
	if (typeof quota !== 'string' || typeof used !== 'number' || typeof limit !== 'number') {
		return null;
	}
	if (quota === 'storage_bytes_max') {
		return `Your plan stores up to ${formatBytes(limit)} and ${formatBytes(used)} is in use.`;
	}
	if (quota === 'listings_max') {
		return `Your plan carries up to ${limit} listings and ${used} are in the catalogue.`;
	}
	return null;
}

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'] as const;

/** A byte count as a seller reads it. Powers of 1024, which is the unit the
 *  quota itself is declared in. */
export function formatBytes(bytes: number): string {
	if (!Number.isFinite(bytes) || bytes < 0) {
		return '—';
	}
	let value = bytes;
	let unit = 0;
	while (value >= 1024 && unit < BYTE_UNITS.length - 1) {
		value /= 1024;
		unit += 1;
	}
	const digits = unit === 0 || value >= 100 ? 0 : 1;
	return `${value.toFixed(digits)} ${BYTE_UNITS[unit]}`;
}
