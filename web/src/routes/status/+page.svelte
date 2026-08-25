<script lang="ts">
	import { api, type InventoryStatus } from '$lib/api';

	let inventories = $state<InventoryStatus[]>([]);
	let loaded = $state(false);

	$effect(() => {
		void api.status().then((view) => {
			inventories = view.inventories;
			loaded = true;
		});
	});
</script>

<h1 class="mb-4 text-xl font-semibold">Marketplace status</h1>

{#if !loaded}
	<p class="text-slate-500">Loading…</p>
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
