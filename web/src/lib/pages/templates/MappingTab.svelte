<script lang="ts">
	// Marketplace words as a guided flow: where (the marketplace and the
	// field), then the match — your term on the left, the marketplace's option
	// on the right — with Save in the step's sticky footer. Saved matches
	// below, each drawn as the same pair of term chips.
	import { createQueries, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import {
		ApiFailure,
		api,
		type OverrideView,
		type TermView,
		type VocabularyView
	} from '$lib/api';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import Explain from '$lib/Explain.svelte';
	import FlowDiagram from '$lib/FlowDiagram.svelte';
	import FlowStep from '$lib/FlowStep.svelte';
	import Stepper, { type StepMark } from '$lib/Stepper.svelte';
	import { SHORT_NAME } from '$lib/platforms';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import { AUTHORABLE_PLATFORMS, platformTitle } from '$lib/platforms';
	import { queryKeys } from '$lib/query';
	import { AXES, AXIS_LABEL, draftOf, isComplete } from '$lib/templates';
	import { chosenOf, destinationsOf } from './destinations';
	import type { InventoryId, TermKind } from '$lib/generated/vocab';
	import { LICENCE_REFUSAL } from './tabs';

	let inventory = $state<InventoryId>(AUTHORABLE_PLATFORMS[0]);
	let axis = $state<TermKind>(AXES[0]);
	let from = $state('');
	let toNative = $state('');
	let kind = $state<'exact' | 'broader'>('exact');

	const base = $props.id();

	// The canonical terms of the chosen axis, which is what an override maps
	// from. Served already, so the picker is real rather than a placeholder.
	const terms = createQuery(() => ({
		queryKey: queryKeys.taxonomyTerms(axis),
		queryFn: () => api.terms(axis).then((view) => view.terms),
		staleTime: Infinity
	}));

	// Each authorable marketplace's own vocabulary, which is what an override
	// maps to. One read per marketplace, cached: it is the registry on the wire
	// and changes only when the server does.
	//
	// The map travels inside an object rather than as the combined result
	// itself: `createQueries` copies the result's own enumerable keys onto a
	// reactive proxy (`createRawRef` in @tanstack/svelte-query), and a Map has
	// none, so a bare Map arrives at the component as an empty object and the
	// first `.get` throws.
	//
	// The query list comes from a constant, so it is complete on the first
	// render and waits on nothing this page reads.
	const vocabularies = createQueries(() => ({
		queries: AUTHORABLE_PLATFORMS.map((one: InventoryId) => ({
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

	const destinations = $derived(destinationsOf(vocabularies.known.get(inventory), axis));

	/** The destination a save may name. Never the raw `toNative`: switching the
	 *  marketplace or the axis re-points the picker at another native field,
	 *  and the identifier already chosen stays a non-empty string that would
	 *  otherwise be written as the destination of a field it does not belong
	 *  to. The selects clear it too, but this is what makes that unnecessary. */
	const chosen = $derived(chosenOf(destinations, toNative));

	const chosenTerm = $derived((terms.data ?? []).find((term: TermView) => term.id === from));

	const queryClient = useQueryClient();

	const overrides = createQuery(() => ({
		queryKey: queryKeys.overrides,
		queryFn: () => api.overrides().then((view) => view.overrides)
	}));

	// Every overridable axis's terms, so a saved override renders the word the
	// seller chose rather than the identifier it is stored under. The form
	// reads one axis; this reads all four, and both share the cache. The query
	// list comes from a constant, so it is complete on the first render and
	// waits on nothing this page reads.
	const termsByAxis = createQueries(() => ({
		queries: AXES.map((one: TermKind) => ({
			queryKey: queryKeys.taxonomyTerms(one),
			queryFn: () => api.terms(one).then((view) => view.terms),
			staleTime: Infinity
		})),
		combine: (results: { data?: TermView[] }[]) => ({
			label: new Map(
				results.flatMap((result) => result.data ?? []).map((term) => [term.id, term.label])
			)
		})
	}));

	const byMarketplace = $derived(
		AUTHORABLE_PLATFORMS.map((one: InventoryId) => ({
			inventory: one,
			rows: (overrides.data ?? []).filter((row: OverrideView) => row.inventory === one)
		})).filter((group) => group.rows.length > 0)
	);

	const complete = $derived(isComplete({ inventory, axis, from, toNative: chosen, kind }));

	/** Loads a saved override into the form. Editing is the same write as
	 *  adding, because the server upserts on the marketplace, axis and term
	 *  together, so saving what this loads replaces the row rather than adding
	 *  a second beside it. */
	function change(row: OverrideView) {
		const draft = draftOf(row);
		inventory = draft.inventory;
		axis = draft.axis;
		from = draft.from;
		toNative = draft.toNative;
		kind = draft.kind;
		refusal = null;
		document.getElementById('step-map-match')?.scrollIntoView({ behavior: 'smooth', block: 'start' });
	}

	let saving = $state(false);
	let refusal = $state<string | null>(null);

	async function save() {
		if (!complete) {
			return;
		}
		saving = true;
		refusal = null;
		try {
			await api.setOverride({
				inventory,
				axis,
				from_term: from,
				to: { segments: [destinationLabel()], native_id: chosen },
				kind
			});
			await queryClient.invalidateQueries({ queryKey: queryKeys.overrides });
			from = '';
			toNative = '';
		} catch (failure) {
			refusal = failure instanceof ApiFailure ? failure.message : 'We couldn’t save that match.';
		} finally {
			saving = false;
		}
	}

	/** The words the marketplace shows for the value chosen, which is what the
	 *  override's path segments carry: the identifier travels beside them in
	 *  `native_id`, and the segments are what a person reads. */
	function destinationLabel(): string {
		if (destinations.kind !== 'values') {
			return chosen;
		}
		return destinations.values.find((value) => value.id === chosen)?.label ?? chosen;
	}

	async function withdraw(row: OverrideView) {
		refusal = null;
		try {
			await api.withdrawOverride({
				inventory: row.inventory,
				axis: row.axis,
				from_term: row.from_term
			});
			await queryClient.invalidateQueries({ queryKey: queryKeys.overrides });
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure ? failure.message : 'We couldn’t remove that match.';
		}
	}

	const KIND_WORD = { exact: 'same as', broader: 'belongs under' } as const;

	const destinationWord = $derived(chosen === '' ? null : destinationLabel());
	const termWord = $derived(chosenTerm?.label ?? null);

	let whereOpen = $state(true);
	let matchOpen = $state(true);

	const steps = $derived<StepMark[]>([
		{ id: 'map-where', label: 'Where', done: true },
		{ id: 'map-match', label: 'Match', done: complete }
	]);
</script>

<Stepper {steps} label="Match steps" />

<div class="flow">
	<FlowStep
		n={1}
		id="map-where"
		title="Where"
		hint="Choose the marketplace and the field."
		summary="{SHORT_NAME[inventory]} · {AXIS_LABEL[axis]}"
		done
		bind:open={whereOpen}
	>
		{#snippet aside()}
			<Explain title="What a match does" label="">
				<p>
					A match says which of a marketplace’s own options one of your words should use. When
					you publish there, the resource carries that option.
				</p>
				<p>{LICENCE_REFUSAL}</p>
			</Explain>
		{/snippet}

		<div class="flow-choice" role="radiogroup" aria-label="Marketplace">
			{#each AUTHORABLE_PLATFORMS as one (one)}
				<button
					type="button"
					role="radio"
					aria-checked={inventory === one}
					onclick={() => {
						inventory = one;
						toNative = '';
					}}
				>
					<span class="tpl-mk-line"><MarketplaceMark inventory={one} size={20} />{platformTitle(one)}</span>
				</button>
			{/each}
		</div>

		<div class="chip-row" role="radiogroup" aria-label="Field">
			{#each AXES as one (one)}
				<button
					type="button"
					role="radio"
					class="choice-chip solo tpl-pick"
					aria-checked={axis === one}
					onclick={() => {
						axis = one;
						from = '';
						toNative = '';
					}}
				>
					{AXIS_LABEL[one]}
				</button>
			{/each}
		</div>
	</FlowStep>

	<FlowStep
		n={2}
		id="map-match"
		title="Match"
		hint="Pick your term, then the option it uses."
		summary={termWord === null ? 'Not chosen' : `${termWord} → ${destinationWord ?? '?'}`}
		done={complete}
		bind:open={matchOpen}
		footer={saveFooter}
	>
		{#snippet aside()}
			<Explain title="Same as, or belongs under" label="">
				<p>Same as: buyers on this marketplace see your term as that option.</p>
				<p>Belongs under: buyers on this marketplace see the broader heading, not your term.</p>
				<p>Saving again replaces your choice for the same marketplace, field and term.</p>
			</Explain>
		{/snippet}

		<div class="tpl-grid">
			<Field label="Your term" id="{base}-term">
				<select id="{base}-term" bind:value={from} disabled={terms.isPending}>
					<option value="">
						{terms.isPending ? 'Loading your terms…' : 'Pick one of your terms'}
					</option>
					{#each terms.data ?? [] as term (term.id)}
						<option value={term.id}>{term.label}</option>
					{/each}
				</select>
			</Field>

			{#if destinations.kind === 'values'}
				<Field label="Use on {platformTitle(inventory)}" id="{base}-destination">
					<select id="{base}-destination" bind:value={toNative}>
						<option value="">Pick an option</option>
						{#each destinations.values as value (value.id)}
							<option value={value.id}>{value.label}</option>
						{/each}
					</select>
				</Field>
			{:else}
				<div class="tpl-absent">
					<b>Use on {platformTitle(inventory)}</b>
					<span>
						{#if destinations.kind === 'unread'}
							{platformTitle(inventory)}’s options haven’t loaded yet.
						{:else if destinations.kind === 'unbound'}
							{platformTitle(inventory)} has no {AXIS_LABEL[axis].toLowerCase()} field to choose from.
						{:else}
							{platformTitle(inventory)} has no list of {AXIS_LABEL[axis].toLowerCase()} options to pick from.
						{/if}
					</span>
				</div>
			{/if}
		</div>

		<div class="flow-choice" role="radiogroup" aria-label="How it matches">
			<button type="button" role="radio" aria-checked={kind === 'exact'} onclick={() => (kind = 'exact')}>
				Same as
				<span class="sub">Your term, as that option.</span>
			</button>
			<button type="button" role="radio" aria-checked={kind === 'broader'} onclick={() => (kind = 'broader')}>
				Belongs under
				<span class="sub">Shown under the broader heading.</span>
			</button>
		</div>

		<FlowDiagram
			from={{ icon: 'tag', label: 'Your term' }}
			to={[{ inventory }]}
			rule={termWord === null ? null : KIND_WORD[kind]}
			empty="pick a term"
			pairs={termWord === null ? [] : [{ from: [termWord], to: [destinationWord ?? '?'] }]}
			label="{termWord ?? 'Your term'} {KIND_WORD[kind]} {destinationWord ?? 'an option'} on {SHORT_NAME[inventory]}"
		/>

		{#if refusal !== null}
			<p class="tpl-refusal">{refusal}</p>
		{/if}
	</FlowStep>

	<section class="flow-section" aria-labelledby="{base}-matches">
		<div class="flow-section-head">
			<h2 id="{base}-matches">Your matches</h2>
		</div>
		{#each byMarketplace as group (group.inventory)}
			<div class="flow-card">
				<div class="flow-card-head">
					<h3 class="flow-label tpl-mk-line">
						<MarketplaceMark inventory={group.inventory} size={18} />{platformTitle(group.inventory)}
					</h3>
				</div>
				<ul class="tpl-matches">
					{#each group.rows as row (`${row.axis}:${row.from_term}`)}
						<li>
							<span class="tpl-axis">{AXIS_LABEL[row.axis]}</span>
							<span class="term-chip from">{termsByAxis.label.get(row.from_term) ?? row.from_term}</span>
							<span class="tpl-arrow" aria-label={KIND_WORD[row.kind]}>{row.kind === 'broader' ? '⊂' : '→'}</span>
							<span class="term-chip to">{row.segments.join(' › ')}</span>
							<span class="tpl-row-acts">
								<Button small tier="quiet" icon="pencil" onclick={() => change(row)}>Change</Button>
								<Button small tier="quiet" danger icon="trash-2" onclick={() => void withdraw(row)}>Remove</Button>
							</span>
						</li>
					{/each}
				</ul>
			</div>
		{:else}
			<p class="tpl-none">
				{overrides.isPending
					? 'Loading your matches…'
					: overrides.isError
						? 'We couldn’t load your matches.'
						: 'No matches yet.'}
			</p>
		{/each}
	</section>
</div>

{#snippet saveFooter()}
	<Button
		tier="primary"
		icon="circle-plus"
		disabled={saving || !complete}
		reason={complete
			? saving
				? 'Saving now.'
				: undefined
			: 'Pick your term and the marketplace option it matches.'}
		onclick={() => void save()}
	>
		{saving ? 'Saving…' : chosenTerm === undefined ? 'Save match' : `Save match for ${chosenTerm.label}`}
	</Button>
{/snippet}
