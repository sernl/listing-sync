<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import { agoLabel } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { queryKeys } from '$lib/query';

	const status = createQuery(() => ({
		queryKey: queryKeys.status,
		queryFn: () => api.status()
	}));

	const inventories = $derived(status.data?.inventories ?? []);
	const halted = $derived(inventories.filter((entry) => entry.halted).length);
</script>

<div class="page">
	<PageHead
		icon="◉"
		title="Marketplace status"
		description="Whether each marketplace is accepting work right now."
	>
		{#snippet aside()}
			<span class="tag-note">
				{#if status.isSuccess}
					{halted === 0 ? 'all operating' : `${halted} halted`}
				{:else}
					reading…
				{/if}
			</span>
		{/snippet}
	</PageHead>

	<Panel>
		{#if status.isPending}
			<p class="quiet">Loading…</p>
		{:else if status.isError}
			<p class="quiet">The status could not be read.</p>
		{:else if inventories.length === 0}
			<div class="placeholder">
				<span class="big" aria-hidden="true">◉</span>
				<b>No marketplace is configured yet</b>
				<p>Each inventory the engine works against appears here with its own state.</p>
			</div>
		{:else}
			{#each inventories as entry (entry.inventory)}
				<div class="row">
					<span class="t">{entry.inventory}</span>
					<span class="badge">{entry.marketplace}</span>
					<span class="grow"></span>
					{#if entry.halted}
						<span class="s">{entry.reason ?? 'no reason recorded'}</span>
						{#if entry.raised_at !== undefined}
							<span class="when">since {agoLabel(entry.raised_at, Date.now())}</span>
						{/if}
						<span class="pill bad">halted</span>
					{:else}
						<span class="pill ok">operating</span>
					{/if}
				</div>
			{/each}
			<p class="foot-note">
				This page needs no sign-in, because it matters most when signing in is what is broken.
			</p>
		{/if}
	</Panel>
</div>
