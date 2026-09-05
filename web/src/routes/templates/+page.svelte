<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import TabBar from '$lib/TabBar.svelte';
	import { queryKeys } from '$lib/query';
	import MappingTab from '$lib/pages/templates/MappingTab.svelte';
	import ResourceTab from '$lib/pages/templates/ResourceTab.svelte';
	import { templateKeys, templates } from '$lib/pages/templates/api';
	import { MAPPING, RESOURCE, tabsOf } from '$lib/pages/templates/tabs';
	import '$lib/pages/templates/templates.css';

	let current = $state<string>(MAPPING);
	/** A request for a blank form, held here rather than in the tab because the
	 *  tab does not exist while the seller is on the other one, and this page's
	 *  own action is reachable from there. The tab clears it when it answers. */
	let blankRequested = $state(false);

	// Both counts, read here because the tab bar carries them. Each tab reads
	// the same key for its own contents, so this is one fetch and not two.
	const overrides = createQuery(() => ({
		queryKey: queryKeys.overrides,
		queryFn: () => api.overrides().then((view) => view.overrides)
	}));

	const store = createQuery(() => ({
		queryKey: templateKeys.all,
		queryFn: () => templates.list()
	}));

	// `?? null` rather than `?? 0`: before the first read there is no figure,
	// and after a failed one there is no figure either. Once a read has
	// succeeded the cache keeps its answer, so a failed refetch reports the
	// number it last actually saw rather than losing it.
	const tabs = $derived(tabsOf(overrides.data?.length ?? null, store.data?.length ?? null));

	/** The header's own action. It belongs to the second tab, so it selects
	 *  that tab before opening its form rather than acting invisibly on a tab
	 *  the seller is not looking at. */
	function newTemplate() {
		current = RESOURCE;
		blankRequested = true;
	}
</script>

<div class="page">
	<PageHead
		icon="layout-template"
		title="Template Manager"
		description="Saved answers you reuse — how your words map onto a marketplace, and what a new resource starts with."
	>
		{#snippet aside()}
			<Button tier="primary" icon="circle-plus" onclick={newTemplate}>New template</Button>
		{/snippet}
	</PageHead>

	<TabBar {tabs} bind:current />

	{#if current === MAPPING}
		<MappingTab />
	{:else}
		<ResourceTab bind:blankRequested />
	{/if}
</div>
