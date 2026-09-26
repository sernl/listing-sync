<script lang="ts">
	// Analytics: one row of figures across every shop, then a panel per shop
	// side by side — each headed by the shop's arrow to what it reports — and
	// the ranked table under them. How the figures are read and how old they
	// are sits behind the header's Explain; the page itself says the numbers.

	import { createQuery } from '@tanstack/svelte-query';
	import './analytics.css';
	import '$lib/flow.css';
	import { formatMetric, METRIC_COLUMNS, titlesByMapping } from '$lib/analytics-view';
	import { allPages, api } from '$lib/api';
	import { agoLabel } from '$lib/elapsed';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import FlowDiagram from '$lib/FlowDiagram.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import StatCard from '$lib/StatCard.svelte';
	import { PORTFOLIO_ROWS, tesPortfolio } from '$lib/tes-portfolio';
	import { ticker } from './clock.svelte';
	import MetricBars from './MetricBars.svelte';
	import {
		chartView,
		combineReads,
		figures,
		groupPortfolioRows,
		headerMeta,
		listingsForMappings,
		mappingsForProducts,
		plainWord,
		readState,
		resourceRows,
		SCOPES,
		scopeCounts,
		scopeReports,
		silentIn,
		standings,
		tileViews,
		type ScopeId
	} from './model';

	const clock = ticker();



	let label = $state<string | null>(null);

	const summary = createQuery(() => ({
		queryKey: queryKeys.analytics,
		queryFn: () => api.analytics()
	}));

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
		readState(catalogue.isSuccess && mappings.isSuccess, catalogue.isError || mappings.isError)
	);

	// A label narrows the figures through the catalogue, so while one is set
	// the figures are only as read as the catalogue is.
	const narrowing = $derived(label !== null);
	const figuresRead = $derived(narrowing ? combineReads(summaryRead, catalogueRead) : summaryRead);

	const bindings = $derived(narrowing ? mappingsForProducts(allMappings, products) : allMappings);
	const shown = $derived(narrowing ? listingsForMappings(listings, bindings) : listings);
	const titles = $derived(titlesByMapping(products, bindings));
	const counts = $derived(scopeCounts(bindings));

	/** How many bars a shop's chart draws: what fits a half-width panel. */
	const TOP = 8;

	const meta = $derived(headerMeta({ scope: 'all', listings: shown, summary: figuresRead, now: clock.now }));

	const tiles = $derived(
		tileViews({
			figures: figures(shown, 'all'),
			scope: 'all',
			summary: figuresRead,
			catalogue: catalogueRead,
			standing: standings(bindings, 'all'),
			now: clock.now,
			format: (value) => formatMetric(value)
		})
	);

	/** One panel per shop, the ones that report figures first. */
	const shops = $derived(
		SCOPES.filter((entry) => entry.mark !== null)
			.sort((a, b) => Number(scopeReports(b.id)) - Number(scopeReports(a.id)))
			.map((entry) => {
				const scope: ScopeId = entry.id;
				const standing = standings(bindings, scope);
				const reports = scopeReports(scope);
				return {
					scope,
					inventory: entry.mark!,
					reports,
					tracked: counts[scope],
					standing,
					figures: figures(shown, scope),
					chart: chartView({
						scope,
						rows: resourceRows(shown, titles, scope),
						standing,
						limit: TOP,
						summary: figuresRead,
						catalogue: catalogueRead
					})
				};
			})
	);

	const groups = $derived(groupPortfolioRows(PORTFOLIO_ROWS));
	const portfolio = $derived(tesPortfolio(products, bindings));

	const tableRows = $derived(
		resourceRows(shown, titles, 'all').map((row) => ({
			...row,
			age: agoLabel(row.observedAt, clock.now),
			captured: new Date(row.observedAt).toLocaleString()
		}))
	);
	const reporting = $derived(SCOPES.filter((entry) => entry.mark !== null && scopeReports(entry.id)));
	const silent = $derived(silentIn('all').join(' and '));
</script>

<div class="page flow-page an-page">
	<PageHead
		icon="chart-line"
		title="Analytics"
		description="What your shops report about your resources."
		guide="analytics"
	>
		{#snippet aside()}
			<div class="an-meta">
				<div class="an-meta-k">{meta.key}</div>
				<div class="an-meta-v" title={meta.at === null ? undefined : new Date(meta.at).toLocaleString()}>
					{meta.value}
				</div>
			</div>
			<Explain title="How these figures are read" label="How it works">
				<p>
					The Teachouse app on your computer reads each shop’s own figures when it is online.
					Nothing here is live: each figure is the latest the app saw.
				</p>
				<p>
					There is no date range. Sold, Views and Earned are totals as the shop reports them.
					Earned is the number the shop gave, in its own currency.
				</p>
				<p>
					“Figures as of” is the oldest reading on the page, so nothing looks fresher than it
					is. A figure drawn from only some of your listings says how many.
				</p>
				<p>
					Only {reporting.map((entry) => entry.label).join(' and ')} reports figures today. {silent}
					doesn’t share any, so its panel counts your own resources instead: what is live, in
					draft, or not sent yet.
				</p>
			</Explain>
		{/snippet}
	</PageHead>

	<div class="flow">
		<div class="an-filters">
			<Field label="Label" id="an-label">
				<select id="an-label" bind:value={label} disabled={labels.isError}>
					<option value={null}>Every resource</option>
					{#each labels.data ?? [] as one (one.name)}
						<option value={one.name}>{one.name}</option>
					{/each}
				</select>
			</Field>
			{#if labels.isError}
				<p class="an-why">We could not load your labels, so you can’t filter right now.</p>
			{/if}
		</div>

		<div class="cards an-stats">
			{#each tiles as tile (tile.key)}
				<StatCard icon={tile.icon} label={tile.label} sub={tile.sub} tag={tile.tag}>
					<span
						class:an-none={!tile.figure}
						class:price={tile.figure && tile.key === 'earnings'}
						class:an-counted-n={tile.counted}>{tile.value}</span
					>
				</StatCard>
			{/each}
		</div>

		<div class="flow-cols an-shops">
			{#each shops as shop (shop.scope)}
				<section class="flow-card an-shop" aria-label="{shop.inventory} figures">
					<FlowDiagram
						from={{ inventory: shop.inventory }}
						to={[
							shop.reports
								? { icon: 'chart-line', label: 'Figures' }
								: { icon: 'layout-list', label: 'Your count' }
						]}
						rule={shop.reports ? 'reports' : 'shares nothing'}
						label={shop.reports
							? `${shop.inventory} reports sold, views and earned`
							: `${shop.inventory} shares no figures; these are counted from your resources`}
					/>

					{#if shop.reports}
						<dl class="an-figs">
							{#each shop.figures as figure (figure.key)}
								<div>
									<dt>{plainWord(figure.key, figure.heading)}</dt>
									<dd class:price={figure.key === 'earnings' && figure.total !== undefined}>
										{figuresRead === 'read' ? formatMetric(figure.total) : '—'}
									</dd>
								</div>
							{/each}
						</dl>

						<h3 class="flow-label">{shop.chart.title}</h3>
						{#if figuresRead === 'failed'}
							<p class="an-quiet">We could not load your figures. Reload the page to try again.</p>
						{:else if figuresRead === 'pending'}
							<p class="an-quiet">Loading…</p>
						{:else if shop.chart.bars.length === 0}
							<p class="an-quiet">No figures yet. They show up when the app is next online.</p>
						{:else}
							<MetricBars
								bars={shop.chart.bars}
								counted={shop.chart.counted}
								label={shop.chart.label}
								format={(value) => formatMetric(value)}
							/>
						{/if}
					{/if}

					<h3 class="flow-label">
						Where they stand
						{#if catalogueRead === 'read'}<span class="an-chip">{shop.tracked} tracked</span>{/if}
					</h3>
					{#if catalogueRead === 'failed'}
						<p class="an-quiet">We could not load your resources. Reload the page to try again.</p>
					{:else if catalogueRead === 'pending'}
						<p class="an-quiet">Counting…</p>
					{:else if shop.scope === 'tes'}
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
					{:else}
						<div class="an-standings">
							<div class="an-standing" title="Last seen live on the marketplace.">
								<div class="an-n">{shop.standing.live}</div>
								<div class="an-l">Live</div>
							</div>
							<div class="an-standing" title="Created on the marketplace and not published yet.">
								<div class="an-n">{shop.standing.drafts}</div>
								<div class="an-l">Drafts</div>
							</div>
							<div class="an-standing" title="Set up for a marketplace, with nothing created there yet.">
								<div class="an-n">{shop.standing.unsent}</div>
								<div class="an-l">Not sent yet</div>
							</div>
							<div
								class="an-standing"
								title="Sent for review, in review, rejected, withdrawn, being created, or removed."
							>
								<div class="an-n">{shop.standing.other}</div>
								<div class="an-l">Other</div>
							</div>
						</div>
					{/if}
				</section>
			{/each}
		</div>

		<section class="flow-section" aria-labelledby="an-top-title">
			<div class="flow-section-head">
				<h2 id="an-top-title">Top resources</h2>
			</div>
			{#if figuresRead === 'failed'}
				<p class="an-quiet">We could not load your figures. Reload the page to try again.</p>
			{:else if figuresRead === 'pending'}
				<p class="an-quiet">Loading…</p>
			{:else if tableRows.length === 0}
				<Placeholder
					icon="chart-line"
					headline="No figures yet"
					body="They show up the next time the Teachouse app on your computer is online."
				/>
			{:else}
				<div class="flow-table-wrap">
					<table class="flow-table an-table">
						<thead>
							<tr>
								<th>Resource</th>
								<th>Shop</th>
								{#each METRIC_COLUMNS as column (column.key)}
									<th class="an-num">{plainWord(column.key, column.heading)}</th>
								{/each}
								<th class="an-num">Read</th>
							</tr>
						</thead>
						<tbody>
							{#each tableRows as row (row.mapping)}
								<tr>
									<td>
										{#if row.title === undefined}
											<span class="an-id" title="We could not find the resource for this listing, so we show its reference instead.">
												{row.mapping}
											</span>
										{:else}
											<span class="res-name an-t" title={row.title}>{row.title}</span>
										{/if}
									</td>
									<td class="marks"><MarketplaceMark inventory={row.inventory} size={18} /></td>
									{#each METRIC_COLUMNS as column (column.key)}
										<td class="an-num" class:price={column.key === 'earnings' && row.metrics[column.key] !== undefined}>
											{formatMetric(row.metrics[column.key])}
										</td>
									{/each}
									<td class="an-num an-age" title={row.captured}>{row.age}</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</section>
	</div>
</div>
