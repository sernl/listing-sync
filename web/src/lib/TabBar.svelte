<script lang="ts" module>
	export interface Tab {
		id: string;
		label: string;
		count: number;
		/** What the tab means where the label cannot say it, such as a filter
		 *  that deliberately overlaps the others. */
		hint?: string;
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
			{tab.label} ({tab.count})
		</button>
	{/each}
</div>
