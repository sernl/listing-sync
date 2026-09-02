<script lang="ts">
	import type { FacetView } from '$lib/api';
	import { atCap, counterOf, searchFacets, togglePick } from '$lib/tpt-form';

	let {
		label,
		required = false,
		placeholder,
		hint,
		facets,
		chosen,
		cap,
		onChange
	}: {
		label: string;
		required?: boolean;
		placeholder: string;
		hint?: string;
		facets: readonly FacetView[];
		chosen: readonly string[];
		cap: number | null;
		onChange: (values: string[]) => void;
	} = $props();

	const base = $props.id();
	let query = $state('');
	let open = $state(false);

	const counter = $derived(counterOf(chosen.length, cap));
	const full = $derived(atCap(chosen.length, cap));
	const matches = $derived(searchFacets(facets, query).slice(0, 60));
	const byslug = $derived(new Map(facets.map((facet) => [facet.slug, facet])));

	function drop(slug: string) {
		onChange(togglePick(chosen, slug, false, cap));
	}
</script>

<!-- TPT renders these as closed react-select comboboxes whose cap lives in the
     placeholder and is discovered by being refused. The control below keeps
     the shape and adds the two things that cost nothing: a live counter
     against the measured cap, and a search over a list that runs to 133
     members. Where no cap is measured the counter reads a plain count rather
     than inventing a ceiling. -->
<div class="field picker">
	<span id="{base}-label">
		{label}
		{#if required}<span class="req">Required</span>{/if}
		<span class="counter {counter.over ? 'over' : ''}">{counter.text}</span>
	</span>
	{#if hint}<span class="hint">{hint}</span>{/if}

	{#if chosen.length > 0}
		<div class="chips" role="list">
			{#each chosen as slug (slug)}
				<span class="chip-pick" role="listitem">
					{byslug.get(slug)?.label ?? slug}
					<button type="button" aria-label="Remove {byslug.get(slug)?.label ?? slug}" onclick={() => drop(slug)}>
						×
					</button>
				</span>
			{/each}
		</div>
	{/if}

	<input
		id="{base}-search"
		type="search"
		{placeholder}
		aria-labelledby="{base}-label"
		value={query}
		oninput={(event) => {
			query = event.currentTarget.value;
			open = true;
		}}
		onfocus={() => (open = true)}
	/>

	{#if open}
		<div class="pick-list" role="group" aria-labelledby="{base}-label">
			{#if matches.length === 0}
				<p class="quiet">Nothing matches “{query}”.</p>
			{:else}
				{#each matches as facet (facet.slug)}
					{@const on = chosen.includes(facet.slug)}
					<label class="tick {full && !on ? 'off' : ''}">
						<input
							type="checkbox"
							checked={on}
							disabled={full && !on}
							onchange={(event) =>
								onChange(togglePick(chosen, facet.slug, event.currentTarget.checked, cap))}
						/>
						<span>{facet.label}</span>
					</label>
				{/each}
			{/if}
		</div>
		<div class="inline-choices">
			<button class="btn small" type="button" onclick={() => (open = false)}>Done</button>
			{#if full}
				<span class="hint">That is the limit TPT's own form states.</span>
			{/if}
		</div>
	{/if}
</div>
