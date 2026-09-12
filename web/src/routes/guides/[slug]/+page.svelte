<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { ApiFailure, api } from '$lib/api';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import { pageTitle } from '$lib/page-title.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import '$lib/pages/guides/guides.css';

	const slug = $derived(page.params.slug ?? '');
	const now = Date.now();

	const guide = createQuery(() => ({
		queryKey: queryKeys.guide(slug),
		queryFn: () => api.guide(slug),
		enabled: slug.length > 0,
		// A draft and a slug nothing holds are the same 404 here, and neither
		// becomes a guide by being asked for again.
		retry: false
	}));

	const view = $derived(guide.data);

	/** This guide's own name in the tab, rather than the destination's. Released
	 *  on the way out, so the next page is not left wearing this title. */
	$effect(() => {
		const name = view?.title;
		if (name === undefined) {
			return;
		}
		pageTitle.claim(name);
		return () => pageTitle.release();
	});

	/** Whether nothing is published at this address. A draft answers the same
	 *  404 as a slug nothing holds, which is the point: an unpublished guide is
	 *  not a guide a seller has. */
	const missing = $derived(guide.error instanceof ApiFailure && guide.error.status === 404);
</script>

<div class="page">
	<PageHead
		icon="book-open"
		title={view?.title ?? 'Guide'}
		description={view === undefined
			? 'Reading this guide…'
			: `Updated ${agoLabel(view.updated_at, now)}`}
		back={{ href: '/guides', label: 'All guides' }}
	/>

	<Panel>
		{#if guide.isPending}
			<p class="quiet">Reading this guide…</p>
		{:else if missing}
			<Placeholder
				icon="book-open"
				headline="There is no guide at this address"
				body="It may have been renamed, or it may never have been published. The guides list
					has everything that has been."
			/>
		{:else if guide.isError}
			<Placeholder
				icon="book-open"
				headline="This guide could not be read"
				body="The request did not come back with an answer we can act on. Reloading is the
					only thing worth trying from here."
			/>
		{:else if view !== undefined}
			<!-- The server's own rendering of the guide's Markdown. Raw HTML is
			     escaped during that rendering rather than passed through, so
			     this sink can only ever carry markup the renderer produced. -->
			<div class="guide-body">{@html view.html}</div>
			<p class="foot-note" title={utcInstant(view.updated_at)}>
				Last updated {agoLabel(view.updated_at, now)}.
			</p>
		{/if}
	</Panel>
</div>
