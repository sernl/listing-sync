<script lang="ts">
	import type { Marketplace } from '$lib/generated/vocab';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import type { MarketplaceRow } from './marketplace-list';

	let {
		rows,
		selected = $bindable(null),
		onselect,
		empty,
		label = 'Your marketplaces'
	}: {
		rows: readonly MarketplaceRow[];
		/** The marketplace the page narrows to, or null where the strip only
		 *  reports link status and selects nothing. */
		selected?: Marketplace | null;
		onselect?: (marketplace: Marketplace) => void;
		/** What the strip says where the seller has connected nothing. */
		empty: string;
		label?: string;
	} = $props();
</script>

<!-- One chip per connected marketplace: its mark, its link status and the
     page's figure for it. Plain buttons with `aria-pressed` where the page
     narrows by marketplace, plain items where it does not: a pressable chip
     that changes nothing is a control that lies. -->
<div class="mk-strip" role="group" aria-label={label}>
	<span class="mk-strip-label">{label}</span>
	{#if rows.length === 0}
		<span class="mk-strip-empty">{empty}</span>
	{:else}
		{#each rows as row (row.marketplace)}
			{#if onselect}
				<button
					type="button"
					class="mk-chip"
					aria-pressed={row.marketplace === selected}
					onclick={() => {
						selected = row.marketplace;
						onselect?.(row.marketplace);
					}}
				>
					<MarketplaceMark marketplace={row.marketplace} size={18} />
					<StatusPill tone={row.tone} label={row.status} />
					{#if row.count !== null}<span class="mk-count">{row.count}</span>{/if}
				</button>
			{:else}
				<span class="mk-chip">
					<MarketplaceMark marketplace={row.marketplace} size={18} />
					<StatusPill tone={row.tone} label={row.status} />
					{#if row.count !== null}<span class="mk-count">{row.count}</span>{/if}
				</span>
			{/if}
		{/each}
	{/if}
</div>

<style>
	.mk-strip {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: var(--s-2);
		margin-bottom: var(--s-5);
	}

	.mk-strip-label {
		margin-right: var(--s-1);
		color: var(--muted);
		font-size: 12.5px;
		font-weight: 600;
	}

	.mk-strip-empty {
		color: var(--muted);
		font-size: 13px;
	}

	.mk-chip {
		display: inline-flex;
		align-items: center;
		gap: var(--s-2);
		min-height: 34px;
		padding: 3px 10px 3px 6px;
		border: 1px solid var(--line);
		border-radius: var(--r-pill);
		background: var(--mark-ground);
		color: var(--text);
		font: inherit;
		font-size: 13px;
	}

	button.mk-chip {
		cursor: pointer;
	}

	button.mk-chip:hover {
		border-color: var(--lavender);
	}

	button.mk-chip:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 2px;
	}

	.mk-chip[aria-pressed='true'] {
		border-color: var(--accent);
		box-shadow: inset 0 0 0 1px var(--accent);
	}

	.mk-count {
		display: inline-grid;
		place-items: center;
		min-width: 22px;
		height: 22px;
		padding: 0 6px;
		border-radius: var(--r-pill);
		background: var(--additive-soft);
		color: var(--additive);
		font-size: 12px;
		font-weight: 700;
	}
</style>
