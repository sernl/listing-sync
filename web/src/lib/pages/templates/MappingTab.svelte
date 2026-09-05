<script lang="ts">
	import { createQueries, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import {
		ApiFailure,
		api,
		type OverrideView,
		type TermView,
		type VocabularyView
	} from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import Panel from '$lib/Panel.svelte';
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
			refusal = failure instanceof ApiFailure ? failure.message : 'That override was not saved.';
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
				failure instanceof ApiFailure ? failure.message : 'That override was not withdrawn.';
		}
	}
</script>

<Banner tone="info" title="Licence is yours alone">
	{LICENCE_REFUSAL}
</Banner>

<Panel
	title="Add an override"
	description="Choose which of a marketplace's own values one of your own words lands in. This applies to your resources alone."
>
	<div class="tpl-grid">
		<Field label="Marketplace" id="{base}-inventory">
			<select
				id="{base}-inventory"
				bind:value={inventory}
				onchange={() => (toNative = '')}
			>
				{#each AUTHORABLE_PLATFORMS as one (one)}
					<option value={one}>{platformTitle(one)}</option>
				{/each}
			</select>
		</Field>

		<Field label="Axis" id="{base}-axis">
			<select
				id="{base}-axis"
				bind:value={axis}
				onchange={() => {
					from = '';
					toNative = '';
				}}
			>
				{#each AXES as one (one)}
					<option value={one}>{AXIS_LABEL[one]}</option>
				{/each}
			</select>
		</Field>

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
			<Field label="Lands in, on {platformTitle(inventory)}" id="{base}-destination">
				<select id="{base}-destination" bind:value={toNative}>
					<option value="">Pick a value</option>
					{#each destinations.values as value (value.id)}
						<option value={value.id}>{value.label}</option>
					{/each}
				</select>
			</Field>
		{:else if destinations.kind === 'unread'}
			<div class="tpl-absent">
				<b>Lands in, on {platformTitle(inventory)}</b>
				<span>What {platformTitle(inventory)} offers has not been read yet.</span>
			</div>
		{:else if destinations.kind === 'unbound'}
			<div class="tpl-absent">
				<b>Lands in, on {platformTitle(inventory)}</b>
				<span>
					{platformTitle(inventory)} has no field of its own for {AXIS_LABEL[
						axis
					].toLowerCase()}, so there is nothing to choose here.
				</span>
			</div>
		{:else}
			<div class="tpl-absent">
				<b>Lands in, on {platformTitle(inventory)}</b>
				<span>
					{platformTitle(inventory)} publishes no list of values for {AXIS_LABEL[
						axis
					].toLowerCase()}, so there is nothing to pick from here.
				</span>
			</div>
		{/if}
	</div>

	<fieldset class="tpl-choices">
		<legend>How it reads</legend>
		<div class="picks">
			<label>
				<input
					type="radio"
					name="{base}-kind"
					checked={kind === 'exact'}
					onchange={() => (kind = 'exact')}
				/>
				Same as
			</label>
			<label>
				<input
					type="radio"
					name="{base}-kind"
					checked={kind === 'broader'}
					onchange={() => (kind = 'broader')}
				/>
				Belongs under
			</label>
		</div>
	</fieldset>
	<p class="tpl-note">
		Belongs under: buyers on this marketplace see the heading, not your own term.
	</p>

	{#if refusal !== null}
		<p class="tpl-refusal">{refusal}</p>
	{/if}

	<p class="tpl-note">
		Saving again replaces your answer for the same marketplace, field and term.
	</p>

	<div class="tpl-actions">
		<Button
			tier="additive"
			icon="circle-plus"
			disabled={saving || !complete}
			reason={complete
				? saving
					? 'Saving now.'
					: undefined
				: 'Pick one of your terms and the value it lands in.'}
			onclick={() => void save()}
		>
			{saving
				? 'Saving…'
				: chosenTerm === undefined
					? 'Save override'
					: `Save override for ${chosenTerm.label}`}
		</Button>
	</div>
</Panel>

<Panel title="Your overrides">
	{#each byMarketplace as group (group.inventory)}
		<div class="tpl-group">
			<h3>{platformTitle(group.inventory)}</h3>
			{#each group.rows as row (`${row.axis}:${row.from_term}`)}
				<div class="tpl-row">
					<span class="who">
						<span class="t">{termsByAxis.label.get(row.from_term) ?? row.from_term}</span>
						<span class="meta">
							{AXIS_LABEL[row.axis]} · lands in {row.segments.join(' › ')}{row.kind === 'broader'
								? ' · belongs under'
								: ''}
						</span>
					</span>
					<span class="tpl-row-acts">
						<Button small onclick={() => change(row)}>Change</Button>
						<Button small danger onclick={() => void withdraw(row)}>Remove</Button>
					</span>
				</div>
			{/each}
		</div>
	{:else}
		<p class="tpl-none">
			{overrides.isPending
				? 'Reading your overrides…'
				: overrides.isError
					? 'Your overrides could not be read.'
					: 'No overrides here yet.'}
		</p>
	{/each}
</Panel>
