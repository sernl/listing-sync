<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { allPages, api, type MappingHead } from '$lib/api';
	import { formatMetric } from '$lib/analytics-view';
	import CrossListDialog from '$lib/CrossListDialog.svelte';
	import MarkListedDialog from '$lib/MarkListedDialog.svelte';
	import { agoLabel } from '$lib/elapsed';
	import {
		STATE_LABEL,
		WORK_RUNS,
		inventoryTally,
		matchesFilters,
		newestWork,
		rowFor,
		type Filters,
		type InventoryRow,
		type MarketplaceState,
		type StandingFilter,
		type WorkItem
	} from '$lib/inventory';
	import { BULK_ACTIONS, unavailable, type BulkVerb } from '$lib/bulk-verbs';
	import BulkDeleteDialog from '$lib/BulkDeleteDialog.svelte';
	import { formatPrice, metricsByProduct } from '$lib/listings-view';
	import MarketplaceChips from '$lib/MarketplaceChips.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { AUTHORABLE_PLATFORMS, platformTitle } from '$lib/platforms';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import { visibleWindow } from '$lib/window';
	import type { InventoryId } from '$lib/generated/vocab';

	const ROW_HEIGHT = 60;
	/** One card is taller than one table row, and the windowing is told which
	 *  it is measuring: `visibleWindow` derives every scroll spacer from the
	 *  row height it is given, so a card rendered at one height and counted at
	 *  another drifts the scrollbar by the difference on every row. Matched to
	 *  `.card-tbl tr` in `app.css`. */
	const PHONE_ROW_HEIGHT = 124;
	/** The one breakpoint the phone layout uses, written here as well because
	 *  a media query is the only way JavaScript can read it. */
	const PHONE_QUERY = '(max-width: 620px)';
	const COLUMNS = 8;

	const queryClient = useQueryClient();

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
	const connections = createQuery(() => ({
		queryKey: queryKeys.connections,
		queryFn: () => api.connections().then((view) => view.connections)
	}));
	const halts = createQuery(() => ({
		queryKey: queryKeys.status,
		queryFn: () => api.status().then((view) => view.inventories)
	}));
	// What is happening to each mapping right now, which no endpoint answers
	// directly: an item is reachable only through the run that holds it, so the
	// newest runs are read and the newest item per mapping is kept.
	const work = createQuery(() => ({
		queryKey: queryKeys.inventoryWork,
		queryFn: async () => {
			const first = await api.jobs();
			const heads = first.jobs.slice(0, WORK_RUNS);
			const pages = await Promise.all(heads.map((head) => api.items(head.job)));
			const items: WorkItem[] = heads.flatMap((head, index) =>
				pages[index].items.map((item) => ({ job: head.job, item }))
			);
			return newestWork(items);
		}
	}));

	let scrollTop = $state(0);
	let viewport = $state(600);
	// Safe to read at init: the layout is `ssr = false`, so this component
	// never renders without a window.
	const phoneQuery = window.matchMedia(PHONE_QUERY);
	let onPhone = $state(phoneQuery.matches);
	$effect(() => {
		const watch = (event: MediaQueryListEvent) => {
			onPhone = event.matches;
		};
		phoneQuery.addEventListener('change', watch);
		return () => phoneQuery.removeEventListener('change', watch);
	});
	let selected = $state<Set<string>>(new Set());
	let marketplace = $state<InventoryId | 'all'>('all');
	let standing = $state<StandingFilter>('all');
	let crossListing = $state(false);
	let deleting = $state(false);
	let markingListed = $state(false);

	/** Only the three built verbs open anything; the other two are disabled at
	 *  the control, so this is exhaustive over what can actually be clicked. */
	function start(verb: BulkVerb) {
		if (verb === 'cross_list') {
			crossListing = true;
		} else if (verb === 'mark_listed') {
			markingListed = true;
		} else if (verb === 'delete') {
			deleting = true;
		}
	}

	async function markedListed(bound: number) {
		markingListed = false;
		selected = new Set();
		toast('info', `${bound} ${bound === 1 ? 'listing' : 'listings'} attached.`);
		await queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
	}

	async function deleted(count: number) {
		deleting = false;
		selected = new Set();
		toast('info', `${count} ${count === 1 ? 'item' : 'items'} deleted.`);
		await Promise.all([
			queryClient.invalidateQueries({ queryKey: queryKeys.products }),
			queryClient.invalidateQueries({ queryKey: queryKeys.mappings })
		]);
	}

	const products = $derived(catalogue.data ?? []);
	const query = $derived(page.url.searchParams.get('q')?.trim() ?? '');
	const filters = $derived<Filters>({ query, marketplace, standing });

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

	const allRows = $derived(
		products.map((product) =>
			rowFor({
				product,
				mappings: byProduct.get(product.id) ?? [],
				work: work.data ?? new Map(),
				connections: connections.data ?? [],
				statuses: halts.data ?? []
			})
		)
	);

	const rows = $derived(allRows.filter((row) => matchesFilters(row, filters)));
	const tally = $derived(inventoryTally(allRows, rows));

	const shown = $derived.by(() => {
		const now = Date.now();
		return rows.map((row) => {
			const metrics = figures.get(row.product.id);
			return {
				row,
				price: formatPrice(row.product.price),
				views: formatMetric(metrics?.views ?? undefined),
				sales: formatMetric(metrics?.sales ?? undefined),
				updated: agoLabel(row.product.updated_at, now)
			};
		});
	});

	const rowHeight = $derived(onPhone ? PHONE_ROW_HEIGHT : ROW_HEIGHT);
	const win = $derived(visibleWindow(shown.length, rowHeight, scrollTop, viewport));

	const chosen = $derived(allRows.filter((row) => selected.has(row.product.id)));
	const allShownSelected = $derived(
		shown.length > 0 && shown.every((entry) => selected.has(entry.row.product.id))
	);

	// Every state a chip can hold, in the order a seller reads them: what needs
	// them first, then what is under way, then what is settled.
	const STANDINGS: readonly { value: StandingFilter; label: string }[] = [
		{ value: 'all', label: 'Any standing' },
		{ value: 'attention', label: 'Needs attention' },
		...(
			[
				'needs_signin',
				'stranded',
				'failed',
				'blocked',
				'in_flight',
				'draft',
				'listed',
				'not_listed'
			] as MarketplaceState[]
		).map((value) => ({ value: value as StandingFilter, label: STATE_LABEL[value] }))
	];

	function toggle(product: string) {
		const next = new Set(selected);
		if (next.has(product)) {
			next.delete(product);
		} else {
			next.add(product);
		}
		selected = next;
	}

	function toggleAll() {
		selected = allShownSelected
			? new Set()
			: new Set(shown.map((entry) => entry.row.product.id));
	}

	function started(runs: { inventory: InventoryId; job: string }[]) {
		crossListing = false;
		selected = new Set();
		void queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
		void queryClient.invalidateQueries({ queryKey: queryKeys.inventoryWork });
		if (runs.length === 0) {
			return;
		}
		if (runs.length === 1) {
			toast('info', `Send started on ${platformTitle(runs[0].inventory)}.`);
			void goto(`/sync/${runs[0].job}`);
			return;
		}
		toast('info', `Send started on ${runs.length} marketplaces.`);
	}

	/** The first thing on this row a person has to act on, or null. The row
	 *  states one rather than every one: the item screen holds the rest, and a
	 *  table row that lists four problems is read as none. */
	function firstProblem(row: InventoryRow) {
		return row.attention[0] ?? null;
	}
</script>

<div class="page">
	<PageHead
		icon="▤"
		title="Inventory"
		description="Every item once, and what each marketplace is doing with it."
	>
		{#snippet aside()}
			<span class="tag-note">
				{#if rows.length === tally.total}
					{tally.total} in the catalogue
				{:else}
					{rows.length} of {tally.total}
				{/if}
			</span>
			<span class="row-actions bulk-only">
				{#each BULK_ACTIONS as action (action.verb)}
					<button
						class="btn {action.verb === 'delete' ? 'danger' : ''}"
						type="button"
						disabled={action.missing !== null || selected.size === 0}
						title={action.missing ?? undefined}
						onclick={() => start(action.verb)}
					>
						{action.label}{selected.size === 0 || action.missing !== null
							? ''
							: ` ${selected.size}`}…
					</button>
				{/each}
			</span>
			<a class="cta" href="/inventory/new">New item</a>
		{/snippet}
	</PageHead>

	{#if tally.attention > 0 && standing !== 'attention'}
		<div class="attn warn">
			<div class="t">
				{tally.attention}
				{tally.attention === 1 ? 'item needs' : 'items need'} you
			</div>
			<p>
				A marketplace is waiting on a sign-in, holding a send, or reporting a failure. Work for
				Tes and TPT runs on your own device, so nothing moves while that device is off.
			</p>
			<button class="act" type="button" onclick={() => (standing = 'attention')}>
				Show only those
			</button>
		</div>
	{/if}

	<Panel>
		<div class="filter-bar">
			<select
				aria-label="Filter by marketplace"
				value={marketplace}
				onchange={(event) =>
					(marketplace = event.currentTarget.value as InventoryId | 'all')}
			>
				<option value="all">Any marketplace</option>
				{#each AUTHORABLE_PLATFORMS as inventory (inventory)}
					<option value={inventory}>{platformTitle(inventory)}</option>
				{/each}
			</select>
			<select
				aria-label="Filter by standing"
				value={standing}
				onchange={(event) => (standing = event.currentTarget.value as StandingFilter)}
			>
				{#each STANDINGS as option (option.value)}
					<option value={option.value}>{option.label}</option>
				{/each}
			</select>
			{#if marketplace !== 'all' || standing !== 'all'}
				<button
					class="btn small"
					type="button"
					onclick={() => {
						marketplace = 'all';
						standing = 'all';
					}}
				>
					Clear filters
				</button>
			{/if}
			<span class="grow"></span>
			<span class="tag-note">
				{tally.listedOn}
				{tally.listedOn === 1 ? 'marketplace' : 'marketplaces'} carrying a live listing
			</span>
		</div>

		{#if catalogue.isPending || mappings.isPending}
			<p class="quiet">Loading the catalogue…</p>
		{:else if catalogue.isError || mappings.isError}
			<p class="quiet">The catalogue could not be read.</p>
		{:else if products.length === 0}
			<div class="placeholder">
				<span class="big" aria-hidden="true">▤</span>
				<b>No items yet</b>
				<p>
					Author one here, or let an import bring your existing listings in. Once an item is in your
					catalogue it appears here, with what every marketplace is doing with it beside it.
				</p>
				<div class="actions" style="justify-content: center; margin-top: 14px">
					<a class="cta" href="/inventory/new">New item</a>
				</div>
			</div>
		{:else if rows.length === 0}
			<div class="placeholder">
				<span class="big" aria-hidden="true">⌕</span>
				<b>Nothing matches</b>
				<p>
					{#if query}
						Search reads item titles. Clear the box, or widen the filters, to see the whole
						catalogue again.
					{:else}
						No item is in that state on that marketplace. Clear the filters to see the whole
						catalogue again.
					{/if}
				</p>
			</div>
		{:else}
			<div
				class="tbl-wrap scroll-tbl"
				onscroll={(event) => {
					scrollTop = event.currentTarget.scrollTop;
					viewport = event.currentTarget.clientHeight;
				}}
			>
				<table class="card-tbl">
					<thead>
						<tr>
							<th>
								<input
									type="checkbox"
									aria-label="Select every item shown"
									checked={allShownSelected}
									onchange={toggleAll}
								/>
							</th>
							<th>Item</th>
							<th>Marketplaces</th>
							<th>Needs you</th>
							<th class="num">Price</th>
							<th class="num">Views</th>
							<th class="num">Sales</th>
							<th class="num">Updated</th>
						</tr>
					</thead>
					<tbody>
						{#if win.padTop > 0}
							<tr class="spacer" style="height: {win.padTop}px"><td colspan={COLUMNS}></td></tr>
						{/if}
						{#each shown.slice(win.start, win.end) as entry (entry.row.product.id)}
							{@const problem = firstProblem(entry.row)}
							<tr style="height: {rowHeight}px">
								<td>
									<input
										type="checkbox"
										aria-label={`Select ${entry.row.product.title}`}
										checked={selected.has(entry.row.product.id)}
										onchange={() => toggle(entry.row.product.id)}
									/>
								</td>
								<td class="title-cell">
									<a
										class="t"
										href={`/inventory/${entry.row.product.id}`}
										title={entry.row.product.title}
									>
										{entry.row.product.title}
									</a>
									<span class="card-price">{entry.price}</span>
								</td>
								<td><MarketplaceChips chips={entry.row.chips} /></td>
								<td>
									{#if problem === null}
										<span class="s">—</span>
									{:else}
										<a class="link" href={problem.action?.href ?? `/inventory/${entry.row.product.id}`}>
											{problem.detail}
										</a>
									{/if}
								</td>
								<td class="num">{entry.price}</td>
								<td class="num">{entry.views}</td>
								<td class="num">{entry.sales}</td>
								<td class="num">{entry.updated}</td>
							</tr>
						{/each}
						{#if win.padBottom > 0}
							<tr class="spacer" style="height: {win.padBottom}px"><td colspan={COLUMNS}></td></tr>
						{/if}
					</tbody>
				</table>
			</div>
			<p class="foot-note">
				{#each unavailable() as action (action.verb)}
					<span class="block">{action.label} in bulk is not built. {action.missing}</span>
				{/each}
			</p>
			<p class="foot-note">
				Views and sales come from the analytics capture and carry its age, not a live check.
				{#if captured.isError}
					The capture could not be read just now, so both columns are blank rather than zero.
				{/if}
				{#if work.isError}
					What each marketplace is doing right now could not be read, so the chips show the last
					state recorded rather than a live one.
				{/if}
			</p>
		{/if}
	</Panel>
</div>

<CrossListDialog
	open={crossListing}
	rows={chosen}
	onClose={() => (crossListing = false)}
	onStarted={started}
/>

<MarkListedDialog
	open={markingListed}
	rows={chosen}
	onClose={() => (markingListed = false)}
	onBound={markedListed}
/>

<BulkDeleteDialog
	open={deleting}
	rows={chosen}
	onClose={() => (deleting = false)}
	onDeleted={deleted}
/>
