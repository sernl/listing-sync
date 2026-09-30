<script lang="ts">
	import { useQueryClient } from '@tanstack/svelte-query';
	import { api, type NotificationView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { readState } from '$lib/pages/automations/read-state';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import '$lib/styles/data.css';
	import { heldAfter, inboxAfter, rows, unreadLine, type InboxChange } from './view';
	import './notifications.css';

	const queryClient = useQueryClient();

	let held = $state<NotificationView[]>([]);
	let unread = $state(0);
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
	const anyRead = $derived(held.some((one) => one.read_at !== null));

	async function loadPage(cursor?: string | null) {
		const first = cursor === undefined || cursor === null;
		try {
			const view = await api.notifications(cursor ?? undefined);
			held = heldAfter(held, view.notifications, cursor);
			unread = view.unread;
			nextCursor = view.next_cursor;
			failed = false;
			moreFailed = false;
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

	async function loadMore() {
		loadingMore = true;
		await loadPage(nextCursor);
		loadingMore = false;
	}

	/** Moves the rows and the count at once, then asks the server; a refusal
	 *  puts both back. The bell reads the same inbox, so it is told either
	 *  way. */
	async function apply(change: InboxChange, call: () => Promise<unknown>) {
		const before = { notifications: held, unread };
		const after = inboxAfter(before, change);
		held = after.notifications;
		unread = after.unread;
		try {
			await call();
		} catch {
			held = before.notifications;
			unread = before.unread;
			toast('error', 'That change was not saved. Try again.');
		}
		await queryClient.invalidateQueries({ queryKey: queryKeys.bell });
	}

	$effect(() => {
		void loadPage();
	});
</script>

<div class="page ntf-page">
	<PageHead
		icon="bell"
		title="Notifications"
		description="Everything Teachouse has finished for you, and anything you missed, newest first."
		guide="updates"
	>
		{#snippet aside()}
			<Button
				tier="outline"
				icon="check"
				disabled={unread === 0}
				reason={unread === 0 ? 'Nothing is unread.' : undefined}
				onclick={() =>
					apply({ kind: 'readAll', at: Date.now() }, () => api.markAllNotificationsRead())}
			>
				Mark all read
			</Button>
			<Button
				tier="outline"
				icon="trash-2"
				disabled={!anyRead}
				reason={anyRead ? undefined : 'Nothing here has been read yet.'}
				onclick={() => apply({ kind: 'dismissRead' }, () => api.dismissReadNotifications())}
			>
				Delete read
			</Button>
		{/snippet}
	</PageHead>

	{#if read.kind === 'pending'}
		<p class="ntf-said">Loading…</p>
	{:else if read.kind === 'failed'}
		<Banner tone="bad" title="We could not load your notifications">
			Nothing is lost. Try again in a moment.
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
			headline="Nothing here yet"
			body="Each import or update shows here when it finishes, and so does any message you missed."
		>
			{#snippet actions()}
				<Button tier="outline" href="/sync" icon="refresh-cw">Go to Updates</Button>
			{/snippet}
		</Placeholder>
	{:else}
		<p class="ntf-total">{unreadLine(unread)}</p>
		<div class="data-table-wrap">
			<table class="data-table stack ntf-table">
				<thead>
					<tr>
						<th>Notification</th>
						<th>When</th>
						<th class="act"><span class="sr-only">Actions</span></th>
					</tr>
				</thead>
				<tbody>
					{#each read.rows as row (row.id)}
						<tr class:is-unread={row.unread}>
							<td data-label="">
								<span class="ntf-cell">
									<!-- The dot is the row's tone; a colour a screen reader
									     cannot see is not a state, so an unread row says the
									     word as well. -->
									<span class="ntf-dot {row.tone}" aria-hidden="true"></span>
									<span class="ntf-main">
										{#if row.href !== null}
											<a class="ntf-title" href={row.href}>
												{#if row.unread}<span class="sr-only">New: </span>{/if}{row.title}
											</a>
										{:else}
											<span class="ntf-title">
												{#if row.unread}<span class="sr-only">New: </span>{/if}{row.title}
											</span>
										{/if}
										{#if row.outcomes.length > 0}
											<span class="ntf-counts">
												{#each row.outcomes as outcome (outcome.label)}
													<span class="ntf-count">
														<i class={outcome.tone} aria-hidden="true"></i>
														{outcome.count}
														{outcome.label}
													</span>
												{/each}
											</span>
										{:else if row.line !== ''}
											<span class="ntf-quiet">{row.line}</span>
										{/if}
									</span>
								</span>
							</td>
							<td data-label="When" class="nowrap ntf-when">{row.when}</td>
							<td data-label="" class="act">
								{#if row.unread}
									<button
										type="button"
										class="ntf-act"
										onclick={() =>
											apply({ kind: 'read', id: row.id, at: Date.now() }, () =>
												api.markNotificationRead(row.id)
											)}
									>
										Mark read<span class="sr-only">: {row.title}</span>
									</button>
								{/if}
								<button
									type="button"
									class="ntf-act"
									onclick={() =>
										apply({ kind: 'dismiss', id: row.id }, () => api.dismissNotification(row.id))}
								>
									Delete<span class="sr-only">: {row.title}</span>
								</button>
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>

		{#if moreFailed}
			<Banner tone="bad" title="We could not load more notifications">
				The ones above are still here.
				{#snippet action()}
					<Button icon="refresh-cw" disabled={loadingMore} onclick={loadMore}>Try again</Button>
				{/snippet}
			</Banner>
		{:else if nextCursor !== null}
			<div class="ntf-more">
				<Button
					icon="chevron-down"
					disabled={loadingMore}
					reason={loadingMore ? 'Loading more…' : undefined}
					onclick={loadMore}
				>
					{loadingMore ? 'Loading…' : 'Load more'}
				</Button>
			</div>
		{/if}
	{/if}
</div>
