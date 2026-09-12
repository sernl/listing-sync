<script lang="ts">
	import { untrack } from 'svelte';
	import { createQuery } from '@tanstack/svelte-query';
	import { beforeNavigate, goto } from '$app/navigation';
	import { page } from '$app/state';
	import { ApiFailure, api } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import Field from '$lib/Field.svelte';
	import Menu from '$lib/Menu.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import {
		filterKey,
		filterSearch,
		filtersFromUrl,
		offered,
		taxonLabel,
		type GuideFilters
	} from '$lib/pages/guides/filters';
	import '$lib/pages/guides/guides.css';

	const now = Date.now();

	// The narrowing is the address. Every read is keyed by it, so the back
	// button, a reload and a shared link all land on the same list, and a slow
	// answer to a narrowing the reader has already left cannot be drawn into
	// the one they are looking at.
	const filters = $derived(filtersFromUrl(page.url.searchParams));
	const narrowed = $derived(
		filters.q.length > 0 || filters.topic !== null || filters.tags.length > 0
	);

	const taxonomy = createQuery(() => ({
		queryKey: queryKeys.guideTaxonomy,
		queryFn: () => api.guideTaxonomy()
	}));

	const guides = createQuery(() => ({
		queryKey: [...queryKeys.guides, filterKey(filters)],
		// The wire takes an absent topic rather than a null one, which is the
		// one place the two shapes differ.
		queryFn: () => api.guides({ q: filters.q, topic: filters.topic ?? undefined, tags: filters.tags }),
		// A refused narrowing is refused however many times it is asked: the
		// ids in the address either name taxa or do not. The 401 arm is the
		// client's own default, restated because naming a retry here replaces
		// it wholesale.
		retry: (failures: number, failure: Error) =>
			!(failure instanceof ApiFailure && (failure.status === 422 || failure.status === 401)) &&
			failures < 3
	}));

	const rows = $derived(guides.data?.guides ?? []);
	const total = $derived(guides.data?.total ?? 0);
	/** Whether the server refused the ids in the address rather than failing to
	 *  answer. A retired taxon is not one of these — retirement hides a taxon
	 *  from the pickers and keeps every published guide that carries it
	 *  reachable — so this is a typo or a hand-edited link. */
	const refusedFilters = $derived(
		guides.error instanceof ApiFailure && guides.error.status === 422
	);

	// The topics and tags published guides actually carry, which is what the
	// reader's taxonomy endpoint answers: a control offering a topic no
	// published guide sits under can only ever narrow to nothing.
	const topics = $derived(
		offered(taxonomy.data?.topics ?? [], filters.topic === null ? [] : [filters.topic])
	);
	const tags = $derived(offered(taxonomy.data?.tags ?? [], filters.tags));

	let box = $state(filtersFromUrl(page.url.searchParams).q);
	let tagMenu = $state(false);
	/** The pending address write, so typing a phrase is one narrowing rather
	 *  than one per keystroke. Held as a timer rather than debounced inside the
	 *  query, because the address is the thing being written. */
	let typing: ReturnType<typeof setTimeout> | null = null;

	// The box follows the address where the address changed under it — a back
	// button, a reload, a link from elsewhere — and is otherwise left alone.
	//
	// Only the address is read here. Reading the box as well made this effect
	// re-run on every keystroke and compare the new text against the address it
	// had not been written to yet, so each letter typed was immediately reset
	// to the last narrowing: with the 300ms settle below, typing "lesson" ended
	// up as "n". The write is untracked for the same reason.
	$effect(() => {
		const asked = filters.q;
		untrack(() => {
			if (asked !== box.trim()) {
				box = asked;
			}
		});
	});

	const SETTLE_MS = 300;

	/** A scheduled address write is cancelled when this page is left, and is
	 *  fenced against the page it was scheduled on.
	 *
	 * Without both, following a link while a keystroke's timer is still out
	 * lands on the destination and is then dragged back here by the timer's
	 * `goto`, replacing the destination's own history entry. */
	let left = false;

	function cancel() {
		if (typing !== null) {
			clearTimeout(typing);
			typing = null;
		}
	}

	beforeNavigate((navigation) => {
		if (navigation.to?.url.pathname !== page.url.pathname) {
			left = true;
			cancel();
		}
	});

	$effect(() => cancel);

	function narrow(next: GuideFilters, settle = 0) {
		cancel();
		const from = page.url.pathname;
		const search = filterSearch(next);
		const to = search.length === 0 ? from : `${from}?${search}`;
		const write = () => {
			typing = null;
			if (left || page.url.pathname !== from) {
				return;
			}
			// Replaced rather than pushed: a filter is not a place, and a back
			// button that walked every keystroke would be one.
			void goto(to, { replaceState: true, keepFocus: true, noScroll: true });
		};
		if (settle === 0) {
			write();
			return;
		}
		typing = setTimeout(write, settle);
	}

	/** A change to one control, carrying whatever the others hold now.
	 *
	 * The search box rather than the address for `q`: a topic chosen while a
	 * keystroke's timer is still out cancels that timer, and composing from the
	 * address would navigate with the previous search and then reset the box to
	 * it — the reader's half-typed query dropped by choosing a topic. */
	function choose(over: Partial<GuideFilters>) {
		narrow({ q: box.trim(), topic: filters.topic, tags: filters.tags, ...over });
	}

	function typed(next: string) {
		box = next;
		narrow({ ...filters, q: next.trim() }, SETTLE_MS);
	}

	/** Every control back to nothing, the search box included: a reset that
	 *  left the typed phrase in the box would clear the narrowing and then have
	 *  the next topic choice put the phrase straight back. */
	function reset() {
		box = '';
		narrow({ q: '', topic: null, tags: [] });
	}
</script>

<div class="page">
	<PageHead
		icon="book-open"
		title="Help and guides"
		description="Guides, articles and news from Teachouse."
	/>

	<Panel>
		<div class="gd-filters">
			<Field label="Search" id="guide-search">
				<input
					id="guide-search"
					type="search"
					placeholder="Search titles, text and topics"
					value={box}
					oninput={(event) => typed(event.currentTarget.value)}
				/>
			</Field>

			<Field label="Topic" id="guide-topic">
				<select
					id="guide-topic"
					value={filters.topic ?? ''}
					disabled={taxonomy.isError}
					onchange={(event) =>
						choose({
							topic: event.currentTarget.value.length === 0 ? null : event.currentTarget.value
						})}
				>
					<option value="">Every topic</option>
					{#each topics as topic (topic.id)}
						<option value={topic.id}>{taxonLabel(topic)}</option>
					{/each}
				</select>
			</Field>

			<div class="gd-group">
				<span class="gd-group-label" id="guide-tags">Tags</span>
				<Menu bind:open={tagMenu} label="Filter by tag" align="start">
					{#snippet trigger()}
						<Button onclick={() => (tagMenu = !tagMenu)}>
							{filters.tags.length === 0 ? 'Any tag' : `${filters.tags.length} chosen`}
						</Button>
					{/snippet}
					<div class="gd-tag-menu" role="group" aria-labelledby="guide-tags">
						{#if taxonomy.isPending}
							<p class="none">Reading the tags…</p>
						{:else if taxonomy.isError}
							<p class="none">The tags could not be read, so none can be chosen here.</p>
						{:else}
							{#each tags as tag (tag.id)}
								<label>
									<input
										type="checkbox"
										checked={filters.tags.includes(tag.id)}
										onchange={(event) =>
											choose({
												tags: event.currentTarget.checked
													? [...filters.tags, tag.id]
													: filters.tags.filter((id) => id !== tag.id)
											})}
									/>
									<span class="gd-tax">{taxonLabel(tag)}</span>
								</label>
							{:else}
								<p class="none">No published guide carries a tag yet.</p>
							{/each}
						{/if}
					</div>
				</Menu>
			</div>
		</div>

		<div class="gd-filter-foot">
			<!-- The count is the server's, and it is stated only once a read is
			     in: "0 guides" is what an empty search says, so standing it in
			     for a read that failed would state something false. -->
			<span class="eyebrow">
				{#if guides.isSuccess}
					{total}
					{total === 1 ? 'guide' : 'guides'}
					{narrowed ? 'match' : 'published'}
				{:else if guides.isPending}
					{narrowed ? 'Searching…' : 'Reading the guides…'}
				{:else}
					Nothing counted
				{/if}
			</span>
			{#if narrowed}
				<Button tier="quiet" small onclick={reset}>
					Reset filters
				</Button>
			{/if}
		</div>
	</Panel>

	<Panel>
		{#if guides.isPending}
			<p class="quiet">Reading the guides…</p>
		{:else if refusedFilters}
			<!-- A hand-edited address, or a link written against a topic or tag id
			     that never existed. The server refuses the ids rather than
			     answering a narrowing it cannot read, and this page says so
			     rather than quietly dropping them and showing a different list
			     than the one that was asked for. -->
			<Placeholder
				icon="book-open"
				headline="Those filters cannot be read"
				body="The address names a topic or tag this site does not hold. Resetting the filters
					shows every published guide."
			/>
			<div class="gd-filter-foot">
				<Button tier="outline" small onclick={reset}>
					Reset filters
				</Button>
			</div>
		{:else if guides.isError}
			<Placeholder
				icon="book-open"
				headline="The guides could not be read"
				body="The request did not come back with an answer we can act on. Clearing the filters
					or reloading are the things worth trying from here."
			/>
			{#if narrowed}
				<div class="gd-filter-foot">
					<Button tier="outline" small onclick={reset}>
						Reset filters
					</Button>
				</div>
			{/if}
		{:else if rows.length === 0 && narrowed}
			<!-- An empty search and an empty shelf are different answers, and the
			     first one has something to do about it. -->
			<Placeholder
				icon="book-open"
				headline="No guide matches those filters"
				body="Nothing published matches that search, topic and tag together. Widening any one
					of them is the way back to the full list."
			/>
			<div class="gd-filter-foot">
				<Button tier="outline" small onclick={reset}>
					Reset filters
				</Button>
			</div>
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
						<span class="gd-row-tax">
							{#if guide.topic !== null}
								<span class="gd-tax gd-tax-topic">{taxonLabel(guide.topic)}</span>
							{/if}
							{#each guide.tags as tag (tag.id)}
								<span class="gd-tax">{taxonLabel(tag)}</span>
							{/each}
						</span>
						<span class="when" title={utcInstant(guide.updated_at)}>
							updated {agoLabel(guide.updated_at, now)}
						</span>
					</a>
				{/each}
			</div>
		{/if}
	</Panel>
</div>
