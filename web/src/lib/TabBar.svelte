<script lang="ts" module>
	import type { Marketplace } from '$lib/generated/vocab';

	export interface Tab {
		id: string;
		label: string;
		/** How many rows this tab holds, or `null` where the caller does not
		 *  know — a read that failed, rather than a read that returned nothing.
		 *
		 *  Nullable rather than optional on purpose: a caller still has to say
		 *  something, and "I have no figure" is a different statement from
		 *  forgetting to pass one. Zero is the wrong stand-in precisely because
		 *  it is plausible, so a seller whose templates could not be read would
		 *  see the same number as a seller who has none. */
		count: number | null;
		/** What the tab means where the label cannot say it, such as a filter
		 *  that deliberately overlaps the others. */
		hint?: string;
		/** The marketplace whose mark is drawn in place of the label, where the
		 *  tab is one marketplace; the label stays the tab's accessible name. */
		mark?: Marketplace;
	}

	/** What a tab reads as. An unknown count shows the label alone: no
	 *  parenthesis is honest, and a parenthesis around a dash or a zero is a
	 *  figure the seller has no reason to distrust. */
	export function labelFor(tab: Tab): string {
		return tab.count === null ? tab.label : `${tab.label} (${tab.count})`;
	}
</script>

<script lang="ts">
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';

	let {
		tabs,
		current = $bindable(),
		onselect
	}: {
		tabs: readonly Tab[];
		current: string;
		onselect?: (id: string) => void;
	} = $props();
</script>

<div class="tab-bar" role="tablist">
	{#each tabs as tab (tab.id)}
		<button
			type="button"
			role="tab"
			aria-current={tab.id === current}
			aria-selected={tab.id === current}
			aria-label={tab.mark === undefined ? undefined : labelFor(tab)}
			title={tab.hint}
			onclick={() => {
				current = tab.id;
				onselect?.(tab.id);
			}}
		>
			{#if tab.mark === undefined}
				{labelFor(tab)}
			{:else}
				<MarketplaceMark marketplace={tab.mark} size={18} />{#if tab.count !== null}<span
						class="count">({tab.count})</span
					>{/if}
			{/if}
		</button>
	{/each}
</div>

<style>
	.count {
		margin-left: 5px;
	}
</style>
