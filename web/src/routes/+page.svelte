<script lang="ts">
	import { api, allPages, ApiFailure, type MappingHead, type ProductHead } from '$lib/api';
	import { visibleWindow } from '$lib/window';
	import { toast } from '$lib/toast';
	import { goto } from '$app/navigation';
	import type { InventoryId } from '$lib/generated/vocab';

	const ROW_HEIGHT = 44;
	const INVENTORIES: InventoryId[] = ['TesGb', 'TesNz'];

	let products = $state<ProductHead[]>([]);
	let mappings = $state<MappingHead[]>([]);
	let loaded = $state(false);
	let scrollTop = $state(0);
	let viewport = $state(600);
	let selected = $state<Set<string>>(new Set());
	let pendingKey: string | null = null;
	let syncing = $state(false);

	$effect(() => {
		void (async () => {
			const [productRows, mappingRows] = await Promise.all([
				allPages(api.products, (page) => page.products),
				api.mappings().then((view) => view.mappings)
			]);
			products = productRows;
			mappings = mappingRows;
			loaded = true;
		})();
	});

	const byProduct = $derived.by(() => {
		const index = new Map<string, Map<string, MappingHead>>();
		for (const mapping of mappings) {
			const cell = index.get(mapping.product) ?? new Map<string, MappingHead>();
			cell.set(mapping.inventory, mapping);
			index.set(mapping.product, cell);
		}
		return index;
	});

	const win = $derived(visibleWindow(products.length, ROW_HEIGHT, scrollTop, viewport));

	function toggle(mapping: MappingHead) {
		const next = new Set(selected);
		if (next.has(mapping.id)) {
			next.delete(mapping.id);
		} else {
			next.add(mapping.id);
		}
		selected = next;
	}

	async function startSync() {
		if (selected.size === 0) {
			return;
		}
		const chosen = mappings.filter(
			(mapping) => selected.has(mapping.id) && mapping.inventory === 'TesNz'
		);
		if (chosen.length !== selected.size) {
			toast('error', 'A sync targets one inventory; only TesNz cells are selectable for now.');
			return;
		}
		syncing = true;
		// One key per intent: a retry after a failure reuses it, so the retry
		// and the double-click are the same job on the server.
		pendingKey ??= crypto.randomUUID();
		try {
			const created = await api.createJob(
				'TesNz',
				chosen.map((mapping) => mapping.id),
				pendingKey
			);
			pendingKey = null;
			selected = new Set();
			toast('info', created.replay ? 'That sync already exists; showing it.' : 'Sync started.');
			goto(`/jobs/${created.job}`);
		} catch (failure) {
			if (failure instanceof ApiFailure && failure.code() === 'duplicate_sync_item') {
				pendingKey = null;
				toast(
					'error',
					'These files are already in the ledger unchanged; nothing needs re-uploading.'
				);
			} else {
				toast('error', 'The sync did not start; retrying will not double it.');
			}
		} finally {
			syncing = false;
		}
	}
</script>

<div class="mb-4 flex items-center gap-4">
	<h1 class="text-xl font-semibold">Products</h1>
	<span class="text-sm text-slate-500">{products.length} in the catalogue</span>
	<span class="grow"></span>
	<button
		class="rounded bg-slate-900 px-4 py-2 text-sm text-white disabled:opacity-40"
		disabled={selected.size === 0 || syncing}
		onclick={startSync}
	>
		{syncing ? 'Starting…' : `Sync ${selected.size} to Tes NZ`}
	</button>
</div>

{#if !loaded}
	<p class="text-slate-500">Loading the catalogue…</p>
{:else if products.length === 0}
	<p class="text-slate-500">No products yet; the pipeline ingests them.</p>
{:else}
	<div
		class="max-h-[70vh] overflow-y-auto rounded border border-slate-200 bg-white"
		onscroll={(event) => {
			scrollTop = event.currentTarget.scrollTop;
			viewport = event.currentTarget.clientHeight;
		}}
	>
		<table class="w-full text-sm">
			<thead class="sticky top-0 bg-slate-100 text-left">
				<tr>
					<th class="px-3 py-2">Product</th>
					{#each INVENTORIES as inventory (inventory)}
						<th class="px-3 py-2">{inventory}</th>
					{/each}
				</tr>
			</thead>
			<tbody>
				{#if win.padTop > 0}
					<tr style="height: {win.padTop}px"><td colspan={INVENTORIES.length + 1}></td></tr>
				{/if}
				{#each products.slice(win.start, win.end) as product (product.id)}
					<tr class="border-t border-slate-100" style="height: {ROW_HEIGHT}px">
						<td class="px-3 py-2 font-medium">{product.title}</td>
						{#each INVENTORIES as inventory (inventory)}
							{@const mapping = byProduct.get(product.id)?.get(inventory)}
							<td class="px-3 py-2">
								{#if mapping}
									<label class="flex items-center gap-2">
										{#if inventory === 'TesNz'}
											<input
												type="checkbox"
												checked={selected.has(mapping.id)}
												onchange={() => toggle(mapping)}
											/>
										{/if}
										<span class="rounded bg-slate-100 px-2 py-0.5 text-xs">
											{mapping.binding_state} · {mapping.lifecycle_state}
										</span>
									</label>
								{:else}
									<span class="text-xs text-slate-400">no mapping</span>
								{/if}
							</td>
						{/each}
					</tr>
				{/each}
				{#if win.padBottom > 0}
					<tr style="height: {win.padBottom}px"><td colspan={INVENTORIES.length + 1}></td></tr>
				{/if}
			</tbody>
		</table>
	</div>
{/if}
