<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { METRIC_COLUMNS, capturedAgo, formatMetric, titlesByMapping } from '$lib/analytics-view';
	import { PORTFOLIO_ROWS, tesPortfolio, type PortfolioRow } from '$lib/tes-portfolio';
	import { allPages, api } from '$lib/api';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { queryKeys } from '$lib/query';

	const summary = createQuery(() => ({
		queryKey: queryKeys.analytics,
		queryFn: () => api.analytics()
	}));

	// The titles are read separately on purpose: the figures render as soon as
	// the summary lands, so a catalogue that is slow or unreadable costs a row
	// its title rather than costing the table its numbers. The Tes panel below
	// is counted from these same two reads and shares their cache entries.
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

	// Both reads are needed before a figure is honest: a count taken while one
	// of them is still in flight would read as a real zero.
	const counted = $derived(catalogue.isSuccess && mappings.isSuccess);
	const uncountable = $derived(catalogue.isError || mappings.isError);
	const portfolio = $derived(tesPortfolio(catalogue.data ?? [], mappings.data ?? []));

	// Each breakdown row belongs under the figure it breaks down, so the panel
	// cannot read two sub-counts as peers of the total they come out of.
	const groups: { row: PortfolioRow; under: PortfolioRow[] }[] = [];
	for (const row of PORTFOLIO_ROWS) {
		const parent = groups.at(-1);
		if (row.breakdown && parent !== undefined) {
			parent.under.push(row);
		} else {
			groups.push({ row, under: [] });
		}
	}
</script>

<div class="page">
	<PageHead
		icon="◔"
		title="Analytics"
		description="Captured from Teachers Pay Teachers · figures show their age, never pretend to be live."
	/>

	<Panel
		title="Per-listing performance"
		description="Each row states the age of its own oldest figure."
	>
		{#if summary.isPending}
			<p class="quiet">Loading…</p>
		{:else if summary.isError}
			<p class="quiet">The analytics could not be read.</p>
		{:else if rows.length === 0}
			<div class="placeholder">
				<span class="big" aria-hidden="true">◔</span>
				<b>No analytics have been captured yet.</b>
				<p>
					A capture runs once a day for each organisation it is enabled for, and the first
					figures appear here after the first pass. Nothing is missing until then; there is
					simply nothing recorded to show.
				</p>
			</div>
		{:else}
			<div class="tbl-wrap">
				<table>
					<thead>
						<tr>
							<th>Listing</th>
							<th>Inventory</th>
							{#each METRIC_COLUMNS as column (column.key)}
								<th class="num" title={column.key}>{column.heading}</th>
							{/each}
							<th class="num">Captured</th>
						</tr>
					</thead>
					<tbody>
						{#each rows as row (row.listing.mapping)}
							<tr>
								<td class="title-cell">
									{#if row.title === undefined}
										<span
											class="mono"
											title="This mapping's product is not in the catalogue read, so only its identifier is known here."
										>
											{row.listing.mapping}
										</span>
									{:else}
										<div class="t">{row.title}</div>
									{/if}
								</td>
								<td><span class="badge">{row.listing.inventory}</span></td>
								{#each METRIC_COLUMNS as column (column.key)}
									<td class="num">{formatMetric(row.listing.metrics[column.key])}</td>
								{/each}
								<td class="num" title={row.captured}>{row.age}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
			<p class="foot-note">
				Every figure is a captured total, not a live one. A row states the age of its oldest
				figure, so nothing above reads fresher than it is.
			</p>
		{/if}
	</Panel>

	<Panel
		title="Your Tes portfolio"
		description="Tes publishes no statistics of its own, so this is counted from your own catalogue instead."
	>
		{#snippet more()}
			{#if counted}
				<span class="s">{portfolio.listings} tracked for Tes</span>
			{/if}
		{/snippet}
		{#if uncountable}
			<p class="quiet">Your catalogue could not be read.</p>
		{:else if !counted}
			<p class="quiet">Counting…</p>
		{:else}
			<div class="tes-band">
				{#each groups as group (group.row.key)}
					<div class="mini" title={group.row.explanation}>
						<div class="n">{portfolio[group.row.key]}</div>
						<div class="l">{group.row.label}</div>
						{#each group.under as sub (sub.key)}
							<div class="l" title={sub.explanation}>
								{portfolio[sub.key]}
								{sub.label}
							</div>
						{/each}
					</div>
				{/each}
			</div>
			<p class="foot-note">
				Each listing reads as it was last recorded here, not as a live check of Tes. The price
				is your catalogue's own: a mapping that converts or overrides it can put a different
				figure on the listing itself.
			</p>
		{/if}
	</Panel>
</div>
