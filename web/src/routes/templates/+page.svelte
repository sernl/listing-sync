<script lang="ts">
	import { createQueries, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import {
		ApiFailure,
		api,
		type OverrideView,
		type TermView,
		type VocabularyView
	} from '$lib/api';
	import { AUTHORABLE_PLATFORMS, platformTitle } from '$lib/platforms';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { queryKeys } from '$lib/query';
	import { AXES, AXIS_LABEL, draftOf, isComplete } from '$lib/templates';
	import type { InventoryId, TermKind } from '$lib/generated/vocab';

	let inventory = $state<InventoryId>(AUTHORABLE_PLATFORMS[0]);
	let axis = $state<TermKind>(AXES[0]);
	let from = $state('');
	let toNative = $state('');
	let kind = $state<'exact' | 'broader'>('exact');

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
	const vocabularies = createQueries(() => ({
		queries: AUTHORABLE_PLATFORMS.map((one: InventoryId) => ({
			queryKey: queryKeys.vocabulary(one),
			queryFn: () => api.vocabulary(one),
			staleTime: Infinity
		})),
		combine: (results: { data?: VocabularyView }[]) =>
			new Map(
				results.flatMap((result) =>
					result.data === undefined
						? []
						: [[result.data.inventory, result.data] as [InventoryId, VocabularyView]]
				)
			)
	}));

	/** The values this marketplace offers on the chosen axis.
	 *
	 *  An axis lands in one native field, and a closed field carries its own
	 *  value list; anything else is a field whose values this server has not
	 *  captured, so the picker offers none rather than inventing them. */
	const destinations = $derived.by(() => {
		const vocabulary = vocabularies.get(inventory);
		const binding = vocabulary?.axes.find((one) => one.axis === axis);
		if (vocabulary === undefined || binding === undefined) {
			return [];
		}
		return vocabulary.natives.find((native) => native.name === binding.native)?.values ?? [];
	});

	const chosenTerm = $derived(
		(terms.data ?? []).find((term: TermView) => term.id === from)
	);

	const queryClient = useQueryClient();

	const overrides = createQuery(() => ({
		queryKey: queryKeys.overrides,
		queryFn: () => api.overrides().then((view) => view.overrides)
	}));

	// Every overridable axis's terms, so a saved override renders the word the
	// seller chose rather than the identifier it is stored under. The form
	// reads one axis; this reads all four, and both share the cache.
	const termsByAxis = createQueries(() => ({
		queries: AXES.map((one: TermKind) => ({
			queryKey: queryKeys.taxonomyTerms(one),
			queryFn: () => api.terms(one).then((view) => view.terms),
			staleTime: Infinity
		})),
		combine: (results: { data?: TermView[] }[]) =>
			new Map(
				results.flatMap((result) => result.data ?? []).map((term) => [term.id, term.label])
			)
	}));

	const byMarketplace = $derived(
		AUTHORABLE_PLATFORMS.map((one: InventoryId) => ({
			inventory: one,
			rows: (overrides.data ?? []).filter((row: OverrideView) => row.inventory === one)
		}))
	);

	const complete = $derived(isComplete({ inventory, axis, from, toNative, kind }));

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
				to: { segments: [destinationLabel()], native_id: toNative },
				kind
			});
			await queryClient.invalidateQueries({ queryKey: queryKeys.overrides });
			from = '';
			toNative = '';
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure ? failure.message : 'That override was not saved.';
		} finally {
			saving = false;
		}
	}

	/** The words the marketplace shows for the value chosen, which is what the
	 *  override's path segments carry: the identifier travels beside them in
	 *  `native_id`, and the segments are what a person reads. */
	function destinationLabel(): string {
		return destinations.find((value) => value.id === toNative)?.label ?? toNative;
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
			failure instanceof ApiFailure
				? (refusal = failure.message)
				: (refusal = 'That override was not withdrawn.');
		}
	}
</script>

<div class="page">
	<PageHead
		icon="❏"
		title="Templates"
		description="Your own answers to how this catalogue's words map onto each marketplace's."
	/>

	<Panel
		title="What these do"
		description="An override tells us which of a marketplace's own values one of your subjects, topics, resource types or phases should land in. It applies to your catalogue alone and reaches no one else."
	>
		<p class="foot-note">
			This is not Reconciliation. That screen answers gaps in the shared mapping, once, for
			everybody; an override is your own answer for your own catalogue, and it is neither global
			nor a gap. Licence is not overridable here, because a licence is a legal statement about
			your work rather than a mapping choice.
		</p>
	</Panel>

	<Panel
		title="Add an override"
		description="Pick one of your own terms, and the value it should land in on that marketplace."
	>
		<div class="form-row">
			<label class="field">
				<span class="label">Marketplace</span>
				<select bind:value={inventory}>
					{#each AUTHORABLE_PLATFORMS as one (one)}
						<option value={one}>{platformTitle(one)}</option>
					{/each}
				</select>
			</label>

			<label class="field">
				<span class="label">Axis</span>
				<select bind:value={axis} onchange={() => (from = '')}>
					{#each AXES as one (one)}
						<option value={one}>{AXIS_LABEL[one]}</option>
					{/each}
				</select>
			</label>
		</div>

		<label class="field">
			<span class="label">Your term</span>
			<select bind:value={from} disabled={terms.isPending}>
				<option value="">
					{terms.isPending ? 'Loading your terms…' : 'Pick one of your terms'}
				</option>
				{#each terms.data ?? [] as term (term.id)}
					<option value={term.id}>{term.label}</option>
				{/each}
			</select>
		</label>

		<label class="field">
			<span class="label">Lands in, on {platformTitle(inventory)}</span>
			{#if destinations.length === 0}
				<span class="hint">
					This marketplace publishes no value list for {AXIS_LABEL[axis].toLowerCase()}, so there
					is nothing here to map onto. Nothing is offered rather than a free-text box that would
					be guessed at.
				</span>
			{:else}
				<select bind:value={toNative}>
					<option value="">Pick a value</option>
					{#each destinations as value (value.id)}
						<option value={value.id}>{value.label}</option>
					{/each}
				</select>
			{/if}
		</label>

		<div class="inline-choices">
			<label>
				<input
					type="radio"
					name="override-kind"
					checked={kind === 'exact'}
					onchange={() => (kind = 'exact')}
				/>
				Same as
			</label>
			<label>
				<input
					type="radio"
					name="override-kind"
					checked={kind === 'broader'}
					onchange={() => (kind = 'broader')}
				/>
				Belongs under
			</label>
		</div>
		<p class="foot-note">
			Belongs under: buyers on this marketplace see the heading, not your term. The difference is
			recorded either way, so a later reader knows whether the mapping matched cleanly or lost
			detail.
		</p>

		{#if refusal !== null}
			<p class="refusal">{refusal}</p>
		{/if}

		<p class="foot-note">
			Changing an override is saving it again: one marketplace, axis and term hold one answer, so
			a save replaces what is there rather than adding a second beside it.
		</p>

		<div class="actions">
			<button class="cta" type="button" disabled={saving || !complete} onclick={() => void save()}>
				{saving
					? 'Saving…'
					: chosenTerm === undefined
						? 'Save override'
						: `Save override for ${chosenTerm.label}`}
			</button>
		</div>
	</Panel>

	{#each byMarketplace as group (group.inventory)}
		<Panel title={platformTitle(group.inventory)}>
			{#each group.rows as row (`${row.axis}:${row.from_term}`)}
				<div class="row">
					<span class="what">
						<span class="t">{termsByAxis.get(row.from_term) ?? row.from_term}</span>
						<span class="s">
							{AXIS_LABEL[row.axis]} · lands in {row.segments.join(' › ')}
							{row.kind === 'broader' ? ' · belongs under' : ''}
						</span>
					</span>
					<span class="grow"></span>
					<button class="btn small" type="button" onclick={() => change(row)}>Change</button>
					<button class="btn small danger" type="button" onclick={() => void withdraw(row)}>
						Remove
					</button>
				</div>
			{:else}
				<p class="quiet">
					{overrides.isPending
						? 'Reading your overrides…'
						: overrides.isError
							? 'Your overrides could not be read.'
							: 'No overrides here yet.'}
				</p>
			{/each}
		</Panel>
	{/each}
</div>

<style>
	.form-row {
		display: flex;
		gap: 12px;
		flex-wrap: wrap;
	}

	.form-row .field {
		flex: 1 1 220px;
	}
</style>
