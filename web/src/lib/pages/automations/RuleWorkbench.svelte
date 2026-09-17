<script lang="ts">
	import { createQueries, createQuery } from '@tanstack/svelte-query';
	import { onMount, untrack } from 'svelte';
	import { page } from '$app/state';
	import { ApiFailure, api, type ProductHead, type TermView, type VocabularyView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import { external } from '$lib/external';
	import type { InventoryId, TermKind } from '$lib/generated/vocab';
	import { INVENTORY_ORDER, formatPrice, normaliseQuery } from '$lib/listings-view';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import { productsFromUrl } from '$lib/migration-plan';
	import Pagination from '$lib/Pagination.svelte';
	import Panel from '$lib/Panel.svelte';
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
	const kindWord = $derived(kind === 'pricing' ? 'pricing rule' : 'mapping rule');
</script>

<div class="rule-page">
	<Panel title="Direction" description="Which marketplace a rule reads, and which one it writes.">
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
			<p class="pair-refusal">{pairRefusal}</p>
		{/if}
		{#if kind === 'mapping' && licenceRefusal !== null}
			<p class="pair-refusal">{licenceRefusal}</p>
		{/if}
	</Panel>

	<Panel title="Your rules" description="Every {kindWord} you have written for this direction.">
		{#snippet more()}
			<Button
				tier="primary"
				small
				onclick={() =>
					open({
						rule: null,
						revision: null,
						definition: blankDefinition(kind, source, target),
						preset: null
					})}
			>
				New rule
			</Button>
		{/snippet}

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
			<span class="pick-count" role="status" aria-live="polite">
				{counts === null ? 'Totals not read yet' : `${counts.enabled} on, ${counts.disabled} off`}
			</span>
		</div>

		{#if savedNote !== null}
			<Banner tone="ok" onDismiss={() => (savedNote = null)}>{savedNote}</Banner>
		{/if}
		{#if removeFailure !== null}
			<Banner tone="bad">{removeFailure}</Banner>
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
				<p class="quiet">
					{rulePage > 1
						? 'There is nothing left on this page. Go back for the rules before it.'
						: normaliseQuery(box).length > 0
							? 'No rule matches that search. Clear it to see the rest.'
							: listState !== 'all'
								? `No ${kindWord} for this direction is ${listState === 'enabled' ? 'on' : 'off'}. Show All to see the rest.`
								: `You have written no ${kindWord} for ${SHORT_NAME[source]} → ${SHORT_NAME[target]} yet. New rule starts one, or load a preset below.`}
				</p>
			{:else}
				{#each rules as row (row.id)}
					<div class="auto-row rule-row">
						{#if scopeKind === 'chosen'}
							<span class="pick">
								<input
									type="checkbox"
									checked={chosenRules.has(row.id)}
									aria-label={`Preview with ${row.definition.title}`}
									onchange={(event) => chooseRule(row.id, event.currentTarget.checked)}
								/>
							</span>
						{/if}
						<span class="who">
							<span class="t">{row.definition.title}</span>
							{#if row.definition.description.length > 0}
								<span class="meta">{row.definition.description}</span>
							{/if}
							<span class="meta">{ruleMeta(row.definition)}</span>
						</span>
						<span class="mark">
							<StatusPill
								tone={row.definition.enabled ? 'ok' : 'soon'}
								label={row.definition.enabled ? 'On' : 'Off'}
							/>
							<MarketplaceMark inventory={row.definition.source} size={16} />
							<MarketplaceMark inventory={row.definition.target} size={16} />
						</span>
						<span class="act rule-acts">
							<Button
								small
								onclick={() =>
									open({
										rule: row.id,
										revision: row.revision,
										definition: structuredClone(row.definition),
										preset: null
									})}
							>
								Edit
							</Button>
							<Button
								small
								tier="quiet"
								onclick={() =>
									open({
										rule: null,
										revision: null,
										definition: duplicateOf(row.definition),
										preset: null
									})}
							>
								Duplicate
							</Button>
							<Button
								small
								danger
								disabled={removing === row.id}
								reason={removing === row.id ? 'This rule is being deleted.' : undefined}
								onclick={() => void remove(row)}
							>
								Delete
							</Button>
						</span>
					</div>
				{/each}
			{/if}
			{#if rules.length > 0 || rulePage > 1}
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
	</Panel>

	{#if editing !== null && draft !== null}
		<Panel
			title={editing.rule === null ? `New ${kindWord}` : `Editing “${draft.title}”`}
			description="What this rule matches, and what it proposes on the target."
		>
			{#if editing.preset !== null}
				<Banner tone="info" title="Loaded from a preset">
					{editing.preset.notice}
					{PRESETS_ARE_SUGGESTIONS}
				</Banner>
			{/if}

			<div class="set-grid">
				<Field label="Name" id="{base}-title">
					<input id="{base}-title" type="text" bind:value={draft.title} />
				</Field>
				<Field label="On or off" id="{base}-enabled">
					<label class="set-toggle">
						<input id="{base}-enabled" type="checkbox" bind:checked={draft.enabled} />
						<span>{draft.enabled ? 'On' : 'Off'}</span>
					</label>
				</Field>
			</div>

			<Field
				label="What this rule is for"
				id="{base}-description"
				hint="Shown beside every figure this rule proposes, so a price can be traced to your own words."
			>
				<textarea id="{base}-description" rows="2" bind:value={draft.description}></textarea>
			</Field>

			<h3 class="rule-heading">What it matches</h3>
			<div class="set-grid">
				<Field label="Source price" id="{base}-pricing">
					<select id="{base}-pricing" bind:value={draft.conditions.pricing}>
						{#each PRICING_CONDITIONS as one (one)}
							<option value={one}>{PRICING_CONDITION_WORD[one]}</option>
						{/each}
					</select>
				</Field>
				<Field
					label="Words in the description"
					id="{base}-keywords"
					hint="Separated by commas and matched as plain text, case-insensitively. Any HTML in a source description is stripped before matching, so a tag or class name never matches."
				>
					<input
						id="{base}-keywords"
						type="text"
						value={keywordBox}
						oninput={(event) => setKeywords(event.currentTarget.value)}
					/>
				</Field>
			</div>
			{#if draft.conditions.keywords.length > 1}
				<Field label="The keywords must match" id="{base}-keyword-mode">
					<select id="{base}-keyword-mode" bind:value={draft.conditions.keyword_mode}>
						<option value="any">Any one of them</option>
						<option value="all">All of them</option>
					</select>
				</Field>
			{/if}

			<Field
				label="Source resource types"
				id="{base}-source-types"
				hint="{SHORT_NAME[draft.source]}’s own types, by the values your import preserved."
			>
				{#if sourceTypes.kind === 'unread'}
					<p class="quiet" id="{base}-source-types">
						{SHORT_NAME[draft.source]}’s vocabulary has not been read yet, so there is nothing
						to tick.
					</p>
				{:else if sourceTypes.kind === 'unbound'}
					<p class="quiet" id="{base}-source-types">
						{SHORT_NAME[draft.source]} has no resource-type field of its own, so a rule cannot
						match on one.
					</p>
				{:else if sourceTypes.kind === 'open'}
					<p class="quiet" id="{base}-source-types">
						Teachouse has captured no list of {SHORT_NAME[draft.source]}’s resource types, so
						there is nothing to pick from. Use the keyword or attribute conditions instead.
					</p>
				{:else}
					<div class="rule-values" id="{base}-source-types">
						{#each sourceTypes.values as value (value.id)}
							<label class="rule-value">
								<input
									type="checkbox"
									checked={draft.conditions.resource_types.includes(value.id)}
									onchange={(event) => toggleSourceType(value.id, event.currentTarget.checked)}
								/>
								{value.label}
							</label>
						{/each}
					</div>
				{/if}
			</Field>

			{#each draft.conditions.attributes as condition, index (index)}
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
					<p class="foot-note">{AXIS_MATCH_NOTE[condition.axis]}</p>
					{#if choices.length === 0}
						<p class="quiet">
							No measured values for {AXIS_WORD[condition.axis].toLowerCase()} have been read,
							so this condition can name none. Remove it, or match on something else.
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
					<Button small tier="quiet" onclick={() => dropCondition(index)}>
						Remove this condition
					</Button>
				</div>
			{/each}
			<div class="set-foot">
				<Button small tier="outline" onclick={addCondition}>Add an attribute condition</Button>
				<p class="foot-note">
					Every condition has to hold. Leave them all empty and the rule matches every resource
					on {SHORT_NAME[draft.source]}.
				</p>
			</div>

			<h3 class="rule-heading">What it proposes</h3>
			{#if draft.action.kind === 'pricing'}
				<div class="set-grid">
					<Field
						label="Rate"
						id="{base}-rate"
						hint="Multiplied by the source price on the server, never in this browser."
					>
						<input
							id="{base}-rate"
							type="text"
							inputmode="decimal"
							value={draft.action.rate}
							oninput={(event) => setManualRate(event.currentTarget.value)}
						/>
					</Field>
					<Field label="Taken to" id="{base}-rounding">
						<select id="{base}-rounding" bind:value={draft.action.rounding}>
							{#each ROUNDINGS as one (one)}
								<option value={one}>{ROUNDING_WORD[one]}</option>
							{/each}
						</select>
					</Field>
				</div>
				<p class="foot-note">{ROUNDING_LINE[draft.action.rounding]}</p>
				<div class="set-foot">
					<Button small tier="outline" onclick={() => setManualRate(MANUAL_RATE)}>
						Use the {MANUAL_RATE} estimate
					</Button>
					<Button
						small
						tier="outline"
						disabled={referencing}
						reason={referencing ? 'The quote is being read.' : undefined}
						onclick={() => void quote()}
					>
						{referencing ? 'Asking…' : 'Get the ECB reference rate'}
					</Button>
				</div>
				<p class="foot-note">{MANUAL_RATE_NOTE}</p>
				<p class="foot-note">{REFERENCE_NOTE}</p>
				{#if quoteLine !== null}
					<Banner tone="info" title="Rate taken from a stored quote">{quoteLine}</Banner>
				{:else if draft.action.reference !== null}
					<p class="foot-note">
						This rate cites a stored quote, so the figure your prices were converted at can be
						shown and re-checked later.
					</p>
				{/if}
				{#if referenceFailure !== null}
					<Banner tone="bad" title="No rate was filled in">{referenceFailure}</Banner>
				{/if}
			{:else}
				<Field label="Licence on {SHORT_NAME[draft.target]}" id="{base}-licence">
					{#if licenceRefusal !== null}
						<p class="quiet" id="{base}-licence">{licenceRefusal}</p>
					{:else if targetLicences === null}
						<p class="quiet" id="{base}-licence">
							{SHORT_NAME[draft.target]}’s licence values have not been read yet, so there is
							nothing to choose.
						</p>
					{:else}
						<select
							id="{base}-licence"
							value={draft.action.licence ?? ''}
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
				{#if freeGrantChosen(draft.action.licence, targetLicences)}
					<Banner tone="warn" title="A free grant is not the marketplace’s licence">
						{FREE_GRANT_WARNING}
					</Banner>
				{/if}

				<Field label="Resource type on {SHORT_NAME[draft.target]}" id="{base}-type">
					{#if targetTypes.kind === 'values'}
						<select
							id="{base}-type"
							value={draft.action.resource_type ?? ''}
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
								? `${SHORT_NAME[draft.target]}’s vocabulary has not been read yet.`
								: targetTypes.kind === 'unbound'
									? `${SHORT_NAME[draft.target]} has no resource-type field of its own.`
									: `Teachouse has captured no list of ${SHORT_NAME[draft.target]}’s resource types, so there is nothing to choose.`}
						</p>
					{/if}
				</Field>
			{/if}

			<h3 class="rule-heading">Apply automatically</h3>
			<div class="rule-values rule-uses">
				{#each RULE_USES as use (use)}
					<label class="rule-value rule-use">
						<input
							type="checkbox"
							checked={draft.auto_apply.includes(use)}
							onchange={(event) => toggleUse(use, event.currentTarget.checked)}
						/>
						<span>
							<strong>{USE_WORD[use]}</strong>
							<span class="meta">{USE_LINE[use]}</span>
						</span>
					</label>
				{/each}
			</div>
			<p class="foot-note">{NO_AUTO_APPLY}</p>

			<div class="set-foot">
				<Button
					tier="primary"
					disabled={draftRefusal !== null || saving}
					reason={draftRefusal ?? (saving ? 'The rule is being saved.' : undefined)}
					onclick={() => void save()}
				>
					{saving ? 'Saving…' : editing.rule === null ? 'Save this rule' : 'Save changes'}
				</Button>
				<Button tier="quiet" onclick={close}>Cancel</Button>
				{#if draftRefusal !== null}
					<p class="foot-note">{draftRefusal}</p>
				{/if}
			</div>
			{#if saveFailure !== null}
				<Banner tone="bad" title="Save not confirmed">{saveFailure}</Banner>
			{/if}
		</Panel>
	{/if}

	<Panel
		title="Presets"
		description="Suggestions Teachouse has evidence for. Loading one changes nothing."
	>
		<Banner tone="info">{PRESETS_ARE_SUGGESTIONS}</Banner>
		{#if presets.isPending}
			<p class="quiet">Loading the suggestions…</p>
		{:else if presets.isError}
			<p class="quiet">
				The presets could not be read, so none are offered here. Writing a rule yourself is
				unaffected.
			</p>
		{:else if offered.length === 0}
			<p class="quiet">
				There is no {kindWord} preset for {SHORT_NAME[source]} → {SHORT_NAME[target]}. One is
				offered only where both marketplaces’ own published terms support it.
			</p>
		{:else}
			{#each offered as preset (preset.id)}
				{@const licence =
					preset.definition.action.kind === 'mapping' ? preset.definition.action.licence : null}
				<div class="rule-preset">
					<div class="rule-preset-head">
						<strong>{preset.definition.title}</strong>
						<Button
							small
							tier="outline"
							onclick={() =>
								open({
									rule: null,
									revision: null,
									definition: structuredClone(preset.definition),
									preset
								})}
						>
							Load into the editor
						</Button>
					</div>
					<p>{preset.definition.description}</p>
					<p class="foot-note">{ruleMeta(preset.definition)}</p>
					<p class="rule-notice">{preset.notice}</p>
					{#if freeGrantChosen(licence, targetLicences)}
						<Banner tone="warn" title="A free grant is not the marketplace’s licence">
							{FREE_GRANT_WARNING}
						</Banner>
					{/if}
					{#if preset.sources.length > 0}
						<p class="foot-note">
							Read from:
							{#each preset.sources as href, index (href)}
								{index > 0 ? ' · ' : ''}<a {href} use:external target="_blank" rel="noreferrer noopener">
									{href}
								</a>
							{/each}
						</p>
					{/if}
				</div>
			{/each}
		{/if}
	</Panel>

	<Panel title="What to preview" description="Which resources, and which rules.">
		<div class="scope-choice" role="radiogroup" aria-label="Which resources">
			<button
				type="button"
				class="disp"
				class:on={all}
				role="radio"
				aria-checked={all}
				onclick={() => (all = true)}
			>
				<span class="disp-word">All resources on {SHORT_NAME[source]}</span>
				<span class="disp-line">
					The catalogue as it stands when the preview is taken, not a selection that moves
					under it.
				</span>
			</button>
			<button
				type="button"
				class="disp"
				class:on={!all}
				role="radio"
				aria-checked={!all}
				onclick={() => (all = false)}
			>
				<span class="disp-word">Choose</span>
				<span class="disp-line">Tick the resources yourself, across as many pages as you like.</span>
			</button>
		</div>

		{#if !all}
			{#if !pickLoaded}
				<p class="quiet">Loading your resources…</p>
			{:else if pickUnread && products.length === 0}
				<Banner tone="bad" action={retryPicks}>
					Your resources could not be read, so there is nothing to tick. Previewing every
					resource on {SHORT_NAME[source]} does not need this list and still works.
				</Banner>
			{:else if products.length === 0 && pickPage === 1}
				<p class="quiet">
					You have no resources yet, so there is nothing to tick. Import brings your existing
					shop across first.
				</p>
			{:else}
				{#if pickUnread}
					<Banner tone="bad" action={retryPicks}>
						That page of your resources could not be read, so the rows below are the last ones
						that did. Nothing you have ticked has been lost.
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
						{ticked.size} chosen{tickedOffPage > 0
							? `, including ${tickedOffPage} not on this page`
							: ''}
					</span>
				</div>
				{#if shown.length === 0}
					<p class="quiet">
						{products.length === 0
							? 'There are no resources on this page. Go back for the ones before it.'
							: 'Nothing on this page matches that. Clear the filter, or try Next for more resources.'}
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
		{/if}

		<div class="scope-choice" role="radiogroup" aria-label="Which rules">
			<button
				type="button"
				class="disp"
				class:on={scopeKind === 'enabled'}
				role="radio"
				aria-checked={scopeKind === 'enabled'}
				onclick={() => (scopeKind = 'enabled')}
			>
				<span class="disp-word">The rules that are on</span>
				<span class="disp-line">Every enabled pricing and mapping rule for this direction.</span>
			</button>
			<button
				type="button"
				class="disp"
				class:on={scopeKind === 'chosen'}
				role="radio"
				aria-checked={scopeKind === 'chosen'}
				onclick={() => (scopeKind = 'chosen')}
			>
				<span class="disp-word">Only the rules I tick</span>
				<span class="disp-line">A tick appears beside each rule in the list above.</span>
			</button>
			<button
				type="button"
				class="disp"
				class:on={scopeKind === 'draft'}
				role="radio"
				aria-checked={scopeKind === 'draft'}
				onclick={() => (scopeKind = 'draft')}
			>
				<span class="disp-word">Only what is in the editor</span>
				<span class="disp-line">
					Previews the rule you are writing on its own, with no saved rule taking part.
				</span>
			</button>
		</div>

			<Field
				label="A rate for this preview only"
				id="{base}-override-rate"
				hint="Resolves the rate for this preview and stays visible in it. Leave it empty to use the rules."
			>
				<input id="{base}-override-rate" type="text" inputmode="decimal" bind:value={rateOverride} />
			</Field>
			{#if overrideRateRefusal !== null}
				<p class="pair-refusal">{overrideRateRefusal}</p>
			{/if}
			<div class="set-grid">
				<Field label="A licence for this preview only" id="{base}-override-licence">
					{#if licenceRefusal !== null}
						<p class="quiet" id="{base}-override-licence">{licenceRefusal}</p>
					{:else if targetLicences === null}
						<p class="quiet" id="{base}-override-licence">
							{SHORT_NAME[target]}’s licence values have not been read yet.
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
				<Field label="A resource type for this preview only" id="{base}-override-type">
					{#if targetTypes.kind === 'values'}
						<select id="{base}-override-type" bind:value={typeOverride}>
							<option value="">Use the rules</option>
							{#each targetTypes.values as value (value.id)}
								<option value={value.id}>{value.label}</option>
							{/each}
						</select>
					{:else}
						<p class="quiet" id="{base}-override-type">
							{SHORT_NAME[target]} offers no captured resource-type list here.
						</p>
					{/if}
				</Field>
			</div>
			{#if freeGrantChosen(licenceOverride.length === 0 ? null : licenceOverride, targetLicences)}
				<Banner tone="warn" title="A free grant is not the marketplace’s licence">
					{FREE_GRANT_WARNING}
				</Banner>
			{/if}
	</Panel>

	<Panel title="Preview and approve" description="Resource by resource, before anything applies.">
		<p class="migrate-lead">{NOTHING_APPLIES_UNTIL_APPROVED}</p>

		<div class="set-foot preview-foot">
			<Button
				tier="outline"
				disabled={previewing || deciding !== null || askRefusal !== null || previewNeedsRead}
				reason={decisionReadRefusal ?? askRefusal ?? (previewing
					? 'The preview is being taken.'
					: deciding !== null ? 'A decision is being sent.' : undefined)}
				onclick={() => void ask()}
			>
				{previewing ? 'Previewing…' : 'Preview'}
			</Button>
			{#if askRefusal !== null}
				<p class="foot-note">{askRefusal}</p>
			{/if}
		</div>

		{#if previewFailure !== null}
			<Banner tone="bad">{previewFailure}</Banner>
		{/if}
		{#if decisionFailure !== null}
			<Banner tone="bad" title="Decision needs attention">{decisionFailure}</Banner>
			{#if previewNeedsRead}
				<div class="set-foot">
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
			<p class="quiet">
				No preview is on screen. Take one and every resource will show what the source carries,
				the current choice for {SHORT_NAME[target]}, and what would be proposed.
			</p>
		{:else if preview.rows.length === 0}
			<p class="quiet">This selection names no resources, so there is nothing to preview.</p>
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
					{picked.size} row{picked.size === 1 ? '' : 's'} selected
				</span>
			</div>

			<div class="plan-rows">
				{#each preview.rows as row (row.product)}
					<div class="plan-row rule-plan-row">
						<span class="pick">
							<input
								type="checkbox"
								checked={picked.has(row.product)}
								disabled={!rejectable(row)}
								title={rejectable(row) ? undefined : 'This row is already decided.'}
								aria-label={`Select ${row.title}`}
								onchange={(event) => pickRow(row.product, event.currentTarget.checked)}
							/>
						</span>
						<span class="plan-title">{row.title}</span>
						<span class="prop-grid">
							<span class="prop-field">
								<span class="prop-label">On {SHORT_NAME[preview.source]}</span>
								<span class="prop-value">{priceText(row.source_price)}</span>
							</span>
							<span class="prop-field">
								<span class="prop-label">Current {SHORT_NAME[preview.target]} choice</span>
								<span class="prop-value">
									{priceText(row.before.price)}
								</span>
								<span class="quiet">
									Licence: {termText(row.before.licence, labels)} · Type: {termText(row.before.resource_type, labels)}
								</span>
							</span>
							<span class="prop-field">
								<span class="prop-label">Proposed</span>
								<span class="prop-value">
									{priceText(row.proposed.price)}
								</span>
								<span class="quiet">
									Licence: {termText(row.proposed.licence, labels)} · Type: {termText(row.proposed.resource_type, labels)}
								</span>
							</span>
						</span>
						<span class="mark">
							<StatusPill tone={STATUS_TONE[row.status]} label={STATUS_WORD[row.status]} />
							<StatusPill tone={DECISION_TONE[row.decision]} label={DECISION_WORD[row.decision]} />
						</span>
						<span class="plan-why">
							{#if row.matches.length > 0}
								{#each row.matches as match (match.id)}
									<span class="block">
										<strong>{match.title}</strong>
										{#if match.description.length > 0}
											— {match.description}
										{/if}
									</span>
								{/each}
							{:else}
								<span class="block">No rule matched this resource.</span>
							{/if}
							{#each row.blockers as blocker (blocker)}
								<span class="block rule-blocker">{blocker}</span>
							{/each}
							{#if !acceptable(row) && row.decision === 'pending'}
								<span class="block rule-blocker">
									A blocked row cannot be approved. Reject it, or fix what it names and preview
									again.
								</span>
							{/if}
						</span>
					</div>
				{/each}
			</div>
			<p class="foot-note">{previewCountsLine(preview.counts)}</p>

			<div class="set-foot">
				<Button
					tier="primary"
					disabled={acceptRefusal !== null || deciding !== null}
					reason={acceptRefusal ?? (deciding !== null ? 'A decision is being sent.' : undefined)}
					onclick={() => void decide('accept', false)}
				>
					Approve the selected rows
				</Button>
				<Button
					disabled={rejectRefusal !== null || deciding !== null}
					reason={rejectRefusal ?? (deciding !== null ? 'A decision is being sent.' : undefined)}
					onclick={() => void decide('reject', false)}
				>
					Reject the selected rows
				</Button>
				<Button
					tier="outline"
					disabled={acceptAllRefusal !== null || deciding !== null}
					reason={acceptAllRefusal ?? (deciding !== null ? 'A decision is being sent.' : undefined)}
					onclick={() => void decide('accept', true)}
				>
					Approve every eligible row
				</Button>
				<Button
					tier="quiet"
					disabled={rejectAllRefusal !== null || deciding !== null}
					reason={rejectAllRefusal ?? (deciding !== null ? 'A decision is being sent.' : undefined)}
					onclick={() => void decide('reject', true)}
				>
					Reject every undecided row
				</Button>
			</div>
			<p class="foot-note">
				“Every eligible row” is every undecided row that is not blocked; a blocked row stays here
				with its reason. Approving the same rows twice is not a second approval.
			</p>
		{/if}
	</Panel>
</div>

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
