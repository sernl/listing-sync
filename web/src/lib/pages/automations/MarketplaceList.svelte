<script lang="ts">
	import type { Marketplace } from '$lib/generated/vocab';
	import StatusPill from '$lib/StatusPill.svelte';
	import type { MarketplaceRow } from './marketplace-list';

	let {
		rows,
		selected = $bindable(null),
		onselect,
		empty
	}: {
		rows: readonly MarketplaceRow[];
		selected?: Marketplace | null;
		onselect?: (marketplace: Marketplace) => void;
		/** What the column says where the seller has connected nothing. */
		empty: string;
	} = $props();
</script>

<!-- Plain buttons under a heading, not a `nav` and not a tablist.

     Not a `nav`, because these controls navigate nowhere: they swap the
     settings card beside the column, and a landmark called "Marketplaces
     navigation" lands a screen-reader user on controls that go nowhere.
     Not a tablist either, because the honest version of that contract needs
     `aria-controls` onto a `tabpanel` the page and not this component renders,
     plus roving tabindex and arrow-key selection — and a tablist announced
     without them tells a screen-reader user arrow keys will work when they
     will not. `aria-pressed` says what is true of each button on its own and
     needs no keyboard model beyond the one buttons already have. -->
<div class="mk-list">
	<h2 id="mk-list-title">Marketplaces</h2>
	{#if rows.length === 0}
		<p class="mk-empty">{empty}</p>
	{:else}
		<div class="mk-rows" aria-labelledby="mk-list-title">
			{#each rows as row (row.marketplace)}
				<button
					type="button"
					class="mk-row"
					aria-pressed={row.marketplace === selected}
					onclick={() => {
						selected = row.marketplace;
						onselect?.(row.marketplace);
					}}
				>
					<span class="edge" aria-hidden="true"></span>
					<span class="who">
						<span class="t">{row.name}</span>
						<StatusPill tone={row.tone} label={row.status} />
					</span>
					{#if row.count !== null}<span class="count">{row.count}</span>{/if}
				</button>
			{/each}
		</div>
	{/if}
</div>
