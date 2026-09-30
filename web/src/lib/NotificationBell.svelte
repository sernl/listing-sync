<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { api, type NotificationsPage } from '$lib/api';
	import { BELL_SIZE, bellHost } from '$lib/bell-host.svelte';
	import { lightDismiss } from '$lib/dismiss';
	import Icon from '$lib/Icon.svelte';
	import {
		badgeLabel,
		inboxAfter,
		rows,
		type InboxChange,
		type NotificationRow
	} from '$lib/pages/notifications/view';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';

	// The bell and its list of the newest twenty. Drawn twice by the shell --
	// in the top strip above the phone breakpoint, where the list drops down
	// under the bell, and in the page header's phone tools below it, where the
	// list is a bottom sheet -- and both read one query, so the two never
	// disagree about the badge.

	let { placement }: { placement: 'bar' | 'phone' } = $props();

	const queryClient = useQueryClient();
	const bell = createQuery(() => ({
		queryKey: queryKeys.bell,
		queryFn: () => api.notifications(null, BELL_SIZE),
		enabled: bellHost.live,
		// A finished run lands on the server, not in this tab, so the badge
		// looks again every minute; a kept toast invalidates it at once.
		refetchInterval: 60_000
	}));

	const unread = $derived(bell.data?.unread ?? 0);
	const badge = $derived(badgeLabel(unread));
	// Ages are read when the list opens rather than ticking while it is shut.
	let now = $state(Date.now());
	const shown = $derived(rows(bell.data?.notifications ?? [], now));

	let open = $state(false);
	let root = $state<HTMLElement | null>(null);
	let trigger = $state<HTMLButtonElement | null>(null);
	let sheet = $state<HTMLDialogElement | null>(null);

	function show() {
		now = Date.now();
		open = true;
		void bell.refetch();
		if (placement === 'phone' && sheet !== null && !sheet.open) {
			sheet.showModal();
		}
	}

	function close(refocus = false) {
		open = false;
		if (sheet?.open) {
			sheet.close();
		}
		if (refocus) {
			trigger?.focus();
		}
	}

	// Above the phone line the list is a plain dropdown, so a press anywhere
	// outside the bell closes it, and Escape does too. Below it the list is a
	// modal sheet, which `lightDismiss` and the browser's own Escape close.
	$effect(() => {
		if (!open || placement !== 'bar') {
			return;
		}
		const pressed = (event: PointerEvent) => {
			if (root !== null && event.target instanceof Node && !root.contains(event.target)) {
				close();
			}
		};
		const keyed = (event: KeyboardEvent) => {
			if (event.key === 'Escape') {
				close(true);
			}
		};
		document.addEventListener('pointerdown', pressed);
		document.addEventListener('keydown', keyed);
		return () => {
			document.removeEventListener('pointerdown', pressed);
			document.removeEventListener('keydown', keyed);
		};
	});

	/** Applies the change to the list and the badge at once, then asks the
	 *  server; a refusal puts both back and says so. */
	async function apply(change: InboxChange, call: () => Promise<unknown>) {
		const before = queryClient.getQueryData<NotificationsPage>(queryKeys.bell);
		if (before !== undefined) {
			queryClient.setQueryData<NotificationsPage>(queryKeys.bell, {
				...before,
				...inboxAfter(before, change)
			});
		}
		try {
			await call();
		} catch {
			if (before !== undefined) {
				queryClient.setQueryData(queryKeys.bell, before);
			}
			toast('error', 'That change was not saved. Try again.');
		}
		await queryClient.invalidateQueries({ queryKey: queryKeys.bell });
	}

	const markRead = (id: string) =>
		apply({ kind: 'read', id, at: Date.now() }, () => api.markNotificationRead(id));
</script>

<div class="bell" bind:this={root}>
	<button
		bind:this={trigger}
		class={placement === 'phone' ? 'head-tool bell-btn' : 'bell-btn'}
		type="button"
		aria-label={unread > 0 ? `Notifications, ${unread} unread` : 'Notifications'}
		aria-haspopup="dialog"
		aria-expanded={open}
		onclick={() => (open ? close() : show())}
	>
		<Icon name="bell" size={placement === 'phone' ? 20 : 18} />
		{#if badge !== null}
			<span class="bell-badge" aria-hidden="true">{badge}</span>
		{/if}
	</button>

	{#if placement === 'bar'}
		{#if open}
			<div class="bell-pop" role="dialog" aria-label="Notifications">
				{@render panel()}
			</div>
		{/if}
	{:else}
		<dialog
			class="bell-sheet"
			aria-label="Notifications"
			bind:this={sheet}
			use:lightDismiss
			onclose={() => (open = false)}
		>
			{#if open}
				{@render panel()}
			{/if}
		</dialog>
	{/if}
</div>

{#snippet rowText(row: NotificationRow)}
	<span class="bell-title">
		{#if row.unread}<span class="sr-only">New: </span>{/if}{row.title}
	</span>
	{#if row.line !== ''}
		<span class="bell-line">{row.line}</span>
	{/if}
	<span class="bell-when">{row.when}</span>
{/snippet}

{#snippet panel()}
	<div class="bell-head">
		<h2>Notifications</h2>
		<button type="button" class="bell-link" disabled={unread === 0}
			onclick={() =>
				apply({ kind: 'readAll', at: Date.now() }, () => api.markAllNotificationsRead())}
		>
			Mark all read
		</button>
		{#if placement === 'phone'}
			<button type="button" class="bell-close" aria-label="Close" onclick={() => close(true)}>
				<Icon name="x" size={18} />
			</button>
		{/if}
	</div>
	{#if bell.isPending}
		<p class="bell-said">Loading…</p>
	{:else if bell.isError}
		<p class="bell-said">We could not load your notifications. Try again in a moment.</p>
	{:else if shown.length === 0}
		<p class="bell-said">Nothing yet. What Teachouse does for you shows here.</p>
	{:else}
		<ul class="bell-list">
			{#each shown as row (row.id)}
				<li class="bell-row" class:is-unread={row.unread}>
					<span class="bell-dot {row.tone}" aria-hidden="true"></span>
					{#if row.href !== null}
						<a
							class="bell-main"
							href={row.href}
							onclick={() => {
								if (row.unread) {
									void markRead(row.id);
								}
								close();
							}}
						>
							{@render rowText(row)}
						</a>
					{:else}
						<div class="bell-main">{@render rowText(row)}</div>
					{/if}
					<span class="bell-acts">
						{#if row.unread}
							<button type="button" class="bell-act" onclick={() => markRead(row.id)}>
								Mark read<span class="sr-only">: {row.title}</span>
							</button>
						{/if}
						<button type="button" class="bell-act" onclick={() => apply({ kind: 'dismiss', id: row.id }, () => api.dismissNotification(row.id))}
						>
							Dismiss<span class="sr-only">: {row.title}</span>
						</button>
					</span>
				</li>
			{/each}
		</ul>
	{/if}
	<a class="bell-all" href="/notifications" onclick={() => close()}>See all notifications</a>
{/snippet}

<style>
	.bell {
		position: relative;
		flex: none;
	}

	/* The strip's bell is the avatar's size, as a circle: the two sit side by
	   side at the end of the strip. On a phone it is a `head-tool`, the 44px
	   circle every control in the header band is. */
	.bell-btn:not(.head-tool) {
		width: var(--avatar-size);
		height: var(--avatar-size);
		border-radius: 50%;
		border: 1px solid var(--line);
		background: var(--surface);
		color: var(--muted);
		display: grid;
		place-items: center;
		padding: 0;
		cursor: pointer;
	}

	.bell-btn {
		position: relative;
	}

	.bell-btn:hover {
		color: var(--text);
		background: var(--hover);
	}

	.bell-btn:focus-visible {
		outline: 2px solid var(--primary);
		outline-offset: 2px;
	}

	.bell-badge {
		position: absolute;
		top: -4px;
		right: -6px;
		min-width: 18px;
		height: 18px;
		padding: 0 5px;
		border-radius: var(--r-pill);
		background: var(--bad);
		color: var(--on-fill);
		border: 2px solid var(--surface);
		font: 700 10.5px/14px var(--sans);
		text-align: center;
		box-sizing: border-box;
	}

	.bell-pop {
		position: absolute;
		top: calc(100% + 8px);
		right: 0;
		z-index: 30;
		width: min(380px, calc(100vw - 32px));
		max-height: min(560px, calc(100vh - 96px));
		display: flex;
		flex-direction: column;
		background: var(--surface);
		border: 1px solid var(--line);
		border-radius: var(--r-panel);
		box-shadow: var(--sh-3);
		overflow: hidden;
	}

	/* The phone's sheet: pinned to the bottom edge, full width, rounded on
	   top only, and never taller than most of the screen. */
	.bell-sheet {
		position: fixed;
		inset: auto 0 0 0;
		margin: 0;
		width: 100%;
		max-width: 100%;
		max-height: 80vh;
		padding: 0 0 env(safe-area-inset-bottom, 0px);
		border: 1px solid var(--line);
		border-bottom: 0;
		border-radius: var(--r-panel) var(--r-panel) 0 0;
		background: var(--surface);
		color: var(--text);
		box-shadow: var(--sh-3);
	}

	.bell-sheet[open] {
		display: flex;
		flex-direction: column;
	}

	.bell-sheet::backdrop {
		background: color-mix(in srgb, var(--text) 35%, transparent);
	}

	.bell-head {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 12px 12px 10px 16px;
		border-bottom: 1px solid var(--line);
	}

	.bell-head h2 {
		flex: 1;
		margin: 0;
		font-size: 14px;
		font-weight: 700;
	}

	.bell-link,
	.bell-act {
		border: 0;
		background: none;
		padding: 4px 6px;
		border-radius: 6px;
		font: inherit;
		font-size: 12px;
		font-weight: 600;
		color: var(--primary);
		cursor: pointer;
	}

	.bell-act {
		color: var(--muted);
	}

	.bell-link:hover:not(:disabled),
	.bell-act:hover {
		background: var(--hover);
		color: var(--text);
	}

	.bell-link:disabled {
		color: var(--muted);
		cursor: default;
		opacity: 0.6;
	}

	.bell-close {
		display: grid;
		place-items: center;
		width: var(--control-h);
		height: var(--control-h);
		border: 0;
		border-radius: 50%;
		background: none;
		color: var(--muted);
		cursor: pointer;
	}

	.bell-said {
		margin: 0;
		padding: 20px 16px;
		font-size: 13px;
		color: var(--muted);
	}

	.bell-list {
		list-style: none;
		margin: 0;
		padding: 0;
		overflow-y: auto;
		flex: 1;
		min-height: 0;
	}

	.bell-row {
		display: grid;
		grid-template-columns: auto minmax(0, 1fr);
		column-gap: 10px;
		padding: 10px 12px 8px 16px;
		border-bottom: 1px solid var(--line);
	}

	.bell-row.is-unread {
		background: color-mix(in srgb, var(--accent-soft) 45%, var(--surface));
	}

	.bell-dot {
		width: 8px;
		height: 8px;
		margin-top: 6px;
		border-radius: 50%;
		background: var(--primary);
	}

	.bell-dot.success {
		background: var(--accent);
	}

	.bell-dot.warning {
		background: var(--warn);
	}

	.bell-dot.error {
		background: var(--bad);
	}

	.bell-main {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
		color: inherit;
		text-decoration: none;
	}

	a.bell-main:hover .bell-title {
		text-decoration: underline;
	}

	.bell-title {
		font-size: 13px;
		font-weight: 500;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.is-unread .bell-title {
		font-weight: 700;
	}

	.bell-line {
		font-size: 12px;
		color: var(--muted);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.bell-when {
		font-size: 11.5px;
		color: var(--muted);
	}

	.bell-acts {
		grid-column: 2;
		display: flex;
		gap: 4px;
		margin: 2px 0 0 -6px;
	}

	.bell-all {
		display: block;
		padding: 12px 16px;
		text-align: center;
		font-size: 13px;
		font-weight: 600;
		color: var(--primary);
		text-decoration: none;
	}

	.bell-all:hover {
		background: var(--hover);
	}
</style>
