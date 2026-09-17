// Money is calculated by the server, not independently in the browser.

import { post, put, request, type PriceIntent } from '$lib/api';
import type { InventoryId, TermKind } from '$lib/generated/vocab';

const BASE = '/v1/seller-rules';

/** Which half of the feature a rule belongs to: what a resource costs on the
 *  target, or which of the target's own terms it lands under. */
export type RuleKind = 'pricing' | 'mapping';

/** The scopes a rule may be applied automatically in. Separate opt-ins, and
 *  empty by default: a rule that is saved does nothing until the seller either
 *  approves a preview or ticks one of these. */
export type RuleUse = 'copy' | 'move' | 'cross_list';

/** How a converted amount is taken to a penny. `UpToCharm` is the smallest
 *  amount ending .99 at or above the exact conversion, which the server
 *  computes; this client only names it. */
export type Rounding = 'Nearest' | 'UpToCharm';

/** The denominations a quote is expressed between, spelled as `tam-types`
 *  serialises them. */
export type Currency = 'Usd' | 'Gbp';

/** Which branch of the source's price a rule is about. `any` matches both. */
export type PricingCondition = 'any' | 'free' | 'paid';

/** Whether every listed value has to match or any one of them. */
export type MatchMode = 'all' | 'any';

/** One measured attribute the source carries, matched by the source's own
 *  values rather than by a path into a document. */
export interface AttributeCondition {
	axis: TermKind;
	values: string[];
	mode: MatchMode;
}

/** What a rule matches. Every dimension is a constraint that may be empty, an
 *  empty one matches everything, and the dimensions combine with AND. */
export interface RuleConditions {
	pricing: PricingCondition;
	/** Source-native resource types, by the identifiers the import preserved. */
	resource_types: string[];
	/** Literal, case-insensitive words in the source description. */
	keywords: string[];
	keyword_mode: MatchMode;
	attributes: AttributeCondition[];
}

/** What a rule does on the target.
 *
 *  `reference` is a quote identifier the server minted and stored; this client
 *  never authors a provider claim of its own, so a rate cited as official is
 *  one the server can re-check. */
export type RuleAction =
	| { kind: 'pricing'; rate: string; rounding: Rounding; reference: string | null }
	| { kind: 'mapping'; licence: string | null; resource_type: string | null };

export interface SellerRuleDefinition {
	title: string;
	description: string;
	enabled: boolean;
	source: InventoryId;
	target: InventoryId;
	auto_apply: RuleUse[];
	conditions: RuleConditions;
	action: RuleAction;
}

export interface SellerRuleRecord {
	id: string;
	/** Bumped by every save. Carried back on an update or a delete so a rule
	 *  edited in another tab refuses rather than being overwritten. */
	revision: number;
	definition: SellerRuleDefinition;
	author: string;
	created_at: number;
	updated_at: number;
}

/** A rule named where its effect is shown, so a proposed figure can be traced
 *  to the sentence the seller wrote for it. */
export interface RuleMatch {
	id: string;
	revision: number;
	title: string;
	description: string;
}

/** A partial target choice. A missing field is unchanged rather than cleared,
 *  and a missing price is never a fabricated one. */
export interface TargetFields {
	price: PriceIntent | null;
	licence: string | null;
	resource_type: string | null;
}

/** A suggestion, complete enough to be saved and never saved by being listed.
 *  `sources` are the documents the suggestion was read off. */
export interface RulePreset {
	id: string;
	definition: SellerRuleDefinition;
	notice: string;
	sources: string[];
}

/** A one-off, previewed departure from the saved rules. Each field resolves
 *  only itself and stays visible in the preview it produced. */
export interface RuleOverrides {
	rate?: string;
	licence?: string;
	resource_type?: string;
}

/** All of the source catalogue, or the resources named. Two shapes rather than
 *  a flag beside a list, so `all` cannot arrive next to a contradicting
 *  selection. */
export type RuleSelection = { all: true } | { products: string[] };

export interface RuleCounts {
	all: number;
	enabled: number;
	disabled: number;
}

export interface SellerRuleList {
	rows: SellerRuleRecord[];
	page: number;
	has_next: boolean;
	counts: RuleCounts;
}

/** What the list endpoint is asked for. The server filters, searches and
 *  paginates; this client sends the question and renders the answer. */
export interface RuleQuery {
	kind: RuleKind;
	state: 'all' | 'enabled' | 'disabled';
	source?: InventoryId;
	target?: InventoryId;
	q?: string;
	page?: number;
}

/** An official reference quote, stored by the server under `id`. The rate is
 *  carried as the decimal string the provider published and as the integer
 *  micros the server converts with, because a browser float of either would be
 *  a third figure. */
export interface RuleReference {
	id: string;
	source: Currency;
	target: Currency;
	rate: string;
	rate_micros: number;
	/** The observation date the provider published, not the day it was read. */
	as_of: string;
	provider: 'ECB';
	source_url: string;
	fetched_at: number;
	notice: string;
}

export interface PreviewRequest {
	source: InventoryId;
	target: InventoryId;
	selection: RuleSelection;
	/** Absent uses the enabled rules for this direction; an explicit empty
	 *  list means no saved rule at all, which is how a draft or an override is
	 *  previewed on its own. */
	rule_ids?: string[];
	draft?: SellerRuleDefinition;
	overrides?: RuleOverrides;
}

export type RowStatus = 'proposed' | 'unchanged' | 'blocked';
export type RowDecision = 'pending' | 'accepted' | 'rejected';

export interface RulePreviewRow {
	product: string;
	title: string;
	/** The canonical source price, as it stands. Never rewritten by a rule. */
	source_price: PriceIntent;
	/** The target choice already on record for this resource. */
	before: TargetFields;
	/** The whole target choice this row proposes, merged rather than a patch. */
	proposed: TargetFields;
	matches: RuleMatch[];
	blockers: string[];
	status: RowStatus;
	decision: RowDecision;
}

export interface PreviewCounts {
	proposed: number;
	unchanged: number;
	blocked: number;
}

export interface RulePreview {
	id: string;
	source: InventoryId;
	target: InventoryId;
	rows: RulePreviewRow[];
	counts: PreviewCounts;
}

export interface DecisionRequest {
	decision: 'accept' | 'reject';
	selection: RuleSelection;
}

export interface DecisionAck {
	accepted: number;
	rejected: number;
	/** Pending rows left, which is how a seller learns that the blocked rows
	 *  an "all" did not touch are still there. */
	remaining: number;
}

function rulePath(id: string): string {
	return `${BASE}/${encodeURIComponent(id)}`;
}

/** The list query as a search string. Omitted where it would say nothing:
 *  `state=all` and page one are the endpoint's own defaults, and a blank
 *  search is no search rather than a search for the empty string. */
export function ruleQuery(params: RuleQuery): string {
	const search = new URLSearchParams({ kind: params.kind });
	if (params.state !== 'all') {
		search.set('state', params.state);
	}
	if (params.source !== undefined) {
		search.set('source', params.source);
	}
	if (params.target !== undefined) {
		search.set('target', params.target);
	}
	const text = params.q?.trim() ?? '';
	if (text.length > 0) {
		search.set('q', text);
	}
	if (params.page !== undefined && params.page > 1) {
		search.set('page', String(params.page));
	}
	return search.toString();
}

export const sellerRules = {
	/** One page of the seller's rules of one kind, filtered and searched by the
	 *  server. The counts come with the page so the tabs state totals rather
	 *  than the size of the page in hand. */
	list: (params: RuleQuery) => request<SellerRuleList>(`${BASE}?${ruleQuery(params)}`),
	create: (definition: SellerRuleDefinition) =>
		post<SellerRuleRecord>(BASE, { definition }),
	/** The revision the seller edited travels with the save: a rule changed
	 *  elsewhere refuses with 409 rather than being overwritten. */
	update: (id: string, revision: number, definition: SellerRuleDefinition) =>
		put<SellerRuleRecord>(rulePath(id), { revision, definition }),
	remove: (id: string, revision: number) =>
		request<void>(`${rulePath(id)}?revision=${revision}`, { method: 'DELETE' }),
	/** The suggestions, which are neither saved nor enabled by being listed. */
	presets: () => request<{ presets: RulePreset[] }>(`${BASE}/presets`),
	/** Fetches and stores the latest dated official quote. Provider failures
	 *  remain errors; an estimate is never substituted for the quote. */
	reference: (source: Currency, target: Currency) =>
		request<RuleReference>(`${BASE}/reference?source=${source}&target=${target}`),
	/** Persists a proposal without changing target choices or publishing. */
	preview: (body: PreviewRequest) => post<RulePreview>(`${BASE}/preview`, body),
	readPreview: (id: string) =>
		request<RulePreview>(`${BASE}/previews/${encodeURIComponent(id)}`),
	/** Accepts or rejects the preview's pending rows. Replaying the same
	 *  decision is not a second approval; the opposite decision conflicts. */
	decide: (preview: string, body: DecisionRequest) =>
		post<DecisionAck>(`${BASE}/previews/${encodeURIComponent(preview)}/decision`, body)
};
