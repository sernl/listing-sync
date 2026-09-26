<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api, type FrameworkView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import TabBar from '$lib/TabBar.svelte';
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

	/** The frameworks as the shared tab bar takes them.
	 *
	 *  Every count is null, and that is the honest figure rather than a missing
	 *  one: how many standards a jurisdiction holds is not read until it is
	 *  searched, and a framework that is not ingested at all holds none in a way
	 *  a zero would misstate. A null prints the bare label. */
	const tabs = $derived(
		frameworks.map((framework) => ({
			id: String(framework.jurisdiction_id),
			label: framework.button_label,
			count: null,
			hint: framework.name
		}))
	);

	const showing = $derived(
		frameworks.find((framework) => framework.jurisdiction_id === active)?.name ?? ''
	);
</script>

<!-- Four jurisdictions, each searched on its own, because no owner publishes a
     cross-framework crosswalk and TPT itself declines to translate between
     them. The catalogue is not ingested yet, and the panel says so rather than
     rendering an empty tree: "nothing matched your words" and "nothing exists
     here yet" are different answers and a seller acts on them differently. -->
<div class="std">
	<!-- Behind the same guard as the search box: an empty tab bar is a hairline
	     with nothing above it, drawn over the sentence explaining that there is
	     nothing to choose from. -->
	{#if frameworks.length > 0}
		<TabBar {tabs} current={String(active)} onselect={(id) => (picked = Number(id))} />

		<label class="sr-only" for="standards-search">Search this framework</label>
		<input
			class="std-search"
			id="standards-search"
			type="search"
			placeholder="Search this framework by code or words"
			value={query}
			oninput={(event) => (query = event.currentTarget.value)}
		/>
	{/if}

	{#if frameworks.length === 0}
		<p class="std-note">No standards are available yet. You can add them later.</p>
	{:else if found.isPending}
		<p class="std-note">Loading standards…</p>
	{:else if found.isError}
		<Banner tone="bad">We couldn't load the standards. You can still create the listing.</Banner>
	{:else if found.data?.state === 'not_ingested'}
		<Banner tone="warn" title="This framework isn't ready yet">
			You can't search {showing} yet. Standards are optional, so you can add them later.
		</Banner>
	{:else if query.trim() === ''}
		<p class="std-note">Type a code or a few words to search.</p>
	{:else if (found.data?.items ?? []).length === 0}
		<p class="std-note">Nothing in this framework matches “{query}”.</p>
	{:else}
		<div class="std-list">
			{#each found.data?.items ?? [] as item (item.source_guid)}
				{@const on = chosen.some((pick) => pick.source_guid === item.source_guid)}
				<label class="std-item">
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
						<b class="std-code">{item.code}</b>
						{#if item.subject}<span class="std-facet">{item.subject}</span>{/if}
						{#if item.grade_band}<span class="std-facet">{item.grade_band}</span>{/if}
						{item.statement}
					</span>
				</label>
			{/each}
		</div>
	{/if}

	<!-- Folded rather than dropped. Each line is a licence obligation of the
	     standards set we mirror, so it has to stay with the standards; the
	     founder's objection was five paragraphs of copyright notice standing
	     open above the fields, not the notices themselves. -->
	{#if (found.data?.notices ?? []).length > 0}
		<details class="std-sources">
			<summary>Sources and licences</summary>
			{#each found.data?.notices ?? [] as notice (notice.text)}
				<p class="std-notice">{notice.text}</p>
			{/each}
		</details>
	{/if}

	{#if chosen.length > 0}
		<div class="std-chosen" role="list">
			{#each chosen as pick (pick.source_guid)}
				<span class="std-chip" role="listitem">
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

<style>
	/* This picker's own rules, in the component rather than in a page sheet: it
	   is a shared control with one consumer today and its shapes belong to it
	   rather than to whichever page renders it. Tokens only. */
	.std {
		display: flex;
		flex-direction: column;
		gap: 10px;
	}

	.std-search {
		border: 1px solid var(--line);
		background: var(--surface);
		border-radius: var(--r-field);
		padding: 9px 11px;
		font: inherit;
		font-weight: 400;
		color: var(--text);
		min-width: 0;
	}

	.std-search:focus-visible {
		border-color: var(--accent);
	}

	.std-note {
		margin: 0;
		font-size: 12.5px;
		color: var(--muted);
	}

	.std-list {
		display: flex;
		flex-direction: column;
		gap: 6px;
		max-height: 320px;
		overflow-y: auto;
	}

	/* One standard: a statement long enough to wrap, so each is a panel rather
	   than a line, and the checkbox holds its own column. */
	.std-item {
		display: grid;
		grid-template-columns: auto minmax(0, 1fr);
		align-items: start;
		gap: 10px;
		border: 1px solid var(--line);
		border-radius: var(--r-panel);
		background: var(--surface);
		padding: 10px 12px;
		font-size: 13px;
		cursor: pointer;
	}

	.std-item:hover {
		border-color: color-mix(in srgb, var(--accent) 35%, var(--line));
	}

	.std-item input {
		margin-top: 3px;
	}

	.std-code {
		font-weight: 700;
		margin-right: 6px;
	}

	/* The subject and the grade band the mirrored set carries, so a code never
	   renders bare. */
	.std-facet {
		display: inline-block;
		border: 1px solid var(--line);
		background: var(--ground);
		border-radius: var(--r-pill);
		padding: 1px 8px;
		margin-right: 6px;
		font-size: 11px;
		color: var(--muted);
	}

	/* A licence obligation rather than a footnote of ours, so it stays with the
	   standards wherever they are shown. */
	.std-notice {
		margin: 0;
		font-size: 11.5px;
		color: var(--faint);
	}

	/* Shut by default: the obligations are discharged by being reachable, and
	   five paragraphs of copyright notice standing open is what the founder
	   asked to be moved out of the way. */
	.std-sources {
		font-size: 11.5px;
	}

	.std-sources summary {
		color: var(--muted);
		cursor: pointer;
	}

	.std-sources > p + p {
		margin-top: 4px;
	}

	.std-chosen {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}

	.std-chip {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		border: 1px solid var(--line);
		background: var(--ground);
		border-radius: var(--r-pill);
		padding: 3px 6px 3px 11px;
		font-size: 12.5px;
	}

	.std-chip button {
		border: 0;
		background: none;
		color: var(--muted);
		font-size: 14px;
		line-height: 1;
		padding: 3px 5px;
		border-radius: 50%;
	}

	.std-chip button:hover {
		background: var(--hover);
		color: var(--text);
	}
</style>
