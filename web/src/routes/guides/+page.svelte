<script lang="ts">
	import { untrack } from 'svelte';
	import { createQuery } from '@tanstack/svelte-query';
	import { beforeNavigate, goto } from '$app/navigation';
	import { page } from '$app/state';
	import { ApiFailure, api, type GuideHeadView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import Field from '$lib/Field.svelte';
	import Icon from '$lib/Icon.svelte';
	import Menu from '$lib/Menu.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Pagination from '$lib/Pagination.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import {
		filterKey,
		filterSearch,
		filtersFromUrl,
		narrowedBy,
		offered,
		pageSummary,
		SORTS,
		taxonLabel,
		type GuideFilters,
		type GuideSort
	} from '$lib/pages/guides/filters';
	import { SECTIONS, groupBySection } from '$lib/pages/guides/article';
	import '$lib/flow.css';
	import '$lib/pages/guides/guides.css';

	const now = Date.now();

	// The question is the address: the narrowing, the order and the page. Every
	// read is keyed by all three, so the back button, a reload and a shared
	// link all land on the same page of the same list, and a slow answer to a
	// question the reader has already left cannot be drawn into the one they
	// are looking at.
	const filters = $derived(filtersFromUrl(page.url.searchParams));
	const narrowed = $derived(narrowedBy(filters));
	/** The narrowing and the order without the page: what has to still be the
	 *  same for a page already read to be worth keeping on the screen. */
	const narrowing = $derived(filterSearch({ ...filters, page: 1 }));

	const taxonomy = createQuery(() => ({
		queryKey: queryKeys.guideTaxonomy,
		queryFn: () => api.guideTaxonomy()
	}));

	const guides = createQuery(() => {
		// One snapshot of the question, captured with the key rather than read
		// when `queryFn` eventually runs. A cached page can have a refresh
		// paused while offline and left behind by the reader, and the client
		// resumes that refresh on reconnect even after its observer has gone:
		// reading `filters` at that moment would ask for whichever page is on
		// the screen by then and cache those rows under this page's key, so
		// coming back to this page would show another page's guides.
		const asked = { ...filters, tags: [...filters.tags] };
		return {
			queryKey: [...queryKeys.guides, filterKey(asked)],
			// The wire takes an absent topic rather than a null one, which is
			// the one place the two shapes differ. The page and the order go
			// with it: the server narrows the whole published corpus and cuts
			// one page out of the narrowing, so this is never a page of rows
			// filtered afterwards.
			queryFn: () =>
				api.guides({
					q: asked.q,
					topic: asked.topic ?? undefined,
					tags: asked.tags,
					sort: asked.sort,
					page: asked.page
				}),
			// A refused narrowing is refused however many times it is asked:
			// the ids in the address either name taxa or do not. The 401 arm is
			// the client's own default, restated because naming a retry here
			// replaces it wholesale.
			retry: (failures: number, failure: Error) =>
				!(failure instanceof ApiFailure && (failure.status === 422 || failure.status === 401)) &&
				failures < 3
		};
	});

	/** The last page that came back, and which narrowing it is a page of.
	 *
	 * Held so that a Next that does not answer leaves the reader looking at
	 * the page they were reading rather than at an empty panel: the rows are
	 * still true, and the thing that failed is the step. Kept per narrowing,
	 * because rows from a different search shown under this one's filters
	 * would be a list that does not match the controls above it. */
	let held = $state<{
		narrowing: string;
		page: number;
		rows: GuideHeadView[];
		total: number;
		hasNext: boolean;
	} | null>(null);

	$effect(() => {
		const answer = guides.data;
		if (answer === undefined) {
			return;
		}
		held = {
			narrowing,
			page: answer.page,
			rows: answer.guides,
			total: answer.total,
			hasNext: answer.has_next
		};
	});

	/** What the list draws: this question's answer, or the last page of this
	 *  same narrowing while the next one is in flight or has failed. */
	const shown = $derived.by(() => {
		const answer = guides.data;
		if (answer !== undefined) {
			return {
				page: answer.page,
				rows: answer.guides,
				total: answer.total,
				hasNext: answer.has_next
			};
		}
		return held !== null && held.narrowing === narrowing ? held : null;
	});
	/** Whether what is on the screen is a page other than the one the address
	 *  names. Said out loud rather than drawn as if current. */
	const stale = $derived(guides.data === undefined && shown !== null);
	/** Whether the server refused the ids in the address rather than failing to
	 *  answer. A retired taxon is not one of these — retirement hides a taxon
	 *  from the pickers and keeps every published guide that carries it
	 *  reachable — so this is a typo or a hand-edited link. A page ordinal and
	 *  an order are never refused: the server reads what it can of them and
	 *  says which order it answered in. */
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
			return;
		}
		// A Back or Forward within this list — the entries paging pushes — is a
		// restored question, and the fence inside `write()` only compares
		// paths. A timer scheduled before the traversal would therefore survive
		// it and replace the entry just restored with the search the reader
		// abandoned by pressing Back. History traversal wins over a pending
		// write instead.
		if (navigation.type === 'popstate') {
			cancel();
		}
	});

	$effect(() => cancel);

	/** Writes a question to the address.
	 *
	 * `push` is the difference between a control and a step: a filter is not a
	 * place, and a back button that walked every keystroke would be one, so
	 * narrowing replaces the entry. A page is a place — the reader went
	 * somewhere and expects Back to bring them to the page they came from — so
	 * paging pushes one. */
	function narrow(next: GuideFilters, settle = 0, push = false) {
		cancel();
		const from = page.url.pathname;
		const search = filterSearch(next);
		const to = search.length === 0 ? from : `${from}?${search}`;
		const write = () => {
			typing = null;
			if (left || page.url.pathname !== from) {
				return;
			}
			void goto(to, { replaceState: !push, keepFocus: true, noScroll: true });
		};
		if (settle === 0) {
			write();
			return;
		}
		typing = setTimeout(write, settle);
	}

	/** A change to one control, carrying whatever the others hold now, read
	 *  from the first page.
	 *
	 * The search box rather than the address for `q`: a topic chosen while a
	 * keystroke's timer is still out cancels that timer, and composing from the
	 * address would navigate with the previous search and then reset the box to
	 * it — the reader's half-typed query dropped by choosing a topic.
	 *
	 * Back to page one on every change, because page four of one narrowing is
	 * not a position in another: a reader on the last page who types a word
	 * would otherwise land on an empty page of results that do exist. */
	function choose(over: Partial<GuideFilters>) {
		narrow({
			q: box.trim(),
			topic: filters.topic,
			tags: filters.tags,
			sort: filters.sort,
			page: 1,
			...over
		});
	}

	function typed(next: string) {
		box = next;
		narrow({ ...filters, q: next.trim(), page: 1 }, SETTLE_MS);
	}

	/** One step through the pages of the narrowing being read.
	 *
	 * Held between one and whatever the answer says exists: Next is disabled
	 * where the server said there is no next page, and this clamp is what
	 * stops a second click landing past the end while the first is in flight. */
	function step(by: number) {
		// The box rather than the address for `q`, for the reason `choose()`
		// gives: a step taken while a keystroke's timer is still out cancels
		// that timer, and composing the step from the address would page the
		// previous search and then reset the box to it — the reader's typed
		// phrase silently dropped by pressing Next.
		//
		// A page of one narrowing is not a position in another, so a step that
		// commits new text reads that search from its first page rather than
		// from an ordinal that belonged to the old one.
		const asked = box.trim();
		const at = shown?.page ?? filters.page;
		const ordinal = asked === filters.q ? Math.max(1, at + by) : 1;
		narrow({ ...filters, q: asked, page: ordinal }, 0, true);
	}

	/** Every filter back to nothing, the search box included: a reset that
	 *  left the typed phrase in the box would clear the narrowing and then have
	 *  the next topic choice put the phrase straight back.
	 *
	 * The order is not a filter and survives this. The page cannot: the first
	 * page of the whole corpus is the only page this is asking for. */
	function reset() {
		box = '';
		narrow({ q: '', topic: null, tags: [], sort: filters.sort, page: 1 });
	}
</script>

<div class="page">
	<PageHead
		icon="book-open"
		title="Help and guides"
		description="Find a guide: search by title or topic."
	/>

	<div class="gd-find">
		<Field label="Search guides" id="guide-search">
			<!-- The console's search idiom: a bare input inside a decorated
			     wrapper that carries the border and the control height, with the
			     magnifier inside the control. -->
			<span class="gd-search">
				<Icon name="search" size={16} />
				<input
					id="guide-search"
					type="search"
					placeholder="Search by title or topic"
					value={box}
					oninput={(event) => typed(event.currentTarget.value)}
				/>
			</span>
		</Field>

		<!-- The sections as chips: one press narrows to one, and the chosen one
		     reads as chosen. The same `topic` the address carries. -->
		<div class="chip-row gd-topics" role="group" aria-label="Section">
			<button
				type="button"
				class="gd-topic-chip"
				aria-pressed={filters.topic === null}
				onclick={() => choose({ topic: null })}>All</button
			>
			{#each topics as topic (topic.id)}
				<button
					type="button"
					class="gd-topic-chip"
					aria-pressed={filters.topic === topic.id}
					disabled={taxonomy.isError}
					onclick={() => choose({ topic: filters.topic === topic.id ? null : topic.id })}
				>
					<Icon name={SECTIONS.find((section) => section.slug === topic.slug)?.icon ?? 'book-open'} size={14} />
					{taxonLabel(topic)}
				</button>
			{/each}
		</div>

		<details class="flow-more gd-more" open={filters.tags.length > 0}>
			<summary>Tags and order</summary>
			<div class="gd-filters">
				<div class="gd-filters-group">
					<span class="gd-group-label" id="guide-tags">Tags</span>
					<Menu bind:open={tagMenu} label="Filter by tag" align="start">
						{#snippet trigger()}
							<Button onclick={() => (tagMenu = !tagMenu)}>
								{filters.tags.length === 0 ? 'Any tag' : `${filters.tags.length} chosen`}
							</Button>
						{/snippet}
						<div class="gd-tag-menu" role="group" aria-labelledby="guide-tags">
							{#if taxonomy.isPending}
								<p class="none">Loading tags…</p>
							{:else if taxonomy.isError}
								<p class="none">We could not load the tags.</p>
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
									<p class="none">No guides have tags yet.</p>
								{/each}
							{/if}
						</div>
					</Menu>
				</div>

				<Field label="Order by" id="guide-sort">
					<select
						id="guide-sort"
						value={filters.sort}
						onchange={(event) => choose({ sort: event.currentTarget.value as GuideSort })}
					>
						{#each SORTS as order (order.id)}
							<option value={order.id}>{order.label}</option>
						{/each}
					</select>
				</Field>
			</div>
		</details>

		<div class="gd-filter-foot">
			<!-- The count is the server's count over the whole narrowing, not
			     the length of the page it sent: "25 guides match" printed over
			     a corpus of two hundred is the reason this is answered beside
			     the rows rather than measured off them. It is stated only once
			     a read is in, because "0 guides" is what an empty search says
			     and standing it in for a read that failed would state
			     something false. -->
			<span class="eyebrow">
				{#if shown !== null}
					{shown.total}
					{shown.total === 1 ? 'guide' : 'guides'}
					{narrowed ? 'match' : 'published'}
				{:else if guides.isPending}
					{narrowed ? 'Searching…' : 'Loading guides…'}
				{:else}
					No count yet
				{/if}
			</span>
			{#if narrowed}
				<Button tier="quiet" small onclick={reset}>
					Clear filters
				</Button>
			{/if}
		</div>
	</div>

	<div class="gd-results">
		{#if refusedFilters}
			<!-- A hand-edited address, or a link written against a topic or tag id
			     that never existed. The server refuses the ids rather than
			     answering a narrowing it cannot read, and this page says so
			     rather than quietly dropping them and showing a different list
			     than the one that was asked for. -->
			<Placeholder
				icon="book-open"
				headline="Those filters do not work"
				body="This link names a topic or tag we do not have. Clear the filters to see every
					guide."
			/>
			<div class="gd-filter-foot">
				<Button tier="outline" small onclick={reset}>
					Clear filters
				</Button>
			</div>
		{:else if shown === null && guides.isPending}
			<p class="quiet">Loading guides…</p>
		{:else if shown === null}
			<Placeholder
				icon="book-open"
				headline="We could not load the guides"
				body="Try clearing the filters or reloading the page."
			/>
			{#if narrowed}
				<div class="gd-filter-foot">
					<Button tier="outline" small onclick={reset}>
						Clear filters
					</Button>
				</div>
			{/if}
		{:else}
			{#if guides.isError}
				<!-- The rows below are the last page that answered, and they are
				     still true; what failed is the step to another one. Kept on
				     the screen with the pager and the filters as they were, so
				     Retry is one press rather than a reader's reconstruction of
				     where they had got to. -->
				<Banner tone="bad" title="That page did not load">
					{#snippet action()}
						<Button tier="outline" small onclick={() => guides.refetch()}>
							Retry
						</Button>
					{/snippet}
					Page {filters.page} did not load. You are still seeing page {shown.page}.
				</Banner>
			{/if}
			{#if shown.rows.length === 0 && shown.total > 0}
				<!-- An address past the end of the set: a real total and no rows
				     at this ordinal. Distinct from an empty search, because the
				     guides are there and the way to them is backwards. -->
				<Placeholder
					icon="book-open"
					headline="That page does not exist"
					body="There is no page {shown.page} for this search. Go back to the first page."
				/>
				<div class="gd-filter-foot">
					<Button
						tier="outline"
						small
						onclick={() => narrow({ ...filters, page: 1 }, 0, true)}
					>
						Go to the first page
					</Button>
				</div>
			{:else if shown.rows.length === 0 && narrowed}
				<!-- An empty search and an empty shelf are different answers, and the
				     first one has something to do about it. -->
				<Placeholder
					icon="book-open"
					headline="No guides match"
					body="No guide matches that search, topic and tag together. Try removing one of
						them."
				/>
				<div class="gd-filter-foot">
					<Button tier="outline" small onclick={reset}>
						Clear filters
					</Button>
				</div>
			{:else if shown.rows.length === 0}
				<Placeholder
					icon="book-open"
					headline="No guides yet"
					body="Guides will appear here: connecting a marketplace, importing your resources,
						and listing a resource everywhere."
				/>
			{:else}
				<div class="gd-sections" class:gd-stale={stale} aria-busy={guides.isFetching}>
					{#each groupBySection(shown.rows) as section (section.id)}
						<section class="gd-section">
							<h2 class="gd-section-head">
								<span class="gd-section-icon"><Icon name={section.icon} size={18} /></span>
								{section.name}
							</h2>
							<div class="gd-cards">
								{#each section.guides as guide (guide.slug)}
									<a class="gd-card" href={`/guides/${guide.slug}`}>
										<span class="gd-card-title">{guide.title}</span>
										<span class="when" title={utcInstant(guide.updated_at)}>
											Updated {agoLabel(guide.updated_at, now)}
										</span>
									</a>
								{/each}
							</div>
						</section>
					{/each}
				</div>
				<!-- The range is the rows on the screen and the total is the
				     server's count over the whole narrowing, so neither number
				     is the other's arithmetic. -->
				<Pagination
					page={shown.page}
					hasNext={shown.hasNext}
					busy={guides.isFetching}
					label="Guide pages"
					summary={pageSummary(shown.page, shown.rows.length, shown.total)}
					onprevious={() => step(-1)}
					onnext={() => step(1)}
				/>
			{/if}
		{/if}
	</div>
</div>
