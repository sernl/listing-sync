<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api, type FrameworkView } from '$lib/api';
	import type { StandardPick } from '$lib/tpt-form';

	let {
		frameworks,
		chosen,
		onChange
	}: {
		frameworks: readonly FrameworkView[];
		chosen: readonly StandardPick[];
		onChange: (standards: StandardPick[]) => void;
	} = $props();

	// Null until a framework is picked, then the jurisdiction id. Deriving the
	// first framework at initialisation would capture the value the props held
	// on the first render and never follow a vocabulary that arrived after it.
	let picked = $state<number | null>(null);
	let query = $state('');
	const active = $derived(picked ?? frameworks[0]?.jurisdiction_id ?? 0);

	// One read per framework and query, cached: a jurisdiction's tree is the
	// server's own data and does not change while a seller types.
	const found = createQuery(() => ({
		queryKey: ['standards', active, query],
		queryFn: () => api.standardsSearch(active, query),
		enabled: active !== 0
	}));

	/** By the mirror's identifier rather than the code, because a code can name
	 *  several different standards and removing one must not remove its
	 *  namesakes. */
	function drop(sourceGuid: string) {
		onChange(chosen.filter((pick) => pick.source_guid !== sourceGuid));
	}
</script>

<!-- Four jurisdictions, each searched on its own, because no owner publishes a
     cross-framework crosswalk and TPT itself declines to translate between
     them. The catalogue is not ingested yet, and the panel says so rather than
     rendering an empty tree: "nothing matched your words" and "nothing exists
     here yet" are different answers and a seller acts on them differently. -->
<div class="field">
	<div class="tabs" role="tablist" aria-label="Standards framework">
		{#each frameworks as framework (framework.jurisdiction_id)}
			<button
				type="button"
				role="tab"
				class="tab"
				aria-selected={active === framework.jurisdiction_id}
				onclick={() => (picked = framework.jurisdiction_id)}
			>
				{framework.button_label}
			</button>
		{/each}
	</div>

	<input
		type="search"
		placeholder="Search this framework by code or words"
		value={query}
		oninput={(event) => (query = event.currentTarget.value)}
	/>

	{#if found.isPending}
		<p class="quiet">Reading the standards…</p>
	{:else if found.isError}
		<p class="refusal">The standards could not be read. A draft can still be created without one.</p>
	{:else if found.data?.state === 'not_ingested'}
		<div class="attn warn">
			<div class="t">This framework is not loaded yet</div>
			<p>
				{frameworks.find((framework) => framework.jurisdiction_id === active)?.name} has not been
				ingested, so there is nothing here to search — which is different from nothing matching.
				Standards alignment is optional on TPT, so a listing publishes without one and can gain
				one later.
			</p>
		</div>
	{:else if (found.data?.items ?? []).length === 0}
		<p class="quiet">Nothing in this framework matches “{query}”.</p>
	{:else}
		<div class="pick-list">
			{#each found.data?.items ?? [] as item (item.source_guid)}
				{@const on = chosen.some((pick) => pick.source_guid === item.source_guid)}
				<label class="tick">
					<input
						type="checkbox"
						checked={on}
						onchange={(event) =>
							onChange(
								event.currentTarget.checked
									? [...chosen, { ...item }]
									: chosen.filter((pick) => pick.source_guid !== item.source_guid)
							)}
					/>
					<span>
						<b>{item.code}</b>
						{#if item.subject}<span class="tag-note">{item.subject}</span>{/if}
						{#if item.grade_band}<span class="tag-note">{item.grade_band}</span>{/if}
						{item.statement}
					</span>
				</label>
			{/each}
		</div>
	{/if}

	{#each found.data?.notices ?? [] as notice (notice.text)}
		<p class="foot-note">{notice.text}</p>
	{/each}

	{#if chosen.length > 0}
		<div class="chips" role="list">
			{#each chosen as pick (pick.source_guid)}
				<span class="chip-pick" role="listitem">
					{pick.code}
					<button
						type="button"
						aria-label="Remove {pick.code}"
						onclick={() => drop(pick.source_guid)}>×</button
					>
				</span>
			{/each}
		</div>
	{/if}
</div>
