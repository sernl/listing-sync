<script lang="ts">
	import type { FacetView } from '$lib/api';
	import Button from '$lib/Button.svelte';
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
<div class="field fp">
	<span id="{base}-label">
		{label}{#if required}<span class="req">Required</span>{/if}
		<span class="fp-count" class:over={counter.over}>{counter.text}</span>
	</span>
	{#if hint}<span class="hint">{hint}</span>{/if}

	{#if chosen.length > 0}
		<div class="fp-picked" role="list">
			{#each chosen as slug (slug)}
				<span class="fp-chip" role="listitem">
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
		<div class="fp-list" role="group" aria-labelledby="{base}-label">
			{#if matches.length === 0}
				<p class="fp-note">Nothing matches “{query}”.</p>
			{:else}
				{#each matches as facet (facet.slug)}
					{@const on = chosen.includes(facet.slug)}
					<label class="fp-tick" class:off={full && !on}>
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
		<div class="fp-acts">
			<Button small onclick={() => (open = false)}>Done</Button>
			{#if full}
				<span class="hint">That is the limit TPT's own form states.</span>
			{/if}
		</div>
	{/if}
</div>

<style>
	/* This picker's own rules, in the component: the wrapper, the label and the
	   hint are `Field`'s own classes so the control reads as one of the
	   catalogue's, and what is here is the search, the chosen chips and the
	   list, which `Field` has no notion of. Tokens only. */
	.fp {
		position: relative;
	}

	/* The search control's own shape. It was `app.css`'s `.picker
	   input[type=search]` before this conversion, and dropping that class
	   dropped the border with it. */
	.fp input[type='search'] {
		margin-top: 6px;
		border: 1px solid var(--line);
		background: var(--surface);
		border-radius: var(--r-field);
		padding: 9px 11px;
		font: inherit;
		font-weight: 400;
		color: var(--text);
		min-width: 0;
	}

	.fp input[type='search']:focus-visible {
		border-color: var(--accent);
	}

	.fp-list {
		margin-top: 6px;
	}

	.fp-picked {
		margin: 6px 0 2px;
	}

	.fp-count {
		margin-left: 8px;
		font-size: 11.5px;
		font-weight: 400;
		color: var(--muted);
		font-variant-numeric: tabular-nums;
	}

	.fp-count.over {
		color: var(--bad);
		font-weight: 600;
	}

	.fp-picked {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}

	.fp-chip {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		border: 1px solid var(--line);
		background: var(--ground);
		border-radius: var(--r-pill);
		padding: 3px 6px 3px 11px;
		font-size: 12.5px;
	}

	.fp-chip button {
		border: 0;
		background: none;
		color: var(--muted);
		font-size: 14px;
		line-height: 1;
		padding: 3px 5px;
		border-radius: 50%;
		cursor: pointer;
	}

	.fp-chip button:hover {
		background: var(--hover);
		color: var(--text);
	}

	/* The list runs to 133 members, so it scrolls inside its own box rather
	   than pushing the rest of the form down the page. */
	.fp-list {
		display: flex;
		flex-direction: column;
		gap: 2px;
		max-height: 260px;
		overflow-y: auto;
		border: 1px solid var(--line);
		border-radius: var(--r-panel);
		background: var(--surface);
		padding: 6px;
	}

	.fp-tick {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px;
		border-radius: 6px;
		font-size: 13px;
		min-height: 32px;
		cursor: pointer;
	}

	.fp-tick:hover {
		background: var(--hover);
	}

	/* At the cap the rest are refused rather than hidden, so the seller can see
	   what they would have to give up to choose another. */
	.fp-tick.off {
		color: var(--soon);
		cursor: not-allowed;
	}

	.fp-note {
		margin: 0;
		padding: 6px;
		font-size: 12.5px;
		color: var(--muted);
	}

	.fp-acts {
		display: flex;
		align-items: center;
		gap: 12px;
		flex-wrap: wrap;
	}
</style>
