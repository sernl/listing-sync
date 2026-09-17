import type { NativeValueView, PriceIntent, VocabularyView } from '$lib/api';
import type { Tab } from '$lib/TabBar.svelte';
import type { InventoryId, TermKind } from '$lib/generated/vocab';
import { formatPrice } from '$lib/listings-view';
import { SHORT_NAME } from '$lib/platforms';
import type {
	AttributeCondition,
	DecisionAck,
	PreviewCounts,
	PreviewRequest,
	RuleCounts,
	RuleKind,
	RuleOverrides,
	RulePreset,
	RulePreviewRow,
	RuleReference,
	Rounding,
	RowStatus,
	RuleUse,
	SellerRuleDefinition
} from '$lib/seller-rules';

// ------------------------------------------------------------- the two pages

export const PRICING_HREF = '/automations/pricing';
export const MAPPINGS_HREF = '/automations/mappings';

/** What each page is called and where it lives, so a hand-off from the
 *  Resources board, the cross-list dialog or the Migrations page cannot come
 *  to name a different path from the navigation. */
export const RULE_PAGE: Record<RuleKind, { href: string; title: string }> = {
	pricing: { href: PRICING_HREF, title: 'Pricing' },
	mapping: { href: MAPPINGS_HREF, title: 'Mappings' }
};

/** What a pricing rule does for a seller, in the words a teacher would use.
 *  Read by the Automations landing card and by the page's own lead. */
export const WHAT_PRICING_IS =
	'Say what a resource should cost on the marketplace you are sending it to — a conversion ' +
	'from the price you already charge, taken to a penny the way you choose. Teachouse ' +
	'proposes the figures; nothing is used until you approve it.';

export const WHAT_MAPPING_IS =
	'Say which of the target marketplace’s own terms your resources should land under — its ' +
	'licence and its resource type. Teachouse proposes the closest fit it has evidence for; ' +
	'you edit it and approve it, and only then does it apply.';

/** Proposal records are not changes to source resources or marketplace listings. */
export const NOTHING_APPLIES_UNTIL_APPROVED =
	'A preview shows source values, your saved target choices and the proposed changes. ' +
	'Approval saves those choices; publishing is a separate action. Rules only apply to future ' +
	'operations without another preview when you explicitly enable “Apply automatically”.';

// ------------------------------------------------------------ the scopes

/** The three scopes a rule may run in without being approved row by row. In
 *  the order the checkboxes show them. */
export const RULE_USES: readonly RuleUse[] = ['copy', 'move', 'cross_list'];

/** A total map, so a scope added on the server is worded here or fails the
 *  type check rather than rendering as an unnamed checkbox. */
export const USE_WORD: Record<RuleUse, string> = {
	copy: 'Copy',
	move: 'Move',
	cross_list: 'Cross-list'
};

export const USE_LINE: Record<RuleUse, string> = {
	copy: 'Apply this rule to resources copied to the target, without asking each time.',
	move: 'Apply this rule to resources moved to the target, without asking each time.',
	cross_list: 'Apply this rule when you cross-list to the target from the Resources board.'
};

/** What ticking nothing means, stated where the checkboxes are: the empty set
 *  is the default and it is not an idle state. */
export const NO_AUTO_APPLY =
	'With none of these ticked the rule proposes and never applies: you approve it per ' +
	'resource in the preview below.';

/** The scopes as one sentence for a saved rule's row. */
export function autoApplyLine(uses: readonly RuleUse[]): string {
	if (uses.length === 0) {
		return 'Proposes only';
	}
	return `Automatic on ${uses.map((use) => USE_WORD[use]).join(', ')}`;
}

// ------------------------------------------------------------- the list

export type RuleState = 'all' | 'enabled' | 'disabled';

export const RULE_STATES: readonly RuleState[] = ['all', 'enabled', 'disabled'];

export const STATE_LABEL: Record<RuleState, string> = {
	all: 'All',
	enabled: 'On',
	disabled: 'Off'
};

/** The list's tabs, with the totals the server sent beside the page.
 *
 *  `null` where the counts have not been read: a figure this console failed to
 *  read is not a figure of zero, and `TabBar` draws no parenthesis for a null
 *  rather than a plausible `(0)`. */
export function ruleTabs(counts: RuleCounts | null): Tab[] {
	return RULE_STATES.map((state) => ({
		id: state,
		label: STATE_LABEL[state],
		count: counts === null ? null : counts[state]
	}));
}

// --------------------------------------------------------- the conditions

export const PRICING_CONDITIONS = ['any', 'free', 'paid'] as const;

export const PRICING_CONDITION_WORD: Record<'any' | 'free' | 'paid', string> = {
	any: 'Any price',
	free: 'Free resources only',
	paid: 'Paid resources only'
};

export const MATCH_MODE_WORD: Record<'all' | 'any', string> = {
	all: 'all of these',
	any: 'any of these'
};

/** The axes a condition may name, and nothing else.
 *
 *  `topic` is left out because the evaluator refuses it: no source topic
 *  declaration is retained on a canonical product, so a rule matching on one
 *  could only ever match nothing. The server answers `UnsupportedAxis` for it,
 *  and offering a control that produced that refusal would be this console
 *  inviting the error. */
export const CONDITION_AXES: readonly TermKind[] = ['subject', 'phase', 'resource_type', 'licence'];

export const AXIS_WORD: Record<TermKind, string> = {
	subject: 'Subject',
	topic: 'Topic',
	resource_type: 'Resource type',
	phase: 'Grade or phase',
	licence: 'Licence'
};

/** What each axis is actually matched against, said under the control.
 *
 *  The four are not the same kind of value, and the difference decides whether
 *  a condition can ever match: a canonical product stores its subjects as our
 *  own term identifiers, while its resource type survives only as the source's
 *  native value, its grades as the paths the seller declared and its licence as
 *  the rights declaration. A picker that called all four "the source's own
 *  terms" would offer subject values that match nothing. */
export const AXIS_MATCH_NOTE: Record<TermKind, string> = {
	subject:
		'Matched against your own subject terms — the ones Teachouse holds for the resource, not the source marketplace’s wording.',
	topic: 'Not matchable: no source topic is kept on a resource.',
	resource_type:
		'Matched against the source marketplace’s own resource-type value, as your import preserved it.',
	phase: 'Matched against the grades you declared, by the paths they were declared under.',
	licence:
		'Matched against the rights you declared for the resource. A resource with no declared rights matches nothing here rather than falling back to a default.'
};

/** A rule as a seller starts it: matches everything, applies nowhere, and is
 *  on — so the only thing a blank rule needs before it can be previewed is a
 *  title and an action. */
export function blankDefinition(
	kind: RuleKind,
	source: InventoryId,
	target: InventoryId
): SellerRuleDefinition {
	return {
		title: '',
		description: '',
		enabled: true,
		source,
		target,
		auto_apply: [],
		conditions: {
			pricing: 'any',
			resource_types: [],
			keywords: [],
			keyword_mode: 'any',
			attributes: []
		},
		action:
			kind === 'pricing'
				? { kind: 'pricing', rate: '', rounding: 'Nearest', reference: null }
				: { kind: 'mapping', licence: null, resource_type: null }
	};
}

/** A copy of a rule, off and manual.
 *
 *  A duplicate that arrived enabled and automatic would double the standing
 *  behaviour of the rule it was copied from the moment it was saved, which is
 *  the opposite of what duplicating one is for: a variant to edit. */
export function duplicateOf(definition: SellerRuleDefinition): SellerRuleDefinition {
	return {
		...definition,
		title: `${definition.title} (copy)`,
		enabled: false,
		auto_apply: [],
		conditions: {
			...definition.conditions,
			resource_types: [...definition.conditions.resource_types],
			keywords: [...definition.conditions.keywords],
			attributes: definition.conditions.attributes.map((one) => ({
				...one,
				values: [...one.values]
			}))
		}
	};
}

/** The words a keyword box holds, as the rule stores them. Blanks and repeats
 *  are dropped: the server refuses an empty keyword, and a trailing comma is
 *  how a seller finishes typing rather than a word they meant. */
export function keywordsFrom(text: string): string[] {
	const seen = new Set<string>();
	for (const raw of text.split(',')) {
		const word = raw.trim();
		if (word.length > 0) {
			seen.add(word);
		}
	}
	return [...seen];
}


// ---------------------------------------------------------------- the rate

/** The manual estimate the founder named: a flat 0.75 from USD to GBP.
 *
 *  An estimate rather than a quote, and said so wherever it is offered. It is
 *  a starting figure for a seller who wants one number they control, not a
 *  reading of any market. */
export const MANUAL_RATE = '0.75';

export const MANUAL_RATE_NOTE =
	'A flat estimate you own: 0.75 GBP for every 1.00 USD. It is not a market rate and it is ' +
	'not read from anywhere — it stays exactly this until you change it.';

/** What the reference action does, said before it is pressed. */
export const REFERENCE_NOTE =
	'Asks the server for the European Central Bank’s published daily reference rate and stores ' +
	'the quote it answers with, so the figure your prices were converted at can be shown and ' +
	're-checked later.';

/** A quote as a seller reads it. The observation date is the provider's, not
 *  the moment it was fetched: a rate read on Monday for Friday's observation
 *  is Friday's rate and saying otherwise would misdate it. */
export function referenceLine(reference: RuleReference): string {
	return (
		`${reference.provider} reference rate ${reference.rate} ` +
		`${reference.source.toUpperCase()}→${reference.target.toUpperCase()}, ` +
		`published for ${reference.as_of}.`
	);
}

/** What a failed quote reads as. No remembered figure and no 0.75 in its
 *  place: a stale rate shown as today's is the one failure a seller cannot
 *  see, so the action refuses and the rate field is left as it was. */
export const REFERENCE_FAILURE =
	'The reference rate could not be read, so no rate has been filled in. Nothing was changed ' +
	'and no earlier figure has been substituted — try again, or type a rate you choose.';

export const ROUNDINGS: readonly Rounding[] = ['Nearest', 'UpToCharm'];

export const ROUNDING_WORD: Record<Rounding, string> = {
	Nearest: 'Nearest penny',
	UpToCharm: 'Up to the next .99'
};

export const ROUNDING_LINE: Record<Rounding, string> = {
	Nearest: 'The converted amount taken to the nearest penny.',
	UpToCharm:
		'The smallest amount ending in .99 that is not below the converted amount, so a price ' +
		'is never rounded down below what the conversion gave.'
};

/** Why this rate cannot be used, or null where it can.
 *
 *  The same shape the server parses: a positive decimal with at most six
 *  fraction digits. Said here so a seller learns it while typing rather than
 *  from a refused save, and checked again there because this is a browser. */
export function rateRefusal(rate: string): string | null {
	const text = rate.trim();
	if (text.length === 0) {
		return 'A pricing rule needs a rate, such as 0.75.';
	}
	if (!/^\d+(\.\d+)?$/.test(text)) {
		return 'A rate is a plain decimal number, such as 0.75 — no currency sign and no comma.';
	}
	const fraction = text.split('.')[1] ?? '';
	if (fraction.length > 6) {
		return 'A rate carries at most six digits after the point.';
	}
	if (Number(text) <= 0) {
		return 'A rate has to be above zero: a rate of zero would make every converted price free.';
	}
	return null;
}

// ------------------------------------------------------------ the refusals

/** Why this pair cannot carry a rule, or null where it can. */
export function rulePairRefusal(source: InventoryId, target: InventoryId): string | null {
	if (source === target) {
		return `A rule converts between two marketplaces, and both ends here are ${SHORT_NAME[source]}.`;
	}
	return null;
}

/** Which marketplaces have a licence field of their own.
 *
 *  A total map, so a marketplace added in Rust is answered here or fails the
 *  type check. TPT states one set of terms for its whole store rather than a
 *  per-listing licence, so a rule that claimed to set one there would be
 *  promising something no adapter can deliver. */
export const CARRIES_LICENCE: Record<InventoryId, boolean> = {
	Tes: true,
	Tpt: false,
	Etsy: false
};

export function licenceTargetRefusal(target: InventoryId): string | null {
	return CARRIES_LICENCE[target]
		? null
		: `${SHORT_NAME[target]} has no per-resource licence field, so Teachouse cannot set one there.`;
}

function attributeRefusal(condition: AttributeCondition): string | null {
	if (!CONDITION_AXES.includes(condition.axis)) {
		return `${AXIS_WORD[condition.axis]} is not an attribute a rule can match on.`;
	}
	if (condition.values.length === 0 || condition.values.some((value) => value.trim().length === 0)) {
		return `Choose at least one ${AXIS_WORD[condition.axis].toLowerCase()} value, or remove that condition.`;
	}
	return null;
}

/** Why this rule cannot be saved, or null where it can. One sentence, naming
 *  the thing the seller would change, in the order they would meet it. */
export function definitionRefusal(definition: SellerRuleDefinition): string | null {
	if (definition.title.trim().length === 0) {
		return 'Give the rule a name, so a proposal can say which rule produced it.';
	}
	const pair = rulePairRefusal(definition.source, definition.target);
	if (pair !== null) {
		return pair;
	}
	if (definition.conditions.keywords.some((word) => word.trim().length === 0)) {
		return 'A keyword cannot be blank. Remove the empty one, or clear the keyword box.';
	}
	for (const condition of definition.conditions.attributes) {
		const refusal = attributeRefusal(condition);
		if (refusal !== null) {
			return refusal;
		}
	}
	if (definition.action.kind === 'pricing') {
		return rateRefusal(definition.action.rate);
	}
	if (definition.action.licence === null && definition.action.resource_type === null) {
		return 'Choose a licence, a resource type, or both: a mapping rule with neither would propose nothing.';
	}
	if (definition.action.licence !== null) {
		return licenceTargetRefusal(definition.target);
	}
	return null;
}

// -------------------------------------------------------------- the summary

/** What a rule does, in one line for its row in the list. */
export function actionSummary(definition: SellerRuleDefinition): string {
	if (definition.action.kind === 'pricing') {
		return `× ${definition.action.rate}, ${ROUNDING_WORD[definition.action.rounding].toLowerCase()}`;
	}
	const parts: string[] = [];
	if (definition.action.licence !== null) {
		parts.push(`licence ${definition.action.licence}`);
	}
	if (definition.action.resource_type !== null) {
		parts.push(`type ${definition.action.resource_type}`);
	}
	return parts.length === 0 ? 'no field chosen' : parts.join(', ');
}

/** What a rule matches, in one line. Every dimension that constrains anything
 *  is named; an unconstrained rule says so rather than reading as narrow. */
export function conditionSummary(definition: SellerRuleDefinition): string {
	const conditions = definition.conditions;
	const parts: string[] = [];
	if (conditions.pricing !== 'any') {
		parts.push(PRICING_CONDITION_WORD[conditions.pricing].toLowerCase());
	}
	if (conditions.resource_types.length > 0) {
		parts.push(`${conditions.resource_types.length} source type(s)`);
	}
	if (conditions.keywords.length > 0) {
		parts.push(
			`${MATCH_MODE_WORD[conditions.keyword_mode]} of ${conditions.keywords.length} keyword(s)`
		);
	}
	for (const attribute of conditions.attributes) {
		parts.push(
			`${AXIS_WORD[attribute.axis].toLowerCase()}: ${MATCH_MODE_WORD[attribute.mode]} of ${attribute.values.length}`
		);
	}
	return parts.length === 0 ? 'Every resource on the source' : parts.join(' · ');
}

/** The direction and both summaries, which is a row's whole second line. */
export function ruleMeta(definition: SellerRuleDefinition): string {
	return (
		`${SHORT_NAME[definition.source]} → ${SHORT_NAME[definition.target]} · ` +
		`${actionSummary(definition)} · ${conditionSummary(definition)} · ` +
		autoApplyLine(definition.auto_apply)
	);
}

// --------------------------------------------------------------- the presets

/** A preset is a suggestion. Said wherever one is shown, because the whole
 *  hazard of a preset is that it looks like a decision already taken. */
export const PRESETS_ARE_SUGGESTIONS =
	'A preset is a starting point, not a saved rule. Loading one fills the form in and changes ' +
	'nothing: it applies only once you save it and then approve a preview, or tick a scope ' +
	'under “Apply automatically”.';

/** The warning a Creative Commons licence suggestion carries, in full and
 *  never abbreviated to "closest fit".
 *
 *  The grant is wider than the marketplace's own in exactly the ways a seller
 *  cares about, and no text here claims the two are equivalent. */
export const FREE_GRANT_WARNING =
	'A Creative Commons grant is not equivalent to TPT’s terms. CC BY-ND allows anyone to ' +
	'redistribute unchanged copies, including commercially, with attribution. Those freedoms ' +
	'cannot be revoked while recipients comply with the licence. CC BY and CC BY-SA also ' +
	'allow sharing adaptations. Choose one only if you intend to grant those rights; otherwise ' +
	'do not publish the resource as free on Tes.';

/** The presets that belong on this page, in this direction. A preset for
 *  another pair is not shown as though it applied here. */
export function presetsFor(
	presets: readonly RulePreset[],
	kind: RuleKind,
	source: InventoryId,
	target: InventoryId
): RulePreset[] {
	return presets.filter(
		(preset) =>
			preset.definition.action.kind === kind &&
			preset.definition.source === source &&
			preset.definition.target === target
	);
}

// --------------------------------------------------------------- the preview

export const STATUS_WORD: Record<RowStatus, string> = {
	proposed: 'Proposed',
	unchanged: 'Unchanged',
	blocked: 'Blocked'
};

/** `unchanged` is grey rather than green: nothing is wrong with it and nothing
 *  is going to happen to it, which is what grey means in this console. */
export const STATUS_TONE: Record<RowStatus, 'ok' | 'soon' | 'bad'> = {
	proposed: 'ok',
	unchanged: 'soon',
	blocked: 'bad'
};

export const DECISION_WORD: Record<'pending' | 'accepted' | 'rejected', string> = {
	pending: 'Not decided',
	accepted: 'Approved',
	rejected: 'Rejected'
};

export const DECISION_TONE: Record<'pending' | 'accepted' | 'rejected', 'soon' | 'ok' | 'warn'> = {
	pending: 'soon',
	accepted: 'ok',
	rejected: 'warn'
};

/** Every figure is printed, zeroes included: a missing clause reads as a
 *  category the preview did not look at. */
export function previewCountsLine(counts: PreviewCounts): string {
	return (
		`${counts.proposed} proposed, ${counts.unchanged} unchanged, ${counts.blocked} blocked`
	);
}

/** What one decision settled, including what it deliberately left alone. */
export function decisionLine(ack: DecisionAck): string {
	return (
		`${ack.accepted} approved, ${ack.rejected} rejected, ${ack.remaining} still undecided. ` +
		'A blocked row is never approved by “all”, so it stays here with its reason.'
	);
}

/** Which saved rules a preview asks for.
 *
 *  Three states rather than a list that might be empty by accident: the
 *  endpoint reads an absent `rule_ids` as "the enabled rules for this
 *  direction" and an explicitly empty one as "no saved rule at all", and those
 *  are different questions. */
export type RuleScope =
	| { kind: 'enabled' }
	| { kind: 'draft' }
	| { kind: 'chosen'; rules: string[] };

export function previewRequestOf(input: {
	source: InventoryId;
	target: InventoryId;
	all: boolean;
	products: readonly string[];
	scope: RuleScope;
	draft: SellerRuleDefinition | null;
	overrides: RuleOverrides;
}): PreviewRequest {
	const body: PreviewRequest = {
		source: input.source,
		target: input.target,
		selection: input.all ? { all: true } : { products: [...input.products] }
	};
	if (input.scope.kind === 'draft') {
		body.rule_ids = [];
	} else if (input.scope.kind === 'chosen') {
		body.rule_ids = [...input.scope.rules];
	}
	if (input.draft !== null) {
		body.draft = input.draft;
	}
	if (Object.keys(input.overrides).length > 0) {
		body.overrides = input.overrides;
	}
	return body;
}

/** Why the preview on screen cannot be asked for, or null where it can. */
export function previewRefusal(input: {
	source: InventoryId;
	target: InventoryId;
	all: boolean;
	products: readonly string[];
	scope: RuleScope;
	draft: SellerRuleDefinition | null;
}): string | null {
	const pair = rulePairRefusal(input.source, input.target);
	if (pair !== null) {
		return pair;
	}
	if (!input.all && input.products.length === 0) {
		return 'Tick the resources to preview, or choose every resource on the source.';
	}
	if (input.scope.kind === 'chosen' && input.scope.rules.length === 0) {
		return 'Tick at least one rule, or preview with the rules that are on.';
	}
	if (input.scope.kind === 'draft') {
		if (input.draft === null) {
			return 'Open a rule in the editor to preview it on its own.';
		}
		return definitionRefusal(input.draft);
	}
	return null;
}

/** Whether a row could be approved. A blocked row cannot: the server refuses
 *  it and an "all" leaves it behind, so the console never offers it. */
export function acceptable(row: RulePreviewRow): boolean {
	return row.decision === 'pending' && row.status !== 'blocked';
}

/** Whether a row could be rejected. A blocked row can: rejecting it is how a
 *  seller clears a proposal they are not going to fix. */
export function rejectable(row: RulePreviewRow): boolean {
	return row.decision === 'pending';
}

export function eligible(
	rows: readonly RulePreviewRow[],
	decision: 'accept' | 'reject'
): RulePreviewRow[] {
	return rows.filter(decision === 'accept' ? acceptable : rejectable);
}

/** Why this decision cannot be sent, or null where it can. */
export function decisionRefusal(
	rows: readonly RulePreviewRow[],
	decision: 'accept' | 'reject',
	all: boolean,
	picked: ReadonlySet<string>
): string | null {
	const offered = eligible(rows, decision);
	if (offered.length === 0) {
		return decision === 'accept'
			? 'Nothing here can be approved: every row is already decided, unchanged or blocked.'
			: 'Nothing here is left to reject.';
	}
	if (all) {
		return null;
	}
	const named = offered.filter((row) => picked.has(row.product));
	if (named.length === 0) {
		return decision === 'accept'
			? 'Tick the rows to approve. A blocked row cannot be approved, so ticking one does not count.'
			: 'Tick the rows to reject.';
	}
	return null;
}

/** What a 409 from a decision means, in the seller's terms. The preview is
 *  discarded with it: the rows on screen were checked against a source, a rule
 *  revision and a target choice that have since moved. */
export const STALE_PREVIEW =
	'Something this preview was based on has changed — a resource, one of the rules, or a ' +
	'target choice already on record — so nothing was written. The preview has been dropped; ' +
	'take it again to see the current figures.';

/** What a 409 from a save means. */
export const STALE_RULE =
	'This rule was changed somewhere else after you opened it, so your edit was not saved. ' +
	'Reload the list and open it again to edit the current version.';

// --------------------------------------------------------- rendering a row

/** A price as a row prints it, through the console's one price renderer. A
 *  missing target price is a field the proposal does not touch, said as such
 *  rather than as a dash that could read as free. */
export function priceText(price: PriceIntent | null): string {
	return price === null ? 'Unchanged' : formatPrice(price);
}

/** A term identifier as a row prints it: the words where they are known, the
 *  identifier itself where they are not, and "unchanged" for an untouched
 *  field. The identifier is never hidden, because it is what the marketplace
 *  receives. */
export function termText(id: string | null, labels: ReadonlyMap<string, string>): string {
	if (id === null) {
		return 'Unchanged';
	}
	const label = labels.get(id);
	return label === undefined || label === id ? id : `${label} (${id})`;
}

/** Every value list this page can offer for one axis, read off a marketplace's
 *  own vocabulary. Licence is the one axis with two branches, because the
 *  marketplace gates the write on whether the resource is free or paid. */
export interface LicenceChoices {
	native: string;
	paid: NativeValueView[];
	free: NativeValueView[];
}

/** The licence values this marketplace admits, split by branch, or null where
 *  it publishes none. Read from the authoring gate rather than from the axis
 *  list: the gate is what the write is actually checked against. */
export function licenceChoices(vocabulary: VocabularyView | undefined): LicenceChoices | null {
	const gate = vocabulary?.authoring.licence;
	if (vocabulary === undefined || gate === undefined) {
		return null;
	}
	const values = vocabulary.natives.find((native) => native.name === gate.native)?.values ?? [];
	const label = new Map(values.map((value) => [value.id, value.label]));
	return {
		native: gate.native,
		paid: gate.paid.filter((id) => id !== 'TES-PAID-SCHOOL').map((id) => ({ id, label: label.get(id) ?? id })),
		free: gate.free.map((id) => ({ id, label: label.get(id) ?? id }))
	};
}

/** Whether this licence is one of the marketplace's free-resource values, and
 *  so carries the wider-grant warning.
 *
 *  Read off the marketplace's own gate rather than off the identifier's
 *  spelling: which values are admissible for a free resource is a fact the
 *  registry measured, and matching "CC" in a string would be this console
 *  deciding what a licence is. */
export function freeGrantChosen(
	licence: string | null,
	choices: LicenceChoices | null
): boolean {
	if (licence === null || choices === null) {
		return false;
	}
	return choices.free.some((value) => value.id === licence);
}
