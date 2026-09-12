<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import '$lib/pages/guides/guides.css';

	const now = Date.now();

	const guides = createQuery(() => ({
		queryKey: queryKeys.guides,
		queryFn: () => api.guides()
	}));

	const rows = $derived(guides.data?.guides ?? []);
</script>

<div class="page">
	<PageHead
		icon="book-open"
		title="Help and guides"
		description="Guides, articles and news from Teachouse."
	/>

	<Panel>
		{#if guides.isPending}
			<p class="quiet">Reading the guides…</p>
		{:else if guides.isError}
			<Placeholder
				icon="book-open"
				headline="The guides could not be read"
				body="The request did not come back with an answer we can act on. Reloading is the only
					thing worth trying from here."
			/>
		{:else if rows.length === 0}
			<Placeholder
				icon="book-open"
				headline="No guide is published yet"
				body="This is where the guides will be: connecting a marketplace, importing your
					portfolio, and listing a resource everywhere."
			/>
		{:else}
			<div class="guide-list">
				{#each rows as guide (guide.slug)}
					<a href={`/guides/${guide.slug}`}>
						<span class="t">{guide.title}</span>
						<span class="when" title={utcInstant(guide.updated_at)}>
							updated {agoLabel(guide.updated_at, now)}
						</span>
					</a>
				{/each}
			</div>
		{/if}
	</Panel>
</div>
