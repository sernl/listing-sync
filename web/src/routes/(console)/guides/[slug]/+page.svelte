<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { ApiFailure, api } from '$lib/api';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import { pageTitle } from '$lib/page-title.svelte';
	import Icon from '$lib/Icon.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import { IMAGE_PRIVACY, loadsRemoteImages } from '$lib/pages/guides/editor';
	import { nextGuide, outline } from '$lib/pages/guides/article';
	import { NO_FILTERS, filterKey, taxonLabel } from '$lib/pages/guides/filters';
	import '$lib/flow.css';
	import '$lib/pages/guides/guides.css';
	import { capture } from '$lib/posthog';

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

	/** The guide's HTML with anchored headings and captioned pictures, and the
	 *  headings for the contents beside it. */
	const shaped = $derived(view === undefined ? null : outline(view.html));

	// The whole index, under the key the index page reads it by, so arriving
	// from the index costs no second request. It answers the "Next guide"
	// footer: the guide after this one in the index's order.
	const index = createQuery(() => ({
		queryKey: [...queryKeys.guides, filterKey(NO_FILTERS)],
		queryFn: () => api.guides({ sort: NO_FILTERS.sort })
	}));
	const next = $derived(nextGuide(index.data?.guides ?? [], slug));

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

	// One row per guide actually read, once: the effect re-runs whenever the
	// query settles again, and a reader who scrolls is not a second opening.
	let openedSlug: string | null = null;
	$effect(() => {
		if (view === undefined || openedSlug === view.slug) {
			return;
		}
		openedSlug = view.slug;
		capture('guide_opened', {
			slug: view.slug,
			taxon: view.topic?.slug ?? null,
			signed_in: page.data.session !== null && page.data.session !== undefined
		});
	});
</script>

<div class="page gd-reader">
	<PageHead
		icon="book-open"
		title={view?.title ?? 'Guide'}
		description={view === undefined
			? 'Loading guide…'
			: `Updated ${agoLabel(view.updated_at, now)}`}
		back={{ href: '/guides', label: 'All guides' }}
	/>

	{#if guide.isPending}
		<p class="quiet">Loading guide…</p>
	{:else if missing}
		<Placeholder
			icon="book-open"
			headline="We could not find this guide"
			body="It may have been renamed or removed. See all guides for what is available."
		/>
	{:else if guide.isError}
		<Placeholder
			icon="book-open"
			headline="We could not load this guide"
			body="Try reloading the page."
		/>
	{:else if view !== undefined && shaped !== null}
		<div class="gd-article" class:gd-no-toc={shaped.headings.length === 0}>
			<article class="gd-article-main">
				{#if view.topic !== null || view.tags.length > 0}
					<!-- What this guide is filed under, as published. A retired topic
					     or tag still shows here: retirement takes it out of the
					     pickers, not off the guides that carry it. -->
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
				{#if shaped.headings.length > 1}
					<details class="flow-more gd-toc-phone">
						<summary>On this page</summary>
						<ol>
							{#each shaped.headings as heading (heading.id)}
								<li><a href="#{heading.id}">{heading.text}</a></li>
							{/each}
						</ol>
					</details>
				{/if}
				<!-- The server's own rendering of the guide's Markdown. Raw HTML in a
				     guide is escaped during that rendering, the images it writes
				     carry their own `referrerpolicy`, and an address it will not
				     permit never becomes an element. `outline` adds an id to each
				     heading and wraps a picture on its own line in a captioned
				     figure, reusing the alt text the renderer already escaped, so
				     this sink still carries only markup the renderer decided on. -->
				<div class="guide-body">{@html shaped.html}</div>
				<p class="foot-note" title={utcInstant(view.updated_at)}>
					Last updated {agoLabel(view.updated_at, now)}.
				</p>
				{#if remoteImages}
					<p class="foot-note">{IMAGE_PRIVACY}</p>
				{/if}

				{#if next !== null}
					<a class="gd-next" href={`/guides/${next.slug}`}>
						<span class="gd-next-label">Next guide</span>
						<span class="gd-next-title">
							{next.title}
							<Icon name="chevron-right" size={18} />
						</span>
					</a>
				{:else}
					<a class="gd-next" href="/guides">
						<span class="gd-next-label">That's the last one</span>
						<span class="gd-next-title">
							All guides
							<Icon name="chevron-right" size={18} />
						</span>
					</a>
				{/if}
			</article>

			{#if shaped.headings.length > 0}
				<nav class="gd-toc" aria-label="On this page">
					<span class="gd-toc-label">On this page</span>
					<ol>
						{#each shaped.headings as heading (heading.id)}
							<li><a href="#{heading.id}">{heading.text}</a></li>
						{/each}
					</ol>
				</nav>
			{/if}
		</div>
	{/if}
</div>
