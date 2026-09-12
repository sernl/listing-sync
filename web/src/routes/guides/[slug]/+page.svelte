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
	import { IMAGE_PRIVACY, loadsRemoteImages } from '$lib/pages/guides/editor';
	import { taxonLabel } from '$lib/pages/guides/filters';
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

	/** Whether this guide loads a picture from another site, which is the only
	 *  case the privacy line has anything to say about. */
	const remoteImages = $derived(view !== undefined && loadsRemoteImages(view.html));
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
			{#if view.topic !== null || view.tags.length > 0}
				<!-- What this guide is filed under, as published. A retired topic or
				     tag still shows here: retirement takes it out of the pickers,
				     not off the guides that carry it, and a link back to it still
				     filters. -->
				<div class="gd-row-tax gd-detail-tax">
					{#if view.topic !== null}
						<a class="gd-tax gd-tax-topic" href={`/guides?topic=${view.topic.id}`}>
							{taxonLabel(view.topic)}
						</a>
					{/if}
					{#each view.tags as tag (tag.id)}
						<a class="gd-tax" href={`/guides?tags=${tag.id}`}>{taxonLabel(tag)}</a>
					{/each}
				</div>
			{/if}
			<!-- The server's own rendering of the guide's Markdown, unchanged.
			     Raw HTML in a guide is escaped during that rendering rather than
			     passed through, the images it writes carry their own
			     `referrerpolicy`, and an address it will not permit never
			     becomes an element — so this sink can only carry markup the
			     renderer decided on, and nothing here second-guesses it. -->
			<div class="guide-body">{@html view.html}</div>
			<p class="foot-note" title={utcInstant(view.updated_at)}>
				Last updated {agoLabel(view.updated_at, now)}.
			</p>
			{#if remoteImages}
				<p class="foot-note">{IMAGE_PRIVACY}</p>
			{/if}
		{/if}
	</Panel>
</div>
