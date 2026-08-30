<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { allPages, api, ApiFailure, type MappingHead } from '$lib/api';
	import { formatMetric } from '$lib/analytics-view';
	import { agoLabel } from '$lib/elapsed';
	import {
		badgesFor,
		formatPrice,
		matchesQuery,
		metricsByProduct,
		normaliseQuery,
		rowStatus
	} from '$lib/listings-view';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import { visibleWindow } from '$lib/window';

	const ROW_HEIGHT = 52;
	const COLUMNS = 8;

	const catalogue = createQuery(() => ({
		queryKey: queryKeys.products,
		queryFn: () => allPages(api.products, (view) => view.products)
	}));
	const mappings = createQuery(() => ({
		queryKey: queryKeys.mappings,
		queryFn: () => api.mappings().then((view) => view.mappings)
	}));
	// The captured figures are read separately on purpose: a capture that is
	// slow or missing costs the table two columns rather than costing it every
	// row. It shares the analytics page's cache entry.
	const captured = createQuery(() => ({
		queryKey: queryKeys.analytics,
		queryFn: () => api.analytics()
	}));

	let scrollTop = $state(0);
	let viewport = $state(600);
	let selected = $state<Set<string>>(new Set());
	let pendingKey: string | null = null;
	let syncing = $state(false);

	const products = $derived(catalogue.data ?? []);
	const query = $derived(normaliseQuery(page.url.searchParams.get('q')));
	const shown = $derived(products.filter((product) => matchesQuery(product.title, query)));

	const byProduct = $derived.by(() => {
		const index = new Map<string, MappingHead[]>();
		for (const mapping of mappings.data ?? []) {
			index.set(mapping.product, [...(index.get(mapping.product) ?? []), mapping]);
		}
		return index;
	});

	const figures = $derived(
		metricsByProduct(captured.data?.listings ?? [], mappings.data ?? [])
	);

	const rows = $derived.by(() => {
		const now = Date.now();
		return shown.map((product) => {
			const owned = byProduct.get(product.id) ?? [];
			const metrics = figures.get(product.id);
			return {
				product,
				badges: badgesFor(owned),
				status: rowStatus(owned),
				price: formatPrice(product.price),
				views: formatMetric(metrics?.views ?? undefined),
				sales: formatMetric(metrics?.sales ?? undefined),
				updated: agoLabel(product.updated_at, now),
				syncable: owned.find((mapping) => mapping.inventory === 'TesNz')
			};
		});
	});

	const win = $derived(visibleWindow(rows.length, ROW_HEIGHT, scrollTop, viewport));

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
		const chosen = (mappings.data ?? []).filter(
			(mapping) => selected.has(mapping.id) && mapping.inventory === 'TesNz'
		);
		if (chosen.length !== selected.size) {
			toast('error', 'A sync targets one inventory; only Tes NZ cells are selectable for now.');
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
			goto(`/sync/${created.job}`);
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

<div class="page">
	<PageHead
		icon="▤"
		title="Listings"
		description="Every resource once — Teachouse keeps each marketplace matching it."
	>
		{#snippet aside()}
			<span class="tag-note">
				{#if query}
					{rows.length} of {products.length} matching “{query}”
				{:else}
					{products.length} in the catalogue
				{/if}
			</span>
			<button class="btn" disabled={selected.size === 0 || syncing} onclick={startSync}>
				{syncing ? 'Starting…' : `Sync ${selected.size} to Tes NZ`}
			</button>
			<a class="cta" href="/listings/new">New listing</a>
		{/snippet}
	</PageHead>

	<Panel>
		{#if catalogue.isPending || mappings.isPending}
			<p class="quiet">Loading the catalogue…</p>
		{:else if catalogue.isError || mappings.isError}
			<p class="quiet">The catalogue could not be read.</p>
		{:else if products.length === 0}
			<div class="placeholder">
				<span class="big" aria-hidden="true">▤</span>
				<b>No products yet</b>
				<p>
					Author one here, or let an import bring your existing listings in. Once a resource is in
					your catalogue it appears here, and sync keeps every marketplace matching it.
				</p>
				<div class="actions" style="justify-content: center; margin-top: 14px">
					<a class="cta" href="/listings/new">New listing</a>
				</div>
			</div>
		{:else if rows.length === 0}
			<div class="placeholder">
				<span class="big" aria-hidden="true">⌕</span>
				<b>Nothing matches “{query}”</b>
				<p>Search reads listing titles. Clear the box to see the whole catalogue again.</p>
			</div>
		{:else}
			<div
				class="tbl-wrap scroll-tbl"
				onscroll={(event) => {
					scrollTop = event.currentTarget.scrollTop;
					viewport = event.currentTarget.clientHeight;
				}}
			>
				<table>
					<thead>
						<tr>
							<th><span class="sr-only">Select for sync</span></th>
							<th>Listing</th>
							<th>Platforms</th>
							<th>Status</th>
							<th class="num">Price</th>
							<th class="num">Views</th>
							<th class="num">Sales</th>
							<th class="num">Updated</th>
						</tr>
					</thead>
					<tbody>
						{#if win.padTop > 0}
							<tr style="height: {win.padTop}px"><td colspan={COLUMNS}></td></tr>
						{/if}
						{#each rows.slice(win.start, win.end) as row (row.product.id)}
							<tr style="height: {ROW_HEIGHT}px">
								<td>
									{#if row.syncable}
										{@const cell = row.syncable}
										<input
											type="checkbox"
											aria-label={`Select ${row.product.title} for a Tes NZ sync`}
											checked={selected.has(cell.id)}
											onchange={() => toggle(cell)}
										/>
									{/if}
								</td>
								<td class="title-cell">
									<a class="t" href={`/listings/${row.product.id}`} title={row.product.title}>
										{row.product.title}
									</a>
								</td>
								<td>
									{#each row.badges as badge (badge.inventory)}
										<span class="badge {badge.live ? 'live' : ''}" title={badge.state}
											>{badge.label}</span
										>
									{:else}
										<span class="s">—</span>
									{/each}
								</td>
								<td><span class="pill {row.status.tone}">{row.status.label}</span></td>
								<td class="num">{row.price}</td>
								<td class="num">{row.views}</td>
								<td class="num">{row.sales}</td>
								<td class="num">{row.updated}</td>
							</tr>
						{/each}
						{#if win.padBottom > 0}
							<tr style="height: {win.padBottom}px"><td colspan={COLUMNS}></td></tr>
						{/if}
					</tbody>
				</table>
			</div>
			<p class="foot-note">
				Views and sales come from the analytics capture and carry its age, not a live check.
				{#if captured.isError}
					The capture could not be read just now, so both columns are blank rather than zero.
				{/if}
			</p>
		{/if}
	</Panel>
</div>
