<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import { queryKeys } from '$lib/query';

	const status = createQuery(() => ({
		queryKey: queryKeys.status,
		queryFn: () => api.status()
	}));

	const inventories = $derived(status.data?.inventories ?? []);
</script>

<h1 class="mb-4 text-xl font-semibold">Marketplace status</h1>

{#if status.isPending}
	<p class="text-slate-500">Loading…</p>
{:else if status.isError}
	<p class="text-slate-500">The status could not be read.</p>
{:else}
	<ul class="divide-y divide-slate-100 rounded border border-slate-200 bg-white">
		{#each inventories as entry (entry.inventory)}
			<li class="flex items-center gap-4 px-4 py-3">
				<span class="font-medium">{entry.inventory}</span>
				<span class="text-xs text-slate-500">{entry.marketplace}</span>
				<span class="grow"></span>
				{#if entry.halted}
					<span class="rounded bg-red-100 px-2 py-0.5 text-xs text-red-800">
						halted{entry.reason ? ` — ${entry.reason}` : ''}
					</span>
				{:else}
					<span class="rounded bg-emerald-100 px-2 py-0.5 text-xs text-emerald-800">
						operating
					</span>
				{/if}
			</li>
		{/each}
	</ul>
	<p class="mt-3 text-xs text-slate-500">
		This page needs no sign-in, because it matters most when signing in is
		what is broken.
	</p>
{/if}
