<script lang="ts">
	import { createQueries, createQuery } from '@tanstack/svelte-query';
	import { onMount, tick as settle, untrack } from 'svelte';
	import { page } from '$app/state';
	import { ApiFailure, api, type ProductHead, type TermView, type VocabularyView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Explain from '$lib/Explain.svelte';
	import FlowActionBar from '$lib/FlowActionBar.svelte';
	import FlowDiagram, { type FlowExample, type FlowPair } from '$lib/FlowDiagram.svelte';
	import FlowStep from '$lib/FlowStep.svelte';
	import Icon from '$lib/Icon.svelte';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import { external } from '$lib/external';
	import type { InventoryId, TermKind } from '$lib/generated/vocab';
	import { INVENTORY_ORDER, formatPrice, normaliseQuery } from '$lib/listings-view';
	import { productsFromUrl } from '$lib/migration-plan';
	import Note from '$lib/Note.svelte';
	import Pagination from '$lib/Pagination.svelte';
	import { SHORT_NAME, platformTitle } from '$lib/platforms';
	import { queryKeys } from '$lib/query';
	import {
		sellerRules,
		type RuleCounts,
		type RuleKind,
		type RuleOverrides,
		type RulePreset,
		type RulePreview,
		type SellerRuleDefinition,
		type SellerRuleRecord
	} from '$lib/seller-rules';
	import StatusPill from '$lib/StatusPill.svelte';
	import Stepper, { type StepMark } from '$lib/Stepper.svelte';
	import TabBar from '$lib/TabBar.svelte';
	import { destinationsOf } from '$lib/pages/templates/destinations';
	import { pageSelection } from '$lib/pages/automations/migration';
	import {
		AXIS_MATCH_NOTE,
		AXIS_WORD,
		CONDITION_AXES,
		DECISION_TONE,
		DECISION_WORD,
		FREE_GRANT_WARNING,
		MANUAL_RATE,
		MANUAL_RATE_NOTE,
		NOTHING_APPLIES_UNTIL_APPROVED,
		NO_AUTO_APPLY,
		PRESETS_ARE_SUGGESTIONS,
		PRICING_CONDITIONS,
		PRICING_CONDITION_WORD,
		REFERENCE_FAILURE,
		REFERENCE_NOTE,
		ROUNDINGS,
		ROUNDING_LINE,
		ROUNDING_WORD,
		RULE_USES,
		STALE_PREVIEW,
		STALE_RULE,
		STATUS_TONE,
		STATUS_WORD,
		USE_LINE,
		USE_WORD,
		acceptable,
		actionSummary,
		autoApplyLine,
		conditionSummary,
		examplePrice,
		blankDefinition,
		decisionLine,
		decisionRefusal,
		definitionRefusal,
		duplicateOf,
		eligible,
		freeGrantChosen,
		keywordsFrom,
		licenceChoices,
		licenceTargetRefusal,
		presetsFor,
		previewCountsLine,
		previewRefusal,
		previewRequestOf,
		priceText,
		rateRefusal,
		referenceLine,
		rejectable,
		ruleMeta,
		rulePairRefusal,
		ruleTabs,
		termText,
		type RuleScope,
		type RuleState
	} from './seller-rules';
	import './automations.css';
	import '$lib/flow.css';

	let { kind }: { kind: RuleKind } = $props();

	/** How many resources the tick list shows at once, and the page size the
	 *  rule list is served at — named here only so the pager's summary can say
	 *  it. */
	const PICK_PER_PAGE = 25;
	const RULES_PER_PAGE = 25;

	const base = $props.id();

	// TPT → TES is the direction the founder named and the only one a reference
	// quote is offered for today, so a seller who changes nothing is on the path
	// that works. Both ends stay their choice.
	let source = $state<InventoryId>('Tpt');
	let target = $state<InventoryId>('Tes');

	// ------------------------------------------------------------- the rules

	let listState = $state<RuleState>('all');
	let box = $state('');
	let rules = $state<SellerRuleRecord[]>([]);
	// Null until the totals have been read: a figure this page failed to read
	// is not a figure of zero, and the tabs draw no count for a null rather
	// than a plausible (0).
	let counts = $state<RuleCounts | null>(null);
	let rulePage = $state(1);
	let ruleNext = $state(false);
	let rulesBusy = $state(false);
	let rulesLoaded = $state(false);
	// A list that could not be read is not a seller with no rules: the rows on
	// screen stay and the failure is said beside them.
	let rulesUnread = $state(false);
	let listFailure = $state<string | null>(null);
	let savedNote = $state<string | null>(null);

	let generation = 0;

	/** One page of the seller's rules, narrowed, searched and paginated by the
	 *  server. The box is a search of the whole list rather than of the page in
	 *  hand, because this endpoint takes `q`. */
	async function readRules(wanted: number) {
		const mine = ++generation;
		rulesBusy = true;
		try {
			const view = await sellerRules.list({
				kind,
				state: listState,
				source,
				target,
				q: normaliseQuery(box),
				page: wanted
			});
			if (mine !== generation) return;
			rules = view.rows;
			counts = view.counts;
			rulePage = view.page;
			ruleNext = view.has_next;
			rulesUnread = false;
			listFailure = null;
		} catch (caught) {
			if (mine !== generation) return;
			rulesUnread = true;
			listFailure =
				caught instanceof ApiFailure
					? caught.message
					: 'Your rules could not be read, so this page cannot list them.';
		} finally {
			if (mine === generation) {
				rulesBusy = false;
				rulesLoaded = true;
			}
		}
	}

	// ------------------------------------------------------------ the editor

	/** The rule being written: a new one, or a saved one with the revision it
	 *  was opened at. The revision travels with the save, so a rule edited in
	 *  another tab refuses rather than being overwritten. */
	interface Editing {
		rule: string | null;
		revision: number | null;
		definition: SellerRuleDefinition;
		/** The preset this was loaded from, so its notice stays on screen while
		 *  the seller edits what it filled in. */
		preset: RulePreset | null;
	}

	let editing = $state<Editing | null>(null);
	let keywordBox = $state('');
	let saving = $state(false);
	let saveFailure = $state<string | null>(null);
	let removing = $state<string | null>(null);
	let removeFailure = $state<string | null>(null);
	let quoteLine = $state<string | null>(null);
	let referencing = $state(false);
	let referenceFailure = $state<string | null>(null);
	let referenceGeneration = 0;

	const draft = $derived(editing?.definition ?? null);
	const draftRefusal = $derived(draft === null ? null : definitionRefusal(draft));

	function clearReference() {
		referenceGeneration += 1;
		referencing = false;
		quoteLine = null;
		referenceFailure = null;
	}

	function open(next: Editing) {
		editing = next;
		keywordBox = next.definition.conditions.keywords.join(', ');
		saveFailure = null;
		clearReference();
		// The editor drives the direction: previewing a rule for another pair
		// while this one is on screen would show figures for a direction the
		// seller is not looking at.
		source = next.definition.source;
		target = next.definition.target;
	}

	function close() {
		editing = null;
		keywordBox = '';
		saveFailure = null;
		clearReference();
		if (scopeKind === 'draft') {
			scopeKind = 'enabled';
		}
	}

	async function save() {
		const held = editing;
		if (held === null || draftRefusal !== null || saving) {
			return;
		}
		const submitted = JSON.stringify(held.definition);
		saving = true;
		saveFailure = null;
		try {
			const saved =
				held.rule === null || held.revision === null
					? await sellerRules.create(held.definition)
					: await sellerRules.update(held.rule, held.revision, held.definition);
			const newerEdits = editing === held && JSON.stringify(held.definition) !== submitted;
			if (editing === held) {
				if (newerEdits) {
					held.rule = saved.id;
					held.revision = saved.revision;
				} else {
					close();
				}
			}
			// The preview on screen was taken against the rules as they were.
			preview = null;
			previewFailure = null;
			await readRules(rulePage);
			savedNote =
				`“${saved.definition.title}” is saved. ` +
				(saved.definition.auto_apply.length === 0
					? 'It proposes and applies nothing until you approve a preview.'
					: `It applies automatically on ${saved.definition.auto_apply.map((use) => USE_WORD[use]).join(', ')}.`) +
				(newerEdits ? ' Your newer edits are not saved.' : '');
		} catch (caught) {
			if (editing !== held) return;
			saveFailure =
				caught instanceof ApiFailure && caught.status === 409
					? STALE_RULE
					: caught instanceof ApiFailure
						? caught.message
						: 'The save could not be confirmed. Reload the saved rules before trying again.';
		} finally {
			saving = false;
		}
	}

	async function remove(row: SellerRuleRecord) {
		removing = row.id;
		removeFailure = null;
		try {
			await sellerRules.remove(row.id, row.revision);
			if (editing?.rule === row.id) {
				close();
			}
			preview = null;
			await readRules(rulePage);
		} catch (caught) {
			removeFailure =
				caught instanceof ApiFailure && caught.status === 409
					? STALE_RULE
					: caught instanceof ApiFailure
						? caught.message
						: 'The rule could not be deleted, so it is still here.';
		} finally {
			removing = null;
		}
	}

	/** Fills the rate in from a quote the server minted and stored.
	 *
	 *  On failure the field is left exactly as it was: a remembered figure shown
	 *  as today's rate is the one error a seller cannot see, so nothing is
	 *  substituted and the refusal is said. */
	async function quote() {
		const held = editing;
		if (held === null || held.definition.action.kind !== 'pricing') {
			return;
		}
		const mine = ++referenceGeneration;
		referencing = true;
		referenceFailure = null;
		try {
			const answer = await sellerRules.reference('Usd', 'Gbp');
			if (mine !== referenceGeneration || editing !== held) return;
			held.definition.action = {
				kind: 'pricing',
				rate: answer.rate,
				rounding: held.definition.action.rounding,
				reference: answer.id
			};
			quoteLine = `${referenceLine(answer)} ${answer.notice}`;
		} catch (caught) {
			if (mine !== referenceGeneration || editing !== held) return;
			quoteLine = null;
			referenceFailure =
				caught instanceof ApiFailure ? `${REFERENCE_FAILURE} (${caught.message})` : REFERENCE_FAILURE;
		} finally {
			if (mine === referenceGeneration) referencing = false;
		}
	}

	// ----------------------------------------------------------- the presets

	const presets = createQuery(() => ({
		queryKey: queryKeys.sellerRulePresets,
		queryFn: () => sellerRules.presets().then((view) => view.presets),
		staleTime: Infinity
	}));

	const offered = $derived(presetsFor(presets.data ?? [], kind, source, target));

	// -------------------------------------------------------- the vocabulary

	// Both ends' vocabularies: the source's for what a condition matches on,
	// the target's for what an action may write. One read per marketplace and
	// cached, because it is the field registry on the wire.
	//
	// The map travels inside an object: `createQueries` copies the result's own
	// enumerable keys onto a reactive proxy and a Map has none, so a bare Map
	// arrives at the component empty.
	const vocabularies = createQueries(() => ({
		queries: INVENTORY_ORDER.map((one: InventoryId) => ({
			queryKey: queryKeys.vocabulary(one),
			queryFn: () => api.vocabulary(one),
			staleTime: Infinity
		})),
		combine: (results: { data?: VocabularyView }[]) => ({
			known: new Map(
				results.flatMap((result) =>
					result.data === undefined
						? []
						: [[result.data.inventory, result.data] as [InventoryId, VocabularyView]]
				)
			)
		})
	}));

	// The canonical terms of every axis a condition may name, so an identifier
	// renders as the words the seller knows it by.
	const termsByAxis = createQueries(() => ({
		queries: CONDITION_AXES.map((axis: TermKind) => ({
			queryKey: queryKeys.taxonomyTerms(axis),
			queryFn: () => api.terms(axis).then((view) => view.terms),
			staleTime: Infinity
		})),
		combine: (results: { data?: TermView[] }[]) => ({
			byAxis: new Map(
				CONDITION_AXES.map((axis, index) => [axis, results[index]?.data ?? []] as const)
			),
			label: new Map(
				results.flatMap((result) => result.data ?? []).map((term) => [term.id, term.label])
			)
		})
	}));

	const sourceVocabulary = $derived(vocabularies.known.get(source));
	const targetVocabulary = $derived(vocabularies.known.get(target));

	/** Every identifier this page can put words to: both ends' native values
	 *  and the canonical terms. A row prints the identifier either way, because
	 *  that is what the marketplace receives. */
	const labels = $derived.by(() => {
		const index = new Map<string, string>(termsByAxis.label);
		for (const vocabulary of [sourceVocabulary, targetVocabulary]) {
			for (const native of vocabulary?.natives ?? []) {
				for (const value of native.values ?? []) {
					index.set(value.id, value.label);
				}
			}
		}
		return index;
	});

	const sourceTypes = $derived(destinationsOf(sourceVocabulary, 'resource_type'));
	const targetTypes = $derived(destinationsOf(targetVocabulary, 'resource_type'));
	const targetLicences = $derived(licenceChoices(targetVocabulary));
	const licenceRefusal = $derived(licenceTargetRefusal(target));

	/** The values a condition on this axis may name, from whichever list the
	 *  evaluator actually matches against.
	 *
	 *  Subject alone is our own taxonomy: a canonical resource keeps its
	 *  subjects as Teachouse term identifiers, so offering the source's wording
	 *  there would be a picker whose every value matched nothing. The other
	 *  three are matched against the source's own values — its resource type,
	 *  the grade paths the seller declared, the rights they declared — so they
	 *  come from that marketplace's vocabulary.
	 *
	 *  Never a text box either way: a term nobody measured is not a term the
	 *  matcher can find. */
	function valuesFor(axis: TermKind): { id: string; label: string }[] {
		if (axis === 'subject') {
			return (termsByAxis.byAxis.get(axis) ?? []).map((term) => ({
				id: term.id,
				label: term.label
			}));
		}
		const held = destinationsOf(sourceVocabulary, axis);
		return held.kind === 'values' ? held.values : [];
	}

	// -------------------------------------------------------- what to preview

	// A selection arriving from the Resources board or the cross-list dialog
	// opens the tick list on what it named; a bare visit opens on the whole
	// source catalogue.
	const preselected = productsFromUrl(page.url.searchParams);
	let all = $state(preselected.length === 0);
	let ticked = $state<Set<string>>(new Set(preselected));
	let pickBox = $state('');

	let products = $state<ProductHead[]>([]);
	let pickPage = $state(1);
	let pickCursors = $state<(string | null)[]>([null]);
	let pickNext = $state<string | null>(null);
	let pickBusy = $state(false);
	let pickLoaded = $state(false);
	let pickUnread = $state(false);

	async function readProducts(cursor: string | null, wanted: number) {
		pickBusy = true;
		try {
			const view = await api.products(cursor, null, PICK_PER_PAGE);
			products = view.products;
			pickNext = view.next_cursor;
			pickPage = wanted;
			pickCursors = [...pickCursors.slice(0, wanted), view.next_cursor];
			pickUnread = false;
		} catch {
			pickUnread = true;
		} finally {
			pickBusy = false;
			pickLoaded = true;
		}
	}

	const pickQuery = $derived(normaliseQuery(pickBox).toLocaleLowerCase());
	// The page in hand, narrowed, and said so where the seller reads it: the
	// products endpoint takes a cursor and a label and no text, so a box that
	// claimed to search the catalogue would be searching twenty-five rows of it.
	const shown = $derived(
		pickQuery.length === 0
			? products
			: products.filter((product) => product.title.toLocaleLowerCase().includes(pickQuery))
	);
	const allShownTicked = $derived(
		shown.length > 0 && shown.every((product) => ticked.has(product.id))
	);
	const tickedOffPage = $derived(
		[...ticked].filter((id) => !products.some((product) => product.id === id)).length
	);

	// Which rules the preview asks for. Three states, because the endpoint reads
	// an absent list and an empty one as different questions.
	let scopeKind = $state<'enabled' | 'draft' | 'chosen'>('enabled');
	let chosenRules = $state<Set<string>>(new Set());
	const scope = $derived<RuleScope>(
		scopeKind === 'chosen' ? { kind: 'chosen', rules: [...chosenRules] } : { kind: scopeKind }
	);

	// A previewed one-off: each field resolves only itself and stays on screen
	// with the preview it produced.
	let rateOverride = $state('');
	let licenceOverride = $state('');
	let typeOverride = $state('');
	const overrides = $derived.by(() => {
		const held: RuleOverrides = {};
		if (rateOverride.trim().length > 0) {
			held.rate = rateOverride.trim();
		}
		if (licenceOverride.length > 0) {
			held.licence = licenceOverride;
		}
		if (typeOverride.length > 0) {
			held.resource_type = typeOverride;
		}
		return held;
	});
	const overrideRateRefusal = $derived(
		rateOverride.trim().length === 0 ? null : rateRefusal(rateOverride)
	);

	const body = $derived(
		previewRequestOf({
			source,
			target,
			all,
			products: [...ticked],
			scope,
			draft: scopeKind === 'draft' ? draft : null,
			overrides
		})
	);
	// The request aliases the editable draft; object identity misses nested edits.
	const bodyKey = $derived(JSON.stringify(body));

	let preview = $state<RulePreview | null>(null);
	let previewing = $state(false);
	let previewFailure = $state<string | null>(null);
	let picked = $state<Set<string>>(new Set());
	let deciding = $state<'accept' | 'reject' | null>(null);
	let decisionNote = $state<string | null>(null);
	let decisionFailure = $state<string | null>(null);
	let previewNeedsRead = $state(false);
	let reloadingPreview = $state(false);
	const decisionReadRefusal = $derived(
		previewNeedsRead ? 'Reload the saved preview before another decision.' : null
	);

	const askRefusal = $derived(
		previewRefusal({
			source,
			target,
			all,
			products: [...ticked],
			scope,
			draft: scopeKind === 'draft' ? draft : null
		}) ?? overrideRateRefusal
	);

	const pairRefusal = $derived(rulePairRefusal(source, target));

	/** The rows a decision could still name, which is what the select-all box
	 *  and the two "selected" buttons act over. */
	const undecided = $derived(preview === null ? [] : eligible(preview.rows, 'reject'));
	const allUndecidedPicked = $derived(
		undecided.length > 0 && undecided.every((row) => picked.has(row.product))
	);

	const acceptRefusal = $derived(
		decisionReadRefusal ??
			(preview === null
				? 'Take a preview first, so you can see what would be proposed.'
				: decisionRefusal(preview.rows, 'accept', false, picked))
	);
	const rejectRefusal = $derived(
		decisionReadRefusal ??
			(preview === null ? 'Take a preview first.' : decisionRefusal(preview.rows, 'reject', false, picked))
	);
	const acceptAllRefusal = $derived(
		decisionReadRefusal ??
			(preview === null
				? 'Take a preview first, so you can see what would be proposed.'
				: decisionRefusal(preview.rows, 'accept', true, picked))
	);
	const rejectAllRefusal = $derived(
		decisionReadRefusal ??
			(preview === null ? 'Take a preview first.' : decisionRefusal(preview.rows, 'reject', true, picked))
	);

	// A changed direction, selection, rule scope or override makes the preview
	// on screen a statement about something the seller is no longer asking for,
	// so it is dropped rather than left to be approved. The row ticks go with
	// it: they name rows of that preview.
	$effect(() => {
		void bodyKey;
		preview = null;
		previewFailure = null;
		decisionNote = null;
		decisionFailure = null;
		previewNeedsRead = false;
		reloadingPreview = false;
		picked = new Set();
	});

	// The direction, the tab and the search are the list's question, so changing
	// one re-asks the server rather than sieving the page in hand. `untrack`
	// keeps the read itself from subscribing this effect to everything it
	// touches.
	$effect(() => {
		void `${kind}|${listState}|${source}|${target}`;
		untrack(() => void readRules(1));
	});

	onMount(() => {
		void readProducts(null, 1);
	});

	async function ask() {
		const requested = bodyKey;
		previewing = true;
		previewFailure = null;
		try {
			const answer = await sellerRules.preview(body);
			if (bodyKey !== requested) return;
			preview = answer;
			picked = new Set();
		} catch (caught) {
			if (bodyKey !== requested) return;
			preview = null;
			previewFailure =
				caught instanceof ApiFailure
					? caught.message
					: 'The preview could not be taken, so nothing is shown rather than a guess.';
		} finally {
			previewing = false;
		}
	}

	async function reloadPreview() {
		const held = preview;
		if (held === null) return;
		const requested = bodyKey;
		reloadingPreview = true;
		previewNeedsRead = true;
		try {
			const updated = await sellerRules.readPreview(held.id);
			if (bodyKey !== requested || preview?.id !== held.id) return;
			preview = updated;
			picked = new Set();
			previewNeedsRead = false;
			decisionFailure = null;
		} catch {
			if (bodyKey !== requested || preview?.id !== held.id) return;
			decisionFailure =
				'The saved preview could not be reloaded. Retry before making another decision.';
		} finally {
			if (bodyKey === requested && preview?.id === held.id) reloadingPreview = false;
		}
	}

	/** Accepts or rejects, either the ticked rows or every eligible one.
	 *
	 *  The preview is re-read afterwards rather than patched here, so each row
	 *  says what the server now holds for it. A 409 means something the preview
	 *  was checked against has moved: nothing was written, and the preview is
	 *  dropped rather than left on screen as a statement about a past state. */
	async function decide(decision: 'accept' | 'reject', everything: boolean) {
		const held = preview;
		if (held === null || previewNeedsRead || deciding !== null) {
			return;
		}
		const requested = bodyKey;
		deciding = decision;
		decisionFailure = null;
		decisionNote = null;
		try {
			const ack = await sellerRules.decide(held.id, {
				decision,
				selection: everything
					? { all: true }
					: {
							products: eligible(held.rows, decision)
								.filter((row) => picked.has(row.product))
								.map((row) => row.product)
						}
			});
			if (bodyKey !== requested || preview?.id !== held.id) return;
			decisionNote = decisionLine(ack);
			await reloadPreview();
		} catch (caught) {
			if (bodyKey !== requested || preview?.id !== held.id) return;
			picked = new Set();
			if (caught instanceof ApiFailure && caught.status === 409) {
				preview = null;
				decisionFailure = STALE_PREVIEW;
			} else {
				previewNeedsRead = true;
				decisionFailure =
					'The decision could not be confirmed. Reload this saved preview to read its recorded decisions.';
			}
		} finally {
			deciding = null;
		}
	}

	function setKeywords(text: string) {
		keywordBox = text;
		if (editing !== null) {
			editing.definition.conditions.keywords = keywordsFrom(text);
		}
	}

	function toggleUse(use: (typeof RULE_USES)[number], on: boolean) {
		if (editing === null) return;
		const held = editing.definition.auto_apply.filter((one) => one !== use);
		editing.definition.auto_apply = on ? [...held, use] : held;
	}

	function toggleSourceType(id: string, on: boolean) {
		if (editing === null) return;
		const held = editing.definition.conditions.resource_types.filter((one) => one !== id);
		editing.definition.conditions.resource_types = on ? [...held, id] : held;
	}

	function toggleAttributeValue(index: number, id: string, on: boolean) {
		const condition = editing?.definition.conditions.attributes[index];
		if (condition === undefined) return;
		const held = condition.values.filter((one) => one !== id);
		condition.values = on ? [...held, id] : held;
	}

	/** A new condition starts on the first axis with no values, which matches
	 *  nothing until the seller picks one — and the save refuses an empty one,
	 *  so a half-written condition cannot become a rule that matches
	 *  everything. */
	function addCondition() {
		if (editing === null) return;
		editing.definition.conditions.attributes = [
			...editing.definition.conditions.attributes,
			{ axis: CONDITION_AXES[0], values: [], mode: 'any' }
		];
	}

	function dropCondition(index: number) {
		if (editing === null) return;
		editing.definition.conditions.attributes = editing.definition.conditions.attributes.filter(
			(_, at) => at !== index
		);
	}

	function setLicence(value: string) {
		if (editing === null || editing.definition.action.kind !== 'mapping') return;
		editing.definition.action = {
			...editing.definition.action,
			licence: value.length === 0 ? null : value
		};
	}

	function setResourceType(value: string) {
		if (editing === null || editing.definition.action.kind !== 'mapping') return;
		editing.definition.action = {
			...editing.definition.action,
			resource_type: value.length === 0 ? null : value
		};
	}

	function setManualRate(rate: string) {
		if (editing === null || editing.definition.action.kind !== 'pricing') return;
		editing.definition.action.rate = rate;
		editing.definition.action.reference = null;
		clearReference();
	}

	function tick(product: string, on: boolean) {
		const next = new Set(ticked);
		if (on) {
			next.add(product);
		} else {
			next.delete(product);
		}
		ticked = next;
	}

	function pickRow(product: string, on: boolean) {
		const next = new Set(picked);
		if (on) {
			next.add(product);
		} else {
			next.delete(product);
		}
		picked = next;
	}

	function chooseRule(id: string, on: boolean) {
		const next = new Set(chosenRules);
		if (on) {
			next.add(id);
		} else {
			next.delete(id);
		}
		chosenRules = next;
	}

	const tabs = $derived(ruleTabs(counts));

	// ------------------------------------------------------------ the flow

	const ruleWord = $derived(kind === 'pricing' ? 'price rule' : 'term rule');
	const newLabel = $derived(kind === 'pricing' ? 'New price rule' : 'New rule');

	/** The rule the diagram draws on its arrow: the one being written, else the
	 *  first rule on the list that is on, else the first preset. */
	const shownRule = $derived<SellerRuleDefinition | null>(
		draft ?? rules.find((row) => row.definition.enabled)?.definition ?? offered[0]?.definition ?? null
	);
	const shownRuleIsPreset = $derived(
		draft === null &&
			!rules.some((row) => row.definition.enabled) &&
			offered[0] !== undefined
	);

	const arrowText = $derived.by(() => {
		if (shownRule === null) return null;
		if (shownRule.action.kind === 'pricing') {
			return shownRule.action.rate.trim().length === 0
				? 'Rate not set'
				: `× ${shownRule.action.rate} · ${ROUNDING_WORD[shownRule.action.rounding].toLowerCase()}`;
		}
		const parts: string[] = [];
		if (shownRule.action.licence !== null) parts.push('licence');
		if (shownRule.action.resource_type !== null) parts.push('type');
		return parts.length === 0 ? 'Nothing set yet' : `Sets ${parts.join(' and ')}`;
	});

	/** One real resource put through the pricing rule on the arrow. The first
	 *  paid one on the page of resources this component already holds, so the
	 *  example is the seller's own and costs no extra read. */
	const example = $derived.by<FlowExample | null>(() => {
		if (shownRule === null || shownRule.action.kind !== 'pricing') return null;
		const action = shownRule.action;
		for (const product of products) {
			if (product.price === 'Free') continue;
			const after = examplePrice(product.price, action.rate, action.rounding, source, target);
			if (after !== null) {
				return {
					name: product.title,
					before: formatPrice(product.price),
					after: formatPrice(after)
				};
			}
		}
		return null;
	});

	/** Source terms against target terms, for every mapping rule on screen
	 *  that is on (or the one being written). */
	const pairs = $derived.by<FlowPair[]>(() => {
		if (kind !== 'mapping') return [];
		const definitions =
			draft !== null
				? [draft]
				: rules.filter((row) => row.definition.enabled).map((row) => row.definition);
		return definitions.slice(0, 4).flatMap((definition) => {
			if (definition.action.kind !== 'mapping') return [];
			const from = [
				...(definition.conditions.pricing === 'any'
					? []
					: [PRICING_CONDITION_WORD[definition.conditions.pricing]]),
				...definition.conditions.resource_types.map((id) => labels.get(id) ?? id),
				...definition.conditions.attributes.flatMap((one) =>
					one.values.map((id) => labels.get(id) ?? id)
				),
				...definition.conditions.keywords.map((word) => `“${word}”`)
			];
			const to = [definition.action.licence, definition.action.resource_type]
				.filter((id): id is string => id !== null)
				.map((id) => labels.get(id) ?? id);
			return to.length === 0 ? [] : [{ from: from.length === 0 ? ['Any resource'] : from, to }];
		});
	});

	const scopeWord = $derived(
		scopeKind === 'enabled'
			? 'Rules that are on'
			: scopeKind === 'chosen'
				? `${chosenRules.size} ticked rule${chosenRules.size === 1 ? '' : 's'}`
				: 'The rule you are writing'
	);
	const oneOffs = $derived(Object.keys(overrides).length);

	const steps = $derived<StepMark[]>([
		{ id: 'where', label: 'Where', done: pairRefusal === null },
		{ id: 'rule', label: 'Rule', done: (counts?.enabled ?? 0) > 0 && editing === null },
		{ id: 'what', label: 'What', done: all || ticked.size > 0 },
		{ id: 'preview', label: 'Preview', done: preview !== null },
		{
			id: 'approve',
			label: 'Approve',
			done: preview !== null && preview.rows.length > 0 && undecided.length === 0
		}
	]);

	async function startNew(definition: SellerRuleDefinition, preset: RulePreset | null) {
		open({ rule: null, revision: null, definition, preset });
		ruleOpen = true;
		await settle();
		document.getElementById(`${base}-title`)?.focus();
	}

	/** Takes the preview and brings its rows into view, where the approval
	 *  is. */
	async function askAndShow() {
		await ask();
		if (preview === null) return;
		approveOpen = true;
		await settle();
		document.getElementById('step-approve')?.scrollIntoView({ behavior: 'smooth', block: 'start' });
	}

	/** The tabs and the search only earn their space on a list long enough
	 *  to need them, or while one of them is narrowing it. */
	const longList = $derived(
		counts === null ||
			counts.all > 5 ||
			listState !== 'all' ||
			normaliseQuery(box).length > 0
	);

	let whereOpen = $state(true);
	let ruleOpen = $state(true);
	let whatOpen = $state(true);
	let previewOpen = $state(true);
	let approveOpen = $state(true);
</script>

<div class="flow" class:has-editor={editing !== null}>
	<Stepper steps={steps} label="{kind === 'pricing' ? 'Pricing' : 'Target terms'} steps" />

	<FlowStep
		n={1}
		id="where"
		title="Where"
		hint="Choose where the resource comes from and where it goes."
		summary="{SHORT_NAME[source]} → {SHORT_NAME[target]}"
		done={pairRefusal === null}
		bind:open={whereOpen}
	>
		<div class="set-grid">
			<Field label="From" id="{base}-source">
				<select id="{base}-source" bind:value={source}>
					{#each INVENTORY_ORDER as one (one)}
						<option value={one}>{platformTitle(one)}</option>
					{/each}
				</select>
			</Field>
			<Field label="To" id="{base}-target">
				<select id="{base}-target" bind:value={target}>
					{#each INVENTORY_ORDER as one (one)}
						<option value={one}>{platformTitle(one)}</option>
					{/each}
				</select>
			</Field>
		</div>
		{#if pairRefusal !== null}
			<p class="flow-warn">{pairRefusal}</p>
		{/if}
		{#if kind === 'mapping' && licenceRefusal !== null}
			<p class="flow-warn">{licenceRefusal}</p>
		{/if}

		<FlowDiagram
			from={{ inventory: source }}
			to={[{ inventory: target }]}
			rule={arrowText}
			empty="Add a rule in step 2"
			{example}
			exampleLabel={shownRuleIsPreset ? 'With the preset' : 'For example'}
			{pairs}
			label="{SHORT_NAME[source]} to {SHORT_NAME[target]}{arrowText === null ? '' : `, ${arrowText}`}"
		/>
	</FlowStep>

	<FlowStep
		n={2}
		id="rule"
		title="Rule"
		hint="Pick a rule, start from a suggestion, or write your own."
		summary={counts === null
			? undefined
			: `${counts.enabled} on · ${counts.disabled} off`}
		done={(counts?.enabled ?? 0) > 0 && editing === null}
		bind:open={ruleOpen}
		actionInBar
		footer={editing === null ? undefined : editorFooter}
	>
		{#snippet action()}
			<Button
				tier="primary"
				icon="circle-plus"
				small
				onclick={() => void startNew(blankDefinition(kind, source, target), null)}
			>
				{newLabel}
			</Button>
		{/snippet}

		{#if savedNote !== null}
			<Banner tone="ok" onDismiss={() => (savedNote = null)}>{savedNote}</Banner>
		{/if}
		{#if removeFailure !== null}
			<Banner tone="bad">{removeFailure}</Banner>
		{/if}

		{#if editing !== null && draft !== null}
			{@render editor(editing, draft)}
		{/if}

		<div class="flow-section">
			<div class="flow-section-head">
				<h3>Your {ruleWord}s</h3>
			</div>

			{#if longList}
				<TabBar
					{tabs}
					current={listState}
					onselect={(id) => {
						listState = id as RuleState;
					}}
				/>

				<div class="pick-head">
					<!-- A search of the whole list rather than of the page in hand: this
					     endpoint takes `q`, so the server answers it over every rule. -->
					<input
						type="search"
						aria-label="Search your rules by name or description"
						placeholder="Search your rules"
						bind:value={box}
						onchange={() => void readRules(1)}
					/>
				</div>
			{/if}

			{#if !rulesLoaded}
				<p class="quiet">Loading your rules…</p>
			{:else}
				{#if rulesUnread && listFailure !== null}
					<Banner tone="bad" action={retryRules}>
						{listFailure} Nothing you have saved has been lost.
					</Banner>
				{/if}
				{#if rules.length === 0}
					{#if rulePage === 1 && normaliseQuery(box).length === 0 && listState === 'all'}
						<div class="flow-empty">
							<p>No {ruleWord} for {SHORT_NAME[source]} → {SHORT_NAME[target]} yet.</p>
							<Button
								tier="primary"
								icon="circle-plus"
								onclick={() => void startNew(blankDefinition(kind, source, target), null)}
							>
								{newLabel}
							</Button>
						</div>
					{:else}
						<p class="quiet">
							{rulePage > 1
								? 'Nothing left on this page. Go back a page.'
								: normaliseQuery(box).length > 0
									? 'No rule matches that search.'
									: `No ${ruleWord} here is ${listState === 'enabled' ? 'on' : 'off'}.`}
						</p>
					{/if}
				{:else}
					<div class="flow-list">
						{#each rules as row (row.id)}
							<div
								class="flow-item"
								class:off={!row.definition.enabled}
								class:picked={scopeKind === 'chosen' && chosenRules.has(row.id)}
							>
								<span class="rule-lead">
									{#if scopeKind === 'chosen'}
										<input
											type="checkbox"
											checked={chosenRules.has(row.id)}
											aria-label={`Preview with ${row.definition.title}`}
											onchange={(event) => chooseRule(row.id, event.currentTarget.checked)}
										/>
									{/if}
									<StatusPill
										tone={row.definition.enabled ? 'ok' : 'soon'}
										label={row.definition.enabled ? 'On' : 'Off'}
									/>
								</span>
								<span class="flow-item-main">
									<span class="flow-item-title">{row.definition.title}</span>
									<span class="rule-does">
										<span class="rule-action-chip">{actionSummary(row.definition)}</span>
										<span class="flow-item-line">
											{conditionSummary(row.definition)} · {autoApplyLine(row.definition.auto_apply)}
										</span>
									</span>
									{#if row.definition.description.length > 0}
										<span class="flow-item-line">{row.definition.description}</span>
									{/if}
								</span>
								<span class="flow-item-acts">
									<Button
										small
										icon="pencil"
										onclick={() => {
											open({
												rule: row.id,
												revision: row.revision,
												definition: structuredClone(row.definition),
												preset: null
											});
											ruleOpen = true;
										}}
									>
										Edit
									</Button>
									<Button
										small
										tier="quiet"
										icon="copy"
										onclick={() => void startNew(duplicateOf(row.definition), null)}
									>
										Duplicate
									</Button>
									<Button
										small
										tier="quiet"
										danger
										icon="trash-2"
										disabled={removing === row.id}
										reason={removing === row.id ? 'This rule is being deleted.' : undefined}
										onclick={() => void remove(row)}
									>
										Delete
									</Button>
								</span>
							</div>
						{/each}
					</div>
				{/if}
				{#if ruleNext || rulePage > 1}
					<Pagination
						page={rulePage}
						hasNext={ruleNext}
						busy={rulesBusy}
						label="Your rules"
						summary={`${rules.length} of up to ${RULES_PER_PAGE} on this page`}
						onprevious={() => void readRules(rulePage - 1)}
						onnext={() => void readRules(rulePage + 1)}
					/>
				{/if}
			{/if}
		</div>

		<div class="flow-section">
			<div class="flow-section-head">
				<h3>Suggestions</h3>
				<Explain title="About suggestions" label="">
					<p>{PRESETS_ARE_SUGGESTIONS}</p>
					<p>Each one says what it is based on. Check it against your own prices before you use it.</p>
				</Explain>
			</div>
			{#if presets.isPending}
				<p class="quiet">Loading suggestions…</p>
			{:else if presets.isError}
				<p class="quiet">Suggestions could not be loaded.</p>
			{:else if offered.length === 0}
				<p class="quiet">No suggestion for {SHORT_NAME[source]} → {SHORT_NAME[target]}.</p>
			{:else}
				<div class="chip-row">
					{#each offered as preset (preset.id)}
						{@const licence =
							preset.definition.action.kind === 'mapping' ? preset.definition.action.licence : null}
						<span class="choice-chip">
							<span class="rule-action-chip">{actionSummary(preset.definition)}</span>
							<span>{preset.definition.title}</span>
							<Explain title={preset.definition.title} label="" tone={freeGrantChosen(licence, targetLicences) ? 'warn' : 'info'}>
								<p>{preset.definition.description}</p>
								<p class="quiet">{ruleMeta(preset.definition)}</p>
								{#if freeGrantChosen(licence, targetLicences)}
									<p class="flow-warn">{FREE_GRANT_WARNING}</p>
								{/if}
								<p>{preset.notice}</p>
								{#if preset.sources.length > 0}
									<p class="quiet">
										Read from:
										{#each preset.sources as href, index (href)}
											{index > 0 ? ' · ' : ''}<a {href} use:external target="_blank" rel="noreferrer noopener">
												{href}
											</a>
										{/each}
									</p>
								{/if}
							</Explain>
							<Button
								small
								tier="outline"
								onclick={() => void startNew(structuredClone(preset.definition), preset)}
							>
								Use this
							</Button>
						</span>
					{/each}
				</div>
			{/if}
		</div>

	</FlowStep>

	<FlowStep
		n={3}
		id="what"
		title="What"
		hint="Choose the resources to try it on."
		summary={all ? `Every resource on ${SHORT_NAME[source]}` : `${ticked.size} chosen`}
		done={all || ticked.size > 0}
		bind:open={whatOpen}
	>
		<div class="flow-choice" role="radiogroup" aria-label="Which resources">
			<button type="button" role="radio" aria-checked={all} onclick={() => (all = true)}>
				Every resource on {SHORT_NAME[source]}
			</button>
			<button type="button" role="radio" aria-checked={!all} onclick={() => (all = false)}>
				Let me choose
			</button>
		</div>

		{#if !all}
			<div class="flow-pick">
				{#if !pickLoaded}
					<p class="quiet">Loading your resources…</p>
				{:else if pickUnread && products.length === 0}
					<Banner tone="bad" action={retryPicks}>
						Your resources could not be loaded.
					</Banner>
				{:else if products.length === 0 && pickPage === 1}
					<p class="quiet">Import your shop first, then add resources here.</p>
				{:else}
					{#if pickUnread}
						<Banner tone="bad" action={retryPicks}>
							That page could not be loaded. These are the last ones that did.
						</Banner>
					{/if}
					<div class="pick-head">
						<input
							type="search"
							aria-label="Filter the resources on this page"
							placeholder="Filter this page"
							bind:value={pickBox}
						/>
						<label class="pick-all">
							<input
								type="checkbox"
								checked={allShownTicked}
								onchange={() => {
									ticked = pageSelection(
										ticked,
										shown.map((product) => product.id),
										allShownTicked
									);
								}}
							/>
							Select these {shown.length}
						</label>
						<span class="pick-count" role="status" aria-live="polite">
							{ticked.size} chosen{tickedOffPage > 0 ? `, ${tickedOffPage} on other pages` : ''}
						</span>
					</div>
					{#if shown.length === 0}
						<p class="quiet">
							{products.length === 0
								? 'Nothing on this page. Go back a page.'
								: 'Nothing on this page matches. Clear the filter or try Next.'}
						</p>
					{:else}
						<div class="pick-list">
							{#each shown as product (product.id)}
								<label class="pick-row">
									<input
										type="checkbox"
										checked={ticked.has(product.id)}
										onchange={(event) => tick(product.id, event.currentTarget.checked)}
									/>
									<span class="pick-title">{product.title}</span>
									<span class="pick-price">{formatPrice(product.price)}</span>
								</label>
							{/each}
						</div>
					{/if}
					<Pagination
						page={pickPage}
						hasNext={pickNext !== null}
						busy={pickBusy}
						label="Your resources"
						summary={`${products.length} resources on this page`}
						onprevious={() => void readProducts(pickCursors[pickPage - 2] ?? null, pickPage - 1)}
						onnext={() => void readProducts(pickNext, pickPage + 1)}
					/>
				{/if}
			</div>
		{/if}
	</FlowStep>

	<FlowStep
		n={4}
		id="preview"
		title="Preview"
		hint="Choose which rules to try, then preview."
		summary="{scopeWord}{oneOffs > 0 ? ` · ${oneOffs} one-off change${oneOffs === 1 ? '' : 's'}` : ''}"
		done={preview !== null}
		bind:open={previewOpen}
	>
		<div class="flow-choice" role="radiogroup" aria-label="Which rules">
			<button
				type="button"
				role="radio"
				aria-checked={scopeKind === 'enabled'}
				onclick={() => (scopeKind = 'enabled')}
			>
				Rules that are on
				<span class="sub">Price and term rules for this direction.</span>
			</button>
			<button
				type="button"
				role="radio"
				aria-checked={scopeKind === 'chosen'}
				onclick={() => {
					scopeKind = 'chosen';
					ruleOpen = true;
				}}
			>
				Only rules I tick
				<span class="sub">Tick them in step 2.</span>
			</button>
			<button
				type="button"
				role="radio"
				aria-checked={scopeKind === 'draft'}
				onclick={() => (scopeKind = 'draft')}
			>
				Only the rule I am writing
				<span class="sub">Try it before you save it.</span>
			</button>
		</div>

		<details class="flow-more" open={oneOffs > 0}>
			<summary>One-off changes for this preview{oneOffs > 0 ? ` (${oneOffs})` : ''}</summary>
			<Field label="Rate" id="{base}-override-rate" hint="Leave empty to use the rules.">
				<input id="{base}-override-rate" type="text" inputmode="decimal" bind:value={rateOverride} />
			</Field>
			{#if overrideRateRefusal !== null}
				<p class="flow-warn">{overrideRateRefusal}</p>
			{/if}
			<div class="set-grid">
				<Field label="Licence" id="{base}-override-licence">
					{#if licenceRefusal !== null}
						<p class="quiet" id="{base}-override-licence">{licenceRefusal}</p>
					{:else if targetLicences === null}
						<p class="quiet" id="{base}-override-licence">
							{SHORT_NAME[target]}’s licences have not loaded yet.
						</p>
					{:else}
						<select id="{base}-override-licence" bind:value={licenceOverride}>
							<option value="">Use the rules</option>
							<optgroup label="For a paid resource">
								{#each targetLicences.paid as value (value.id)}
									<option value={value.id}>{value.label}</option>
								{/each}
							</optgroup>
							<optgroup label="For a free resource">
								{#each targetLicences.free as value (value.id)}
									<option value={value.id}>{value.label}</option>
								{/each}
							</optgroup>
						</select>
					{/if}
				</Field>
				<Field label="Resource type" id="{base}-override-type">
					{#if targetTypes.kind === 'values'}
						<select id="{base}-override-type" bind:value={typeOverride}>
							<option value="">Use the rules</option>
							{#each targetTypes.values as value (value.id)}
								<option value={value.id}>{value.label}</option>
							{/each}
						</select>
					{:else}
						<p class="quiet" id="{base}-override-type">
							{SHORT_NAME[target]} has no resource-type list here.
						</p>
					{/if}
				</Field>
			</div>
			{#if freeGrantChosen(licenceOverride.length === 0 ? null : licenceOverride, targetLicences)}
				{@render freeGrant()}
			{/if}
		</details>

		{#snippet footer()}
			<Button
				tier="primary"
				icon="eye"
				disabled={previewing || deciding !== null || askRefusal !== null || previewNeedsRead}
				reason={decisionReadRefusal ??
					askRefusal ??
					(previewing
						? 'The preview is being taken.'
						: deciding !== null
							? 'A decision is being sent.'
							: undefined)}
				onclick={() => void askAndShow()}
			>
				{previewing ? 'Previewing…' : 'Preview'}
			</Button>
			<span class="flow-foot-note">
				{askRefusal ?? NOTHING_APPLIES_UNTIL_APPROVED}
			</span>
		{/snippet}
	</FlowStep>

	<FlowStep
		n={5}
		id="approve"
		title="Approve"
		hint={preview === null ? 'Preview first, then approve what you like.' : 'Tick the rows you want, then approve.'}
		summary={preview === null ? 'No preview yet' : previewCountsLine(preview.counts)}
		done={preview !== null && preview.rows.length > 0 && undecided.length === 0}
		bind:open={approveOpen}
		footer={preview !== null && preview.rows.length > 0 ? approveFooter : undefined}
	>
		{#if previewFailure !== null}
			<Banner tone="bad">{previewFailure}</Banner>
		{/if}
		{#if decisionFailure !== null}
			<Banner tone="bad" title="Decision needs attention">{decisionFailure}</Banner>
			{#if previewNeedsRead}
				<div class="flow-actions">
					<Button disabled={reloadingPreview} onclick={() => void reloadPreview()}>
						{reloadingPreview ? 'Reloading…' : 'Reload the saved preview'}
					</Button>
				</div>
			{/if}
		{/if}
		{#if decisionNote !== null}
			<Banner tone="ok" onDismiss={() => (decisionNote = null)}>{decisionNote}</Banner>
		{/if}

		{#if preview === null}
			<p class="quiet">No preview yet.</p>
		{:else if preview.rows.length === 0}
			<p class="quiet">These choices name no resources.</p>
		{:else}
			<div class="pick-head">
				<label class="pick-all">
					<input
						type="checkbox"
						checked={allUndecidedPicked}
						disabled={undecided.length === 0}
						onchange={() => {
							picked = pageSelection(
								picked,
								undecided.map((row) => row.product),
								allUndecidedPicked
							);
						}}
					/>
					Select the {undecided.length} undecided
				</label>
				<span class="pick-count" role="status" aria-live="polite">
					{picked.size} selected · {previewCountsLine(preview.counts)}
				</span>
			</div>

			<div class="pv-rows">
				<div class="pv-row pv-head" aria-hidden="true">
					<span></span>
					<span>Resource</span>
					<span>On {SHORT_NAME[preview.source]}</span>
					<span>Now on {SHORT_NAME[preview.target]}</span>
					<span>Proposed</span>
					<span>Status</span>
				</div>
				{#each preview.rows as row (row.product)}
					{@const termsBefore = `${termText(row.before.licence, labels)} · ${termText(row.before.resource_type, labels)}`}
					{@const termsAfter = `${termText(row.proposed.licence, labels)} · ${termText(row.proposed.resource_type, labels)}`}
					<div class="pv-row" class:blocked={row.status === 'blocked'}>
						<span class="pv-pick">
							<input
								type="checkbox"
								checked={picked.has(row.product)}
								disabled={!rejectable(row)}
								title={rejectable(row) ? undefined : 'Already decided.'}
								aria-label={`Select ${row.title}`}
								onchange={(event) => pickRow(row.product, event.currentTarget.checked)}
							/>
						</span>
						<span class="pv-name">
							<span class="res-name">{row.title}</span>
							<span class="pv-why">
								{#if row.matches.length > 0}
									{#each row.matches as match (match.id)}
										<span title={match.description}>{match.title}</span>
									{/each}
								{:else}
									<span>No rule matched.</span>
								{/if}
							</span>
							{#each row.blockers as blocker (blocker)}
								<span class="pv-blocker">{blocker}</span>
							{/each}
							{#if !acceptable(row) && row.decision === 'pending' && row.blockers.length === 0}
								<span class="pv-blocker">Blocked rows cannot be approved.</span>
							{/if}
						</span>
						<span class="pv-cells">
						<span class="pv-cell">
							<span class="pv-label">On {SHORT_NAME[preview.source]}</span>
							<span class="price muted-price">{priceText(row.source_price)}</span>
						</span>
						<span class="pv-cell">
							<span class="pv-label">Now on {SHORT_NAME[preview.target]}</span>
							<span class="price muted-price">{priceText(row.before.price)}</span>
							<span class="pv-terms">{termsBefore}</span>
						</span>
						<span class="pv-cell">
							<span class="pv-label">Proposed</span>
							<span class="price">{priceText(row.proposed.price)}</span>
							<span class="pv-terms to">{termsAfter}</span>
						</span>
						</span>
						<span class="pv-status">
							<StatusPill tone={STATUS_TONE[row.status]} label={STATUS_WORD[row.status]} />
							<StatusPill tone={DECISION_TONE[row.decision]} label={DECISION_WORD[row.decision]} />
						</span>
					</div>
				{/each}
			</div>
		{/if}

	</FlowStep>
</div>

<FlowActionBar>
	<Button
		tier="primary"
		icon="circle-plus"
		onclick={() => void startNew(blankDefinition(kind, source, target), null)}
	>
		{newLabel}
	</Button>
</FlowActionBar>

{#snippet editorFooter()}
	<Button
		tier="primary"
		icon="circle-check"
		disabled={draftRefusal !== null || saving}
		reason={draftRefusal ?? (saving ? 'The rule is being saved.' : undefined)}
		onclick={() => void save()}
	>
		{saving ? 'Saving…' : editing?.rule === null ? 'Save this rule' : 'Save changes'}
	</Button>
	<Button tier="quiet" onclick={close}>Cancel</Button>
	{#if draftRefusal !== null}
		<span class="flow-foot-note">{draftRefusal}</span>
	{/if}
{/snippet}

{#snippet approveFooter()}
	<Button
		tier="primary"
		icon="circle-check"
		disabled={acceptRefusal !== null || deciding !== null}
		reason={acceptRefusal ?? (deciding !== null ? 'A decision is being sent.' : undefined)}
		onclick={() => void decide('accept', false)}
	>
		Approve selected
	</Button>
	<Button
		icon="circle-x"
		disabled={rejectRefusal !== null || deciding !== null}
		reason={rejectRefusal ?? (deciding !== null ? 'A decision is being sent.' : undefined)}
		onclick={() => void decide('reject', false)}
	>
		Reject selected
	</Button>
	<Button
		tier="outline"
		icon="circle-check"
		disabled={acceptAllRefusal !== null || deciding !== null}
		reason={acceptAllRefusal ?? (deciding !== null ? 'A decision is being sent.' : undefined)}
		onclick={() => void decide('accept', true)}
	>
		Approve all eligible
	</Button>
	<Button
		tier="quiet"
		icon="circle-x"
		disabled={rejectAllRefusal !== null || deciding !== null}
		reason={rejectAllRefusal ?? (deciding !== null ? 'A decision is being sent.' : undefined)}
		onclick={() => void decide('reject', true)}
	>
		Reject all undecided
	</Button>
	<Explain title="What “all eligible” means" label="">
		<p>“All eligible” is every undecided row that is not blocked.</p>
		<p>A blocked row stays with its reason. Reject it to clear it.</p>
	</Explain>
{/snippet}

{#snippet freeGrant()}
	<p class="flow-warn">
		<Icon name="triangle-alert" size={15} />
		Free licence: anyone may share it.
		<Explain title="A free licence is not the marketplace’s own" label="Read the fine print" tone="warn">
			<p>{FREE_GRANT_WARNING}</p>
		</Explain>
	</p>
{/snippet}

{#snippet editor(held: Editing, def: SellerRuleDefinition)}
	<div class="flow-editor" aria-label={held.rule === null ? newLabel : `Editing ${def.title}`} role="group">
		<div class="flow-editor-head">
			<h3 class="rule-heading">
				{held.rule === null ? newLabel : `Edit “${def.title}”`}
			</h3>
			{#if held.preset !== null}
				<span class="choice-chip solo">
					From a suggestion
					<Explain title="About this suggestion" label="">
						<p>{held.preset.notice}</p>
						<p>{PRESETS_ARE_SUGGESTIONS}</p>
					</Explain>
				</span>
			{/if}
		</div>

		<div class="set-grid">
			<Field label="Name" id="{base}-title">
				<input id="{base}-title" type="text" bind:value={def.title} />
			</Field>
			<Field label="On or off" id="{base}-enabled">
				<label class="set-toggle">
					<input id="{base}-enabled" type="checkbox" bind:checked={def.enabled} />
					<span>{def.enabled ? 'On' : 'Off'}</span>
				</label>
			</Field>
		</div>

		{#if def.action.kind === 'pricing'}
			<div class="set-grid">
				<Field label="Multiply the price by" id="{base}-rate">
					<input
						id="{base}-rate"
						type="text"
						inputmode="decimal"
						placeholder={MANUAL_RATE}
						value={def.action.rate}
						oninput={(event) => setManualRate(event.currentTarget.value)}
					/>
				</Field>
				<Field label="Then round" id="{base}-rounding">
					<select id="{base}-rounding" bind:value={def.action.rounding}>
						{#each ROUNDINGS as one (one)}
							<option value={one}>{ROUNDING_WORD[one]}</option>
						{/each}
					</select>
				</Field>
			</div>
			<div class="flow-actions">
				<Button small tier="outline" onclick={() => setManualRate(MANUAL_RATE)}>
					Use {MANUAL_RATE}
				</Button>
				<Button
					small
					tier="outline"
					disabled={referencing}
					reason={referencing ? 'The rate is being fetched.' : undefined}
					onclick={() => void quote()}
				>
					{referencing ? 'Fetching…' : 'Use today’s bank rate'}
				</Button>
				<Explain title="About rates and rounding" label="">
					<p>{MANUAL_RATE_NOTE}</p>
					<p>{REFERENCE_NOTE}</p>
					<p><strong>{ROUNDING_WORD[def.action.rounding]}:</strong> {ROUNDING_LINE[def.action.rounding]}</p>
				</Explain>
			</div>
			{#if quoteLine !== null}
				<Banner tone="info" title="Rate filled in from a stored quote">{quoteLine}</Banner>
			{:else if def.action.reference !== null}
				<Note icon="clock">This rate uses a stored quote.</Note>
			{/if}
			{#if referenceFailure !== null}
				<Banner tone="bad" title="No rate was filled in">{referenceFailure}</Banner>
			{/if}
		{:else}
			<div class="set-grid">
				<Field label="Licence on {SHORT_NAME[def.target]}" id="{base}-licence">
					{#if licenceRefusal !== null}
						<p class="quiet" id="{base}-licence">{licenceRefusal}</p>
					{:else if targetLicences === null}
						<p class="quiet" id="{base}-licence">
							{SHORT_NAME[def.target]}’s licences have not loaded yet.
						</p>
					{:else}
						<select
							id="{base}-licence"
							value={def.action.licence ?? ''}
							onchange={(event) => setLicence(event.currentTarget.value)}
						>
							<option value="">Leave the licence alone</option>
							<optgroup label="For a paid resource">
								{#each targetLicences.paid as value (value.id)}
									<option value={value.id}>{value.label}</option>
								{/each}
							</optgroup>
							<optgroup label="For a free resource">
								{#each targetLicences.free as value (value.id)}
									<option value={value.id}>{value.label}</option>
								{/each}
							</optgroup>
						</select>
					{/if}
				</Field>
				<Field label="Resource type on {SHORT_NAME[def.target]}" id="{base}-type">
					{#if targetTypes.kind === 'values'}
						<select
							id="{base}-type"
							value={def.action.resource_type ?? ''}
							onchange={(event) => setResourceType(event.currentTarget.value)}
						>
							<option value="">Leave the resource type alone</option>
							{#each targetTypes.values as value (value.id)}
								<option value={value.id}>{value.label}</option>
							{/each}
						</select>
					{:else}
						<p class="quiet" id="{base}-type">
							{targetTypes.kind === 'unread'
								? `${SHORT_NAME[def.target]}’s types have not loaded yet.`
								: targetTypes.kind === 'unbound'
									? `${SHORT_NAME[def.target]} has no resource type field.`
									: `Teachouse has no list of ${SHORT_NAME[def.target]}’s types.`}
						</p>
					{/if}
				</Field>
			</div>
			{#if freeGrantChosen(def.action.licence, targetLicences)}
				{@render freeGrant()}
			{/if}
		{/if}

		<details class="flow-more" open={conditionSummary(def) !== 'Every resource on the source'}>
			<summary>Only some resources? {conditionSummary(def)}</summary>
			<div class="set-grid">
				<Field label="Price on {SHORT_NAME[def.source]}" id="{base}-pricing">
					<select id="{base}-pricing" bind:value={def.conditions.pricing}>
						{#each PRICING_CONDITIONS as one (one)}
							<option value={one}>{PRICING_CONDITION_WORD[one]}</option>
						{/each}
					</select>
				</Field>
				<Field label="Words in the description" id="{base}-keywords" hint="Separate with commas.">
					<input
						id="{base}-keywords"
						type="text"
						value={keywordBox}
						oninput={(event) => setKeywords(event.currentTarget.value)}
					/>
				</Field>
			</div>
			{#if def.conditions.keywords.length > 1}
				<Field label="The words must match" id="{base}-keyword-mode">
					<select id="{base}-keyword-mode" bind:value={def.conditions.keyword_mode}>
						<option value="any">Any one of them</option>
						<option value="all">All of them</option>
					</select>
				</Field>
			{/if}

			<Field label="Resource types on {SHORT_NAME[def.source]}" id="{base}-source-types">
				{#if sourceTypes.kind === 'unread'}
					<p class="quiet" id="{base}-source-types">
						{SHORT_NAME[def.source]}’s types have not loaded yet.
					</p>
				{:else if sourceTypes.kind === 'unbound'}
					<p class="quiet" id="{base}-source-types">
						{SHORT_NAME[def.source]} has no resource type field.
					</p>
				{:else if sourceTypes.kind === 'open'}
					<p class="quiet" id="{base}-source-types">
						Match on words or an attribute instead.
					</p>
				{:else}
					<div class="rule-values" id="{base}-source-types">
						{#each sourceTypes.values as value (value.id)}
							<label class="rule-value">
								<input
									type="checkbox"
									checked={def.conditions.resource_types.includes(value.id)}
									onchange={(event) => toggleSourceType(value.id, event.currentTarget.checked)}
								/>
								{value.label}
							</label>
						{/each}
					</div>
				{/if}
			</Field>

			{#each def.conditions.attributes as condition, index (index)}
				{@const choices = valuesFor(condition.axis)}
				<div class="rule-cond">
					<div class="set-grid">
						<Field label="Attribute" id="{base}-axis-{index}">
							<select id="{base}-axis-{index}" bind:value={condition.axis}>
								{#each CONDITION_AXES as axis (axis)}
									<option value={axis}>{AXIS_WORD[axis]}</option>
								{/each}
							</select>
						</Field>
						<Field label="Must match" id="{base}-mode-{index}">
							<select id="{base}-mode-{index}" bind:value={condition.mode}>
								<option value="any">Any one of these</option>
								<option value="all">All of these</option>
							</select>
						</Field>
					</div>
					{#if choices.length === 0}
						<p class="quiet">
							No {AXIS_WORD[condition.axis].toLowerCase()} values yet. Remove this or pick another.
						</p>
					{:else}
						<div class="rule-values">
							{#each choices as choice (choice.id)}
								<label class="rule-value">
									<input
										type="checkbox"
										checked={condition.values.includes(choice.id)}
										onchange={(event) =>
											toggleAttributeValue(index, choice.id, event.currentTarget.checked)}
									/>
									{choice.label}
								</label>
							{/each}
						</div>
					{/if}
					<div class="flow-actions">
						<Button small tier="quiet" icon="trash-2" onclick={() => dropCondition(index)}>
							Remove
						</Button>
						<Explain title="What {AXIS_WORD[condition.axis].toLowerCase()} matches" label="">
							<p>{AXIS_MATCH_NOTE[condition.axis]}</p>
						</Explain>
					</div>
				</div>
			{/each}
			<div class="flow-actions">
				<Button small tier="outline" icon="filter" onclick={addCondition}>Add a condition</Button>
				<Explain title="How conditions combine" label="">
					<p>Every condition has to match. With none, the rule matches every resource.</p>
				</Explain>
			</div>
		</details>

		<details class="flow-more" open={def.auto_apply.length > 0}>
			<summary>Apply without asking? {autoApplyLine(def.auto_apply)}</summary>
			<div class="rule-values rule-uses">
				{#each RULE_USES as use (use)}
					<label class="rule-value rule-use">
						<input
							type="checkbox"
							checked={def.auto_apply.includes(use)}
							onchange={(event) => toggleUse(use, event.currentTarget.checked)}
						/>
						<span>
							<strong>{USE_WORD[use]}</strong>
							<span class="meta">{USE_LINE[use]}</span>
						</span>
					</label>
				{/each}
			</div>
			<Note>{NO_AUTO_APPLY}</Note>
		</details>

		<Field label="Note (optional)" id="{base}-description" hint="Shown beside what this rule proposes.">
			<textarea id="{base}-description" rows="2" bind:value={def.description}></textarea>
		</Field>

		{#if saveFailure !== null}
			<Banner tone="bad" title="Not saved">{saveFailure}</Banner>
		{/if}
	</div>
{/snippet}

{#snippet retryRules()}
	<Button
		tier="outline"
		small
		disabled={rulesBusy}
		reason={rulesBusy ? 'A page is being read.' : undefined}
		onclick={() => void readRules(rulePage)}
	>
		Retry
	</Button>
{/snippet}

{#snippet retryPicks()}
	<Button
		tier="outline"
		small
		disabled={pickBusy}
		reason={pickBusy ? 'A page is being read.' : undefined}
		onclick={() => void readProducts(pickCursors[pickPage - 1] ?? null, pickPage)}
	>
		Retry
	</Button>
{/snippet}

<style>
	.rule-lead {
		display: inline-flex;
		align-items: center;
		gap: var(--s-2);
	}

	.rule-does {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		flex-wrap: wrap;
	}

	.rule-action-chip {
		display: inline-flex;
		padding: 2px 9px;
		border-radius: var(--r-pill);
		background: var(--accent-soft);
		color: var(--primary);
		font-size: 12.5px;
		font-weight: 600;
		white-space: nowrap;
	}

	.flow-empty {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: var(--s-3);
		padding: var(--s-5);
		border: 1.5px dashed var(--line);
		border-radius: var(--r-panel);
	}

	.flow-empty p {
		margin: 0;
		color: var(--muted);
	}

	.flow-editor {
		display: flex;
		flex-direction: column;
		gap: var(--s-4);
		padding: var(--s-5);
		border: 1.5px solid var(--lavender);
		border-radius: var(--r-panel);
		background: color-mix(in srgb, var(--additive-soft) 35%, var(--surface));
	}

	.flow-editor-head {
		display: flex;
		align-items: center;
		gap: var(--s-3);
		flex-wrap: wrap;
	}

	.flow-editor-head .rule-heading {
		flex: 1 1 auto;
	}

	.flow-foot-note {
		color: var(--muted);
		font-size: 13px;
		flex: 1 1 14rem;
	}

	/* The preview: one row per resource, columns on a wide screen and a card
	   on a phone. */
	.pv-rows {
		display: flex;
		flex-direction: column;
		border: 1px solid var(--line);
		border-radius: var(--r-panel);
		overflow: hidden;
	}

	.pv-row {
		display: grid;
		grid-template-columns: 28px minmax(0, 2.2fr) repeat(3, minmax(0, 1fr)) auto;
		align-items: center;
		gap: var(--s-3);
		padding: var(--s-3) var(--s-4);
		border-top: 1px solid var(--line);
		background: var(--surface);
	}

	.pv-row.pv-head {
		border-top: 0;
		background: var(--nav);
		color: var(--muted);
		font-size: 12px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.04em;
	}

	.pv-row.blocked {
		background: color-mix(in srgb, var(--warn-soft) 45%, var(--surface));
	}

	.pv-cells {
		display: contents;
	}

	.pv-name,
	.pv-cell {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}

	.pv-why {
		display: flex;
		flex-wrap: wrap;
		gap: 2px 8px;
		color: var(--muted);
		font-size: 12px;
	}

	.pv-blocker {
		color: var(--warn-ink);
		font-size: 12px;
		font-weight: 500;
	}

	.pv-label {
		display: none;
		color: var(--muted);
		font-size: 11px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.04em;
	}

	.muted-price {
		color: var(--muted);
		font-weight: 500;
	}

	.pv-terms {
		color: var(--muted);
		font-size: 12px;
	}

	.pv-terms.to {
		color: var(--primary);
	}

	.pv-status {
		display: flex;
		flex-direction: column;
		align-items: flex-end;
		gap: 4px;
	}

	@media (max-width: 720px) {
		.pv-row.pv-head {
			display: none;
		}

		.pv-row {
			grid-template-columns: 28px minmax(0, 1fr);
			align-items: start;
		}

		.pv-name {
			grid-column: 2 / -1;
		}

		.pv-cells {
			grid-column: 2 / -1;
			display: grid;
			grid-template-columns: repeat(3, minmax(0, 1fr));
			gap: var(--s-2);
		}

		.pv-label {
			display: block;
		}

		.pv-status {
			grid-column: 2 / -1;
			flex-direction: row;
			align-items: center;
		}
	}
</style>
