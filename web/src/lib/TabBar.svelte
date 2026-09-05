<script lang="ts" module>
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
	}

	/** What a tab reads as. An unknown count shows the label alone: no
	 *  parenthesis is honest, and a parenthesis around a dash or a zero is a
	 *  figure the seller has no reason to distrust. */
	export function labelFor(tab: Tab): string {
		return tab.count === null ? tab.label : `${tab.label} (${tab.count})`;
	}
</script>

<script lang="ts">
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
			title={tab.hint}
			onclick={() => {
				current = tab.id;
				onselect?.(tab.id);
			}}
		>
			{labelFor(tab)}
		</button>
	{/each}
</div>
