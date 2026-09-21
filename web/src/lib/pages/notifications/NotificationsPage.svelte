<script lang="ts">
	import { api, type NotificationView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { readState } from '$lib/pages/automations/read-state';
	import { NOTHING_TO_CHANGE, heldAfter, readThrough, rows, unreadLine } from './view';
	import './notifications.css';

	let held = $state<NotificationView[]>([]);
	let nextCursor = $state<string | null>(null);
	let loaded = $state(false);
	// A list that could not be read is not a seller who has finished nothing:
	// the two are separated here for the same reason every other read in this
	// console separates them.
	let failed = $state(false);
	// Distinct from `failed`, because the two are different sentences. A first
	// page that could not be read leaves nothing to draw; a continuation that
	// failed leaves the page already read on screen, and telling that seller
	// "nothing has changed" would be false of the rows in front of them.
	let moreFailed = $state(false);
	let loadingMore = $state(false);

	// `Date.now()` inside the derivation rather than captured at init, so an
	// age recomputes with its list instead of freezing at mount.
	const shown = $derived(rows(held, Date.now()));
	const read = $derived(readState(loaded, failed, shown));

	async function loadPage(cursor?: string | null) {
		const first = cursor === undefined || cursor === null;
		try {
			const view = await api.notifications(cursor ?? undefined);
			held = heldAfter(held, view.notifications, cursor);
			nextCursor = view.next_cursor;
			failed = false;
			moreFailed = false;
			if (first) {
				markRead();
			}
		} catch {
			// The rows already held stay held: a page that failed to load its
			// second page has still read its first, and throwing those away
			// would turn one unreadable page into an empty inbox.
			if (first) {
				failed = true;
			} else {
				moreFailed = true;
			}
		}
		loaded = true;
	}

	// Read on arrival rather than behind a control. The rail carries no unread
	// badge, so the mark has exactly one reader — the dots on this page — and a
	// seller who has just looked at the list has read it. The rows keep the
	// state they arrived with for the length of the visit, so the dots do not
	// vanish from under the eye that came to see them.
	//
	// The post reaches further than the page: the route marks every unread row
	// of the organisation at or before the id given, in the list's own order,
	// so posting the newest marks the whole inbox and not just the rows
	// rendered. That is intended, and it is what keeps the eyebrow above the
	// list truthful — it counts the unread among the rows loaded, so a narrower
	// mark would leave later pages unread and make that figure rise when the
	// seller pressed Load more. Posting the oldest id on the page is the
	// narrower alternative if that is ever wanted.
	function markRead() {
		const through = readThrough(held);
		if (through !== null) {
			void api.markNotificationsRead(through).catch(() => {
				// Nothing to tell the seller: the list they came for is on
				// screen, and an unmarked row is read again on the next visit.
			});
		}
	}

	async function loadMore() {
		loadingMore = true;
		await loadPage(nextCursor);
		loadingMore = false;
	}

	$effect(() => {
		void loadPage();
	});
</script>

<div class="page ntf-page">
	<PageHead
		icon="bell"
		title="Notifications"
		description="Everything Teachouse has finished for you, newest first."
		guide="updates"
	/>

	{#if read.kind === 'pending'}
		<p class="ntf-said">Loading…</p>
	{:else if read.kind === 'failed'}
		<Banner tone="bad" title="We could not read your notifications">
			We could not load the list, and nothing has changed.
			{#snippet action()}
				<Button
					icon="refresh-cw"
					onclick={() => {
						loaded = false;
						failed = false;
						void loadPage();
					}}>Try again</Button
				>
			{/snippet}
		</Banner>
	{:else if read.kind === 'empty'}
		<Placeholder
			icon="bell"
			headline="Nothing has finished yet"
			body="An import or an update is listed here when it finishes."
		>
			{#snippet actions()}
				<Button tier="outline" href="/sync" icon="refresh-cw">Go to Updates</Button>
			{/snippet}
		</Placeholder>
	{:else}
		<p class="ntf-total">{unreadLine(shown)}</p>
		<ul class="ntf-list">
			{#each read.rows as row (row.id)}
				<li class="ntf-row" class:is-unread={row.unread}>
					<svelte:element
						this={row.href === null ? 'div' : 'a'}
						class="ntf-open"
						href={row.href ?? undefined}
					>
						<!-- The mark is decoration: a colour a screen reader cannot see
						     is not a state, so an unread row says the word as well. The
						     read row keeps the mark's column so that marking one read
						     does not shift every line of text left. -->
						<span
							class={row.unread ? 'ntf-unread' : 'ntf-unread ntf-read'}
							aria-hidden="true"
						></span>
						<span class="ntf-main">
							<span class="ntf-title">
								{#if row.unread}<span class="sr-only">New: </span>{/if}{row.title}
							</span>
							{#if row.outcomes.length === 0}
								<span class="ntf-quiet">{NOTHING_TO_CHANGE}</span>
							{:else}
								<span class="ntf-counts">
									{#each row.outcomes as outcome (outcome.label)}
										<span class="ntf-count">
											<i class={outcome.tone} aria-hidden="true"></i>
											{outcome.count}
											{outcome.label}
										</span>
									{/each}
								</span>
							{/if}
						</span>
						<span class="ntf-when">{row.when}</span>
					</svelte:element>
				</li>
			{/each}
		</ul>

		{#if moreFailed}
			<Banner tone="bad" title="The next page could not be read">
				Only the page after the runs above failed to load.
				{#snippet action()}
					<Button icon="refresh-cw" disabled={loadingMore} onclick={loadMore}>Try again</Button>
				{/snippet}
			</Banner>
		{:else if nextCursor !== null}
			<div class="ntf-more">
				<Button
					icon="chevron-down"
					disabled={loadingMore}
					reason={loadingMore ? 'The next page is loading.' : undefined}
					onclick={loadMore}
				>
					{loadingMore ? 'Loading…' : 'Load more'}
				</Button>
			</div>
		{/if}
	{/if}
</div>
