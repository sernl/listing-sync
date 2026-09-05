<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import './analytics.css';
	import { formatMetric, METRIC_COLUMNS, titlesByMapping } from '$lib/analytics-view';
	import { allPages, api } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import { agoLabel } from '$lib/elapsed';
	import Field from '$lib/Field.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import TabBar from '$lib/TabBar.svelte';
	import { PORTFOLIO_ROWS, tesPortfolio } from '$lib/tes-portfolio';
	import { ticker } from './clock.svelte';
	import FigureTile from './FigureTile.svelte';
	import MetricBars from './MetricBars.svelte';
	import {
		asScope,
		chartView,
		combineReads,
		figures,
		groupPortfolioRows,
		headerMeta,
		listingsForMappings,
		mappingsForProducts,
		readState,
		resourceRows,
		SCOPES,
		scopeCounts,
		scopeReports,
		silentIn,
		standings,
		standingsDescription,
		tableView,
		tileViews
	} from './model';

	const clock = ticker();

	let selected = $state('all');
	let label = $state<string | null>(null);
	const scope = $derived(asScope(selected));

	const summary = createQuery(() => ({
		queryKey: queryKeys.analytics,
		queryFn: () => api.analytics()
	}));

	// The titles are read separately on purpose: the figures render as soon as
	// the summary lands, so a catalogue that is slow or unreadable costs a row
	// its title rather than costing the page its numbers. The counted figures
	// come from these same two reads and share their cache entries.
	//
	// The label narrows this read at the server, which is why it has a cache
	// entry of its own: a filtered catalogue is a different answer, not a view
	// over the whole one.
	const catalogue = createQuery(() => ({
		queryKey: queryKeys.catalogue(label),
		queryFn: () => allPages((cursor) => api.products(cursor, label), (page) => page.products)
	}));
	const mappings = createQuery(() => ({
		queryKey: queryKeys.mappings,
		queryFn: () => api.mappings().then((view) => view.mappings)
	}));
	const labels = createQuery(() => ({
		queryKey: queryKeys.labels,
		queryFn: () => api.labels().then((view) => view.labels)
	}));

	const listings = $derived(summary.data?.listings ?? []);
	const products = $derived(catalogue.data ?? []);
	const allMappings = $derived(mappings.data ?? []);

	const summaryRead = $derived(readState(summary.isSuccess, summary.isError));
	const catalogueRead = $derived(
		readState(
			catalogue.isSuccess && mappings.isSuccess,
			catalogue.isError || mappings.isError
		)
	);

	// A narrowed page depends on the catalogue for its figures as well as for
	// its counts, so its figures are only as sound as the weaker of the two
	// reads. Unnarrowed, the capture read stands alone.
	const narrowing = $derived(label !== null);
	const figuresRead = $derived(
		narrowing ? combineReads(summaryRead, catalogueRead) : summaryRead
	);

	const bindings = $derived(
		narrowing ? mappingsForProducts(allMappings, products) : allMappings
	);
	const shown = $derived(narrowing ? listingsForMappings(listings, bindings) : listings);
	const titles = $derived(titlesByMapping(products, bindings));

	const counts = $derived(scopeCounts(bindings));
	const tabs = $derived(
		SCOPES.map((entry) => ({
			id: entry.id,
			label: entry.label,
			// No number until the read behind it lands, and none at all if it
			// fails: a zero here is a counted figure like any other, and it is
			// the plausible wrong answer rather than an obviously missing one.
			count: catalogueRead === 'read' ? counts[entry.id] : null,
			hint: entry.hint
		}))
	);

	const reports = $derived(scopeReports(scope));
	const silent = $derived(silentIn(scope).join(' and '));
	const standing = $derived(standings(bindings, scope));
	const rows = $derived(resourceRows(shown, titles, scope));

	/** How many bars the chart draws. Eight is what fits the panel at the
	 *  phone width without the drawing becoming a second table. */
	const TOP = 8;
	const chart = $derived(
		chartView({
			scope,
			rows,
			standing,
			limit: TOP,
			summary: figuresRead,
			catalogue: catalogueRead
		})
	);

	const table = $derived(tableView(scope, figuresRead));

	const meta = $derived(
		headerMeta({ scope, listings: shown, summary: figuresRead, now: clock.now })
	);

	const tiles = $derived(
		tileViews({
			figures: figures(shown, scope),
			scope,
			summary: figuresRead,
			catalogue: catalogueRead,
			standing,
			now: clock.now,
			format: (value) => formatMetric(value)
		})
	);

	const groups = $derived(groupPortfolioRows(PORTFOLIO_ROWS));
	const portfolio = $derived(tesPortfolio(products, bindings));

	// The age alone, not `capturedAgo`'s "captured 1 day ago": the column is
	// headed Captured, so the word in every cell only costs the table width it
	// has none of at the tablet size.
	const tableRows = $derived(
		rows.map((row) => ({
			...row,
			age: agoLabel(row.observedAt, clock.now),
			captured: new Date(row.observedAt).toLocaleString()
		}))
	);
</script>

<div class="page">
	<PageHead
		icon="chart-line"
		title="Analytics"
		description="What each marketplace reports about your resources, and when it last said so."
	>
		{#snippet aside()}
			<div class="an-meta">
				<div class="an-meta-k">{meta.key}</div>
				<div
					class="an-meta-v"
					title={meta.at === null ? undefined : new Date(meta.at).toLocaleString()}
				>
					{meta.value}
				</div>
			</div>
		{/snippet}
	</PageHead>

	<TabBar {tabs} bind:current={selected} />

	<div class="an-filters">
		<Field label="Labels" id="an-label">
			<select id="an-label" bind:value={label} disabled={labels.isError}>
				<option value={null}>Every resource</option>
				{#each labels.data ?? [] as one (one.name)}
					<option value={one.name}>{one.name}</option>
				{/each}
			</select>
		</Field>
		<p class="an-why">
			There is no date range here: a capture keeps one figure per listing, its newest, so there
			is no history to narrow. {#if labels.isError}Your labels could not be read, so the filter
				is unavailable.{/if}
		</p>
	</div>

	<Banner tone="info">
		Only TPT reports figures today. TES publishes no statistics of its own, so its panel counts
		your own catalogue instead.
	</Banner>

	<div class="an-tiles">
		{#each tiles as tile (tile.key)}
			<FigureTile
				icon={tile.icon}
				label={tile.label}
				value={tile.value}
				figure={tile.figure}
				tag={tile.tag}
				sub={tile.sub}
				counted={tile.counted}
			/>
		{/each}
	</div>

	<div class="an-band">
		<Panel title={chart.title} description={chart.description}>
			{#if chart.counted}
				{#if catalogueRead === 'failed'}
					<p class="an-quiet">Your catalogue could not be read.</p>
				{:else if catalogueRead === 'pending'}
					<p class="an-quiet">Counting…</p>
				{:else}
					<MetricBars
						bars={chart.bars}
						counted={chart.counted}
						label={chart.label}
						format={(value) => formatMetric(value)}
					/>
				{/if}
			{:else if figuresRead === 'failed'}
				<p class="an-quiet">The analytics could not be read.</p>
			{:else if figuresRead === 'pending'}
				<p class="an-quiet">Loading…</p>
			{:else if chart.bars.length === 0}
				<Placeholder
					icon="chart-line"
					headline="No figures captured yet"
					body="Your device sends them after its next check-in."
				/>
			{:else}
				<MetricBars
					bars={chart.bars}
					counted={chart.counted}
					label={chart.label}
					format={(value) => formatMetric(value)}
				/>
			{/if}
		</Panel>

		{#if scope === 'tes'}
			<Panel
				title="TES (Tes.com) portfolio"
				description={standingsDescription(
					catalogueRead,
					'Tes publishes no statistics of its own, so this is counted from your own catalogue instead.'
				)}
			>
				{#snippet more()}
					{#if catalogueRead === 'read'}
						<span class="an-chip">{portfolio.listings} tracked for Tes</span>
					{/if}
				{/snippet}
				{#if catalogueRead === 'failed'}
					<p class="an-quiet">Your catalogue could not be read.</p>
				{:else if catalogueRead === 'pending'}
					<p class="an-quiet">Counting…</p>
				{:else}
					<div class="an-standings">
						{#each groups as group (group.row.key)}
							<div class="an-standing" title={group.row.explanation}>
								<div class="an-n">{portfolio[group.row.key]}</div>
								<div class="an-l">{group.row.label}</div>
								{#each group.under as sub (sub.key)}
									<div class="an-l an-under" title={sub.explanation}>
										{portfolio[sub.key]}
										{sub.label}
									</div>
								{/each}
							</div>
						{/each}
					</div>
					<p class="an-note">
						Each listing reads as it was last recorded here, not as a live check of Tes. The
						price is your catalogue's own: a mapping that converts or overrides it can put a
						different figure on the listing itself.
					</p>
				{/if}
			</Panel>
		{:else}
			<Panel
				title="Where your listings stand"
				description={standingsDescription(
					catalogueRead,
					'Counted from your own catalogue, not reported by a marketplace.'
				)}
			>
				{#snippet more()}
					{#if catalogueRead === 'read'}
						<span class="an-chip">{standing.listings} tracked</span>
					{/if}
				{/snippet}
				{#if catalogueRead === 'failed'}
					<p class="an-quiet">Your catalogue could not be read.</p>
				{:else if catalogueRead === 'pending'}
					<p class="an-quiet">Counting…</p>
				{:else}
					<div class="an-standings">
						<div
							class="an-standing"
							title="Bound to a listing the marketplace was last recorded as showing."
						>
							<div class="an-n">{standing.live}</div>
							<div class="an-l">Live</div>
						</div>
						<div
							class="an-standing"
							title="Created on the marketplace and not published yet."
						>
							<div class="an-n">{standing.drafts}</div>
							<div class="an-l">Drafts waiting to go live</div>
						</div>
						<div
							class="an-standing"
							title="Mapped to a marketplace, with nothing created there yet."
						>
							<div class="an-n">{standing.unsent}</div>
							<div class="an-l">Not sent yet</div>
						</div>
						<div
							class="an-standing"
							title="Sent for review, in review, rejected, withdrawn, being created, or removed."
						>
							<div class="an-n">{standing.other}</div>
							<div class="an-l">In another state</div>
						</div>
					</div>
					<p class="an-note">
						Each listing reads as it was last recorded here, not as a live check of the
						marketplace.
					</p>
				{/if}
			</Panel>
		{/if}
	</div>

	<Panel title={table.title} description={table.description}>
		{#if !reports}
			<Placeholder
				icon="chart-line"
				headline="{silent} reports no figures"
				body="There is nothing to rank here. What the panels above show for {silent} is counted from your own catalogue instead."
			/>
		{:else if figuresRead === 'failed'}
			<p class="an-quiet">The analytics could not be read.</p>
		{:else if figuresRead === 'pending'}
			<p class="an-quiet">Loading…</p>
		{:else if tableRows.length === 0}
			<Placeholder
				icon="chart-line"
				headline="No figures captured yet"
				body="Your device sends them after its next check-in."
			/>
		{:else}
			<div class="an-table-wrap">
				<table class="an-table">
					<thead>
						<tr>
							<th class="an-what">Resource</th>
							<th>Marketplace</th>
							{#each METRIC_COLUMNS as column (column.key)}
								<th class="an-num">{column.heading}</th>
							{/each}
							<th class="an-num">Captured</th>
						</tr>
					</thead>
					<tbody>
						{#each tableRows as row (row.mapping)}
							<tr>
								<td class="an-what">
									{#if row.title === undefined}
										<span
											class="an-id"
											title={`Mapping ${row.mapping}. Its product is not in the catalogue read, so only its identifier is known here.`}
										>
											{row.mapping}
										</span>
									{:else}
										<div class="an-t" title={row.title}>{row.title}</div>
									{/if}
								</td>
								<td><span class="an-chip">{row.platform}</span></td>
								{#each METRIC_COLUMNS as column (column.key)}
									<td class="an-num">{formatMetric(row.metrics[column.key])}</td>
								{/each}
								<td class="an-num" title={row.captured}>{row.age}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
			<p class="an-note">
				Every figure is a captured total, not a live one. A row states the age of its oldest
				figure, so nothing above reads fresher than it is.
			</p>
		{/if}
	</Panel>
</div>
