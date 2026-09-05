<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { statusLine } from '$lib/pages/account/status-line';
	import '$lib/pages/account/account.css';

	const status = createQuery(() => ({
		queryKey: queryKeys.status,
		queryFn: () => api.status()
	}));

	const inventories = $derived(status.data?.inventories ?? []);
	const halted = $derived(inventories.filter((entry) => entry.halted).length);
	const now = Date.now();
</script>

<div class="page">
	<PageHead
		icon="activity"
		title="Marketplace status"
		description="Whether each marketplace is accepting work right now."
	>
		{#snippet aside()}
			<!-- Three answers, because a failed read is not an ongoing one. This
			     page matters most when other things are broken, so it must not
			     be the one describing a finished failure as still in flight. -->
			{#if status.isError}
				<StatusPill tone="bad" label="unread" />
			{:else if status.isSuccess}
				<StatusPill
					tone={halted === 0 ? 'ok' : 'warn'}
					label={halted === 0 ? 'all operating' : `${halted} halted`}
				/>
			{:else}
				<StatusPill tone="soon" label="reading" />
			{/if}
		{/snippet}
	</PageHead>

	<Panel>
		{#if status.isPending}
			<p class="quiet">Loading…</p>
		{:else if status.isError}
			<p class="quiet">The status could not be read.</p>
		{:else if inventories.length === 0}
			<Placeholder
				icon="activity"
				headline="No marketplace is configured yet"
				body="Each inventory the engine works against appears here with its own state."
			/>
		{:else}
			{#each inventories as entry (entry.inventory)}
				<div class="acct-state-row">
					<span class="who">
						<span class="t">{entry.inventory}</span>
						<span class="why">{statusLine(entry, now)}</span>
					</span>
					<StatusPill
						tone={entry.halted ? 'bad' : 'ok'}
						label={entry.halted ? 'halted' : 'operating'}
					/>
				</div>
			{/each}
			<p class="foot-note">
				This page needs no sign-in, because it matters most when signing in is what is broken.
			</p>
		{/if}
	</Panel>
</div>
