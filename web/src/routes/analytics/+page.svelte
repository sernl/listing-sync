<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { METRIC_COLUMNS, capturedAgo, formatMetric, titlesByMapping } from '$lib/analytics-view';
	import { allPages, api } from '$lib/api';
	import { queryKeys } from '$lib/query';

	const summary = createQuery(() => ({
		queryKey: queryKeys.analytics,
		queryFn: () => api.analytics()
	}));

	// The titles are read separately on purpose: the figures render as soon as
	// the summary lands, so a catalogue that is slow or unreadable costs a row
	// its title rather than costing the table its numbers.
	const catalogue = createQuery(() => ({
		queryKey: queryKeys.products,
		queryFn: () => allPages(api.products, (page) => page.products)
	}));
	const mappings = createQuery(() => ({
		queryKey: queryKeys.mappings,
		queryFn: () => api.mappings().then((view) => view.mappings)
	}));

	const titles = $derived(titlesByMapping(catalogue.data ?? [], mappings.data ?? []));

	const rows = $derived.by(() => {
		const now = Date.now();
		return (summary.data?.listings ?? []).map((listing) => ({
			listing,
			title: titles.get(listing.mapping),
			age: capturedAgo(listing.observed_at, now),
			captured: new Date(listing.observed_at).toLocaleString()
		}));
	});
</script>

<h1 class="mb-1 text-xl font-semibold">Analytics</h1>
<p class="mb-4 text-sm text-slate-500">
	Tes listings never appear here: Tes publishes no statistics of its own, so there
	is nothing for a capture to read. These figures come from Teachers Pay Teachers.
</p>

{#if summary.isPending}
	<p class="text-slate-500">Loading…</p>
{:else if summary.isError}
	<p class="text-slate-500">The analytics could not be read.</p>
{:else if rows.length === 0}
	<div class="rounded border border-slate-200 bg-white p-4">
		<p class="font-medium">No analytics have been captured yet.</p>
		<p class="mt-1 text-sm text-slate-500">
			A capture runs once a day for each organisation it is enabled for, and the
			first figures appear here after the first pass. Nothing is missing until
			then; there is simply nothing recorded to show.
		</p>
	</div>
{:else}
	<div class="overflow-x-auto rounded border border-slate-200 bg-white">
		<table class="w-full text-sm">
			<thead class="bg-slate-100 text-left">
				<tr>
					<th class="px-3 py-2">Listing</th>
					<th class="px-3 py-2">Inventory</th>
					{#each METRIC_COLUMNS as column (column.key)}
						<th class="px-3 py-2 text-right" title={column.key}>{column.heading}</th>
					{/each}
					<th class="px-3 py-2">Captured</th>
				</tr>
			</thead>
			<tbody>
				{#each rows as row (row.listing.mapping)}
					<tr class="border-t border-slate-100">
						<td class="px-3 py-2">
							{#if row.title === undefined}
								<span
									class="font-mono text-xs text-slate-500"
									title="This mapping's product is not in the catalogue read, so only its identifier is known here."
								>
									{row.listing.mapping}
								</span>
							{:else}
								<span class="font-medium">{row.title}</span>
							{/if}
						</td>
						<td class="px-3 py-2">
							<span class="rounded bg-slate-100 px-2 py-0.5 text-xs">
								{row.listing.inventory}
							</span>
						</td>
						{#each METRIC_COLUMNS as column (column.key)}
							<td class="px-3 py-2 text-right tabular-nums">
								{formatMetric(row.listing.metrics[column.key])}
							</td>
						{/each}
						<td class="px-3 py-2 text-slate-500" title={row.captured}>{row.age}</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
	<p class="mt-3 text-xs text-slate-500">
		Every figure is a captured total, not a live one. A row states the age of its
		oldest figure, so nothing above reads fresher than it is.
	</p>
{/if}
