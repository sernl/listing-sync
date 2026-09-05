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

/** The length of a value in the unit a cap declares, in code units where the
 *  unit is one this client does not know.
 *
 *  The fallback is not defensive habit. `unit` arrives from the wire and this
 *  index has no guard, so a unit added to `LengthUnit` and served before this
 *  client is rebuilt throws inside a render — and a throw during render blanks
 *  the whole page, taking the form down over a character counter. The count is
 *  advisory and the cap it feeds is disclosed rather than enforced, so a count
 *  measured in the wrong unit is a far smaller wrong than a page that stops
 *  drawing. UTF-16 code units because that is what every measured cap in the
 *  registry counts in today. */
export function measure(text: string, unit: LengthUnit): number {
	return (MEASURE[unit] ?? MEASURE.Utf16CodeUnits)(text);
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
	// Nullish rather than `=== undefined`, and the difference is not
	// hypothetical: the server omits the key entirely — `licence` carries
	// `skip_serializing_if = "Option::is_none"` — but a JSON source that writes
	// `"licence": null` instead passes an `=== undefined` guard and then throws
	// on `gate.native` one line later. Absent and null mean the same thing here
	// and are read the same way.
	const gate = view?.authoring.licence ?? undefined;
	if (view === undefined || gate === undefined) {
		return [];
	}
	const named = view.natives.find((native) => native.name === gate.native);
	const labels = new Map((named?.values ?? []).map((value) => [value.id, value.label]));
	return licenceValues(gate, branch).map((id) => ({ id, label: labels.get(id) ?? id }));
}

// --------------------------------------------------------------- the subjects

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

/** What a stated price needs from whatever composed it.
 *
 *  Narrower than any one draft type, for the reason the licence helpers are:
 *  the form that asked this question through the generic `Draft` has gone, and
 *  a parameter typed to a whole draft is unreachable from the edit seed that
 *  still asks it. */
export interface StatedPrice {
	branch: PricingBranch;
	amount: string;
	currency: string;
}

/** The price a draft states, or `null` where it states none this client will
 *  send. */
export function priceOf(draft: StatedPrice): PriceIntent | null {
	if (draft.branch === 'free') {
		return 'Free';
	}
	const minorUnits = toMinorUnits(draft.amount, draft.currency);
	if (minorUnits === null || minorUnits <= 0) {
		return null;
	}
	return { Paid: { minor_units: minorUnits, currency: draft.currency } };
}

/** What answering the licence question needs to know about a draft, whichever
 *  form composed it.
 *
 *  Narrower than [`Draft`] deliberately. The create form on the TPT base holds
 *  a different draft shape and asks the identical question, and a function
 *  typed to the wider one is unreachable from there without a second copy of
 *  it — which is how the TPT form came to send no licence at all while this
 *  file held a tested answer. Typing the parameter as the narrower shape is
 *  what stops the next form doing the same. */
export interface LicenceIntent {
	licence: string | null;
	inventories: readonly InventoryId[];
	branch: PricingBranch;
}

/** Which of these marketplaces gate a licence on a create, in the order given.
 *  Empty where none does, which is what decides whether a form asks at all. */
export function licenceGated(
	inventories: readonly InventoryId[],
	vocabularies: ReadonlyMap<InventoryId, VocabularyView>
): InventoryId[] {
	// `!= null` rather than `!== undefined`, for the same reason as
	// [`licenceOptions`]: a written-out `"licence": null` is not a gate, and
	// reading it as one would ask a seller for a licence the marketplace has no
	// field for and then refuse the create until they chose one.
	return inventories.filter(
		(inventory) => vocabularies.get(inventory)?.authoring.licence != null
	);
}

/** The licence answers a create carries.
 *
 * One already-answered election per selected platform that gates a licence,
 * under the pricing branch the write is gated on: `election_item.raised_by` is
 * nullable precisely so an answer authored on a create form can precede every
 * mapping. The product's own rights declaration is written alongside, because
 * that is the field the product view reads back and the edit form writes. */
export function licenceElections(
	draft: LicenceIntent,
	vocabularies: ReadonlyMap<InventoryId, VocabularyView>
): ElectionInput[] {
	if (draft.licence === null || draft.licence.length === 0) {
		return [];
	}
	const answer = { segments: [draft.licence], native_id: draft.licence };
	return licenceGated(draft.inventories, vocabularies)
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
	draft: LicenceIntent,
	vocabularies: ReadonlyMap<InventoryId, VocabularyView>
): RightsInput | null {
	if (draft.licence === null || draft.licence.length === 0) {
		return null;
	}
	const [holder] = licenceGated(draft.inventories, vocabularies);
	return holder === undefined
		? null
		: { inventory: holder, segments: [draft.licence], native_id: draft.licence };
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
	const price = priceOf(seed);
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

/** Every field this marketplace declares required, by its own name.
 *
 *  Read off the served vocabulary rather than listed here, so a field the
 *  registry adds is reported without this file being taught it. The TPT create
 *  form carries no rights declaration and no elections, so on that form every
 *  one of these is unanswered and the marketplace cannot be chosen there — which
 *  is a thing to say beside the tick box rather than after the submit. */
export function requiredFields(view: VocabularyView): string[] {
	return view.natives.filter((native) => native.required).map((native) => native.name);
}

/** The words a seller reads for a marketplace's own field.
 *
 *  The server's own label wherever it sent one — `required_fields_answered`
 *  carries `native.label`, the words the platform heads the control with — so
 *  the sentence has one source rather than a second copy here that nothing
 *  keeps in step. The wire name stands in for itself otherwise, which is what
 *  the server does with an uncaptured label too. */
export function fieldWords(native: string, label?: string): string {
	const word = label ?? native;
	return `a ${word.toLowerCase()}`;
}

/** One marketplace and one field it asked for and did not get. */
export interface MissingField {
	inventory: InventoryId;
	field: string;
	/** The platform's own words for the field, where the server sent them. */
	label?: string;
}

/** The `required_field_missing` refusal's detail, read back as the pairs the
 *  server put in it.
 *
 *  `required_fields_answered` in `crates/tam-api/src/catalogue.rs` composes
 *  `detail.missing` as `{inventory, field}` entries precisely so a client can
 *  name both, and the client threw them away: the seller read "a selected
 *  platform requires a field this product does not carry" and could not tell
 *  which platform or which field. Returns an empty list for a shape this
 *  client does not recognise, so the caller falls back to the server's own
 *  sentence rather than rendering a half-read one. */
export function missingFields(detail: unknown): MissingField[] {
	if (!isRecord(detail) || !Array.isArray(detail.missing)) {
		return [];
	}
	return detail.missing.flatMap((entry) => {
		if (!isRecord(entry)) {
			return [];
		}
		const { inventory, field, label } = entry;
		if (typeof inventory !== 'string' || typeof field !== 'string') {
			return [];
		}
		return [
			{
				inventory: inventory as InventoryId,
				field,
				...(typeof label === 'string' ? { label } : {})
			}
		];
	});
}

/** That refusal as one sentence naming every marketplace and every field, or
 *  `null` where the detail carried none. */
export function requiredFieldSentence(detail: unknown): string | null {
	const missing = missingFields(detail);
	if (missing.length === 0) {
		return null;
	}
	const asked = missing.map(
		({ inventory, field, label }) => `${platformTitle(inventory)} needs ${fieldWords(field, label)}`
	);
	const named = asked.length === 1 ? asked[0] : `${asked.slice(0, -1).join(', ')} and ${asked.at(-1)}`;
	return `${named}, and this listing does not carry one yet.`;
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
