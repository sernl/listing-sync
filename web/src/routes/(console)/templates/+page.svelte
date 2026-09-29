<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import { entitlementRead, limitOf } from '$lib/entitlement-read';
	import PageHead from '$lib/PageHead.svelte';
	import TabBar from '$lib/TabBar.svelte';
	import { queryKeys } from '$lib/query';
	import MappingTab from '$lib/pages/templates/MappingTab.svelte';
	import ResourceTab from '$lib/pages/templates/ResourceTab.svelte';
	import { templateKeys, templates } from '$lib/pages/templates/api';
	import { MAPPING, TEMPLATES, tabsOf } from '$lib/pages/templates/tabs';
	import '$lib/pages/templates/templates.css';
	import '$lib/flow.css';

	let current = $state<string>(TEMPLATES);
	/** A request to clear the editor, raised by this page's own header action
	 *  and answered once by the tab that owns the form. A flag rather than a
	 *  call because the form's state belongs to the tab; the tab clears it when
	 *  it answers, so a re-run cannot clear a form twice. */
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

	// The template cap, from the plan the shell already read. Both controls
	// that open a blank form — this page's own and the one inside the tab —
	// read the same figure, so neither can offer a form the save would then
	// be refused for.
	const plan = createQuery(() => entitlementRead);
	const capped = $derived(limitOf(plan.data, 'templates'));

	// `?? null` rather than `?? 0`: before the first read there is no figure,
	// and after a failed one there is no figure either. Once a read has
	// succeeded the cache keeps its answer, so a failed refetch reports the
	// number it last actually saw rather than losing it.
	const tabs = $derived(
		tabsOf({
			templates: store.data?.length ?? null,
			mappings: overrides.data?.length ?? null
		})
	);

	/** The header's own action. It belongs to the templates tab, so it
	 *  selects that tab before clearing its editor rather than acting
	 *  invisibly on a tab the seller is not looking at. The tab itself decides
	 *  what a clear costs: a form with anything in it is cleared with an Undo
	 *  beside it. */
	function newTemplate() {
		current = TEMPLATES;
		blankRequested = true;
	}
</script>

<div class="page flow-page tpl-page">
	<PageHead
		icon="layout-template"
		title="Templates"
		description="Save answers you reuse, then fill them in on many resources at once."
		guide="templates"
	>
		{#snippet aside()}
			<Button
				tier="primary"
				icon="circle-plus"
				disabled={capped !== null}
				reason={capped ?? undefined}
				onclick={newTemplate}
			>
				New template
			</Button>
		{/snippet}
	</PageHead>

	<TabBar {tabs} bind:current />

	{#if current === MAPPING}
		<MappingTab />
	{/if}

	<!-- Hidden rather than unmounted: the editor inside holds what the seller
	     is typing, and unmounting it on a look at Marketplace words would
	     empty it. The mapping tab stays behind an `{#if}`: it holds no typing
	     of its own, and mounting it would start its reads on a visit that
	     never opens it. -->
	<div class="tpl-pane" hidden={current === MAPPING}>
		<ResourceTab bind:blankRequested />
	</div>
</div>
