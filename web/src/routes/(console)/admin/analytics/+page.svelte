<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import Explain from '$lib/Explain.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatCard from '$lib/StatCard.svelte';
	import MetricBars from '$lib/pages/analytics/MetricBars.svelte';
	import SiteLine from '$lib/pages/admin/SiteLine.svelte';
	import {
		RANGES,
		bars,
		cityLabel,
		conversionRate,
		failureOf,
		formatCount,
		formatPercent,
		pageLabel,
		parseRange,
		referrerLabel,
		rowLabel,
		updatedLabel,
		windowLabel,
		type SiteRange,
		type SiteRow
	} from '$lib/pages/admin/site-analytics';
	import '$lib/flow.css';
	import '$lib/pages/admin/admin.css';
	import '$lib/pages/analytics/analytics.css';
	import '$lib/pages/admin/site-analytics.css';
	import '$lib/styles/data.css';

	const range = $derived(parseRange(page.url.searchParams.get('range')));

	const site = createQuery(() => ({
		queryKey: queryKeys.adminSiteAnalytics(range),
		queryFn: () => api.adminSiteAnalytics(range),
		// The server keeps each range five minutes; asking sooner only reads
		// its memory, so the page does not.
		staleTime: 5 * 60 * 1000,
		retry: false,
		// The last range stays on screen while the next one loads, so the
		// picker does not blank the page.
		placeholderData: (previous) => previous
	}));

	const failure = $derived(site.isError ? failureOf(site.error) : null);
	const view = $derived(site.data);
	const rate = $derived(view === undefined ? null : conversionRate(view.totals));

	function pick(next: SiteRange) {
		const search = next === RANGES[0].id ? '' : `?range=${next}`;
		void goto(`${page.url.pathname}${search}`, {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	}
</script>

{#snippet table(
	id: string,
	heading: string,
	column: string,
	rows: readonly SiteRow[],
	label: (row: SiteRow) => string,
	empty: string
)}
	<section class="flow-section" aria-labelledby={id}>
		<div class="flow-section-head"><h2 {id}>{heading}</h2></div>
		{#if rows.length === 0}
			<p class="data-empty">{empty}</p>
		{:else}
			<div class="data-table-wrap">
				<table class="data-table stack">
					<thead>
						<tr>
							<th scope="col">{column}</th>
							<th scope="col" class="num">Visitors</th>
							<th scope="col" class="num">Page views</th>
						</tr>
					</thead>
					<tbody>
						{#each rows as row, index (index)}
							<tr>
								<td data-label={column} class="sa-name">{label(row)}</td>
								<td data-label="Visitors" class="num">{formatCount(row.visitors)}</td>
								<td data-label="Page views" class="num">{formatCount(row.pageviews)}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
	</section>
{/snippet}

<div class="page flow-page sa-page">
	<PageHead
		icon="activity"
		title="Site analytics"
		description="Who visited the public site, where they came from, and how many signed up."
	>
		{#snippet aside()}
			<div class="segmented" role="radiogroup" aria-label="Range">
				{#each RANGES as choice (choice.id)}
					<button
						type="button"
						role="radio"
						aria-checked={range === choice.id}
						onclick={() => pick(choice.id)}>{choice.label}</button
					>
				{/each}
			</div>
		{/snippet}
	</PageHead>

	{#if failure === 'unconfigured'}
		<Placeholder
			icon="activity"
			headline="Site analytics is not configured"
			body="This server has no PostHog key, so it cannot read the site's visitors. Nothing is wrong with the site."
		/>
	{:else if failure !== null}
		<Placeholder
			icon="circle-alert"
			headline="We could not load the site's figures"
			body={site.error instanceof Error && failure === 'upstream'
				? site.error.message
				: 'Try reloading the page.'}
		/>
	{:else if view === undefined}
		<p class="quiet">Loading the site's figures…</p>
	{:else}
		<div class="flow">
			<div class="sa-meta">
				<span>{windowLabel(view)}, New Zealand time</span>
				<span aria-hidden="true">·</span>
				<span>{updatedLabel(view.fetched_at)}</span>
				<Explain title="Where these figures come from" label="How it works">
					<p>
						PostHog counts every page view on {view.site_host ?? 'the site'}. A visitor is one
						browser, so the same person on a phone and a laptop counts twice.
					</p>
					<p>
						Signups are new accounts made in the range, wherever the person started. Conversion is
						signups divided by visitors.
					</p>
					<p>Figures refresh every five minutes. Visitors who block analytics are not counted.</p>
				</Explain>
			</div>

			<div class="op-stats">
				<StatCard
					icon="users"
					label="Visitors"
					sub={`${formatCount(view.totals.cta_clicks)} Start free ${view.totals.cta_clicks === 1 ? 'click' : 'clicks'}`}
				>
					{formatCount(view.totals.visitors)}
				</StatCard>
				<StatCard icon="eye" label="Page views">
					{formatCount(view.totals.pageviews)}
				</StatCard>
				<StatCard icon="circle-user" tone="ok" label="Signups">
					{formatCount(view.totals.signups)}
				</StatCard>
				<StatCard icon="chart-line" label="Conversion" sub="Signups per visitor">
					{formatPercent(rate)}
				</StatCard>
			</div>

			<section class="flow-section" aria-labelledby="sa-days">
				<div class="flow-section-head"><h2 id="sa-days">Visitors each day</h2></div>
				<div class="flow-card">
					{#if view.days.every((day) => day.visitors === 0)}
						<p class="data-empty">No visitors in this range yet.</p>
					{:else}
						<SiteLine
							days={view.days}
							label={`Visitors each day, ${windowLabel(view)}: ${formatCount(view.totals.visitors)} in all.`}
						/>
					{/if}
				</div>
			</section>

			{@render table(
				'sa-pages',
				'Top pages',
				'Page',
				view.pages,
				(row) => pageLabel(row.label),
				'No page views in this range.'
			)}

			<div class="flow-cols">
				{@render table(
					'sa-referrers',
					'Where visitors came from',
					'Site',
					view.referrers,
					(row) => referrerLabel(row.label),
					'No visits in this range.'
				)}
				{@render table(
					'sa-utm',
					'Campaigns (UTM source)',
					'Source',
					view.utm_sources,
					(row) => rowLabel(row.label),
					'No visits came from a tagged link.'
				)}
			</div>

			<div class="flow-cols">
				{@render table(
					'sa-countries',
					'Countries',
					'Country',
					view.countries,
					(row) => rowLabel(row.label),
					'No locations in this range.'
				)}
				{@render table(
					'sa-cities',
					'Cities',
					'City',
					view.cities,
					cityLabel,
					'No cities in this range.'
				)}
			</div>

			<section class="flow-section" aria-labelledby="sa-devices">
				<div class="flow-section-head"><h2 id="sa-devices">Devices</h2></div>
				<div class="flow-cols">
					{#each [{ key: 'device', title: 'Device type', rows: view.devices }, { key: 'browser', title: 'Browser', rows: view.browsers }, { key: 'os', title: 'Operating system', rows: view.systems }] as group (group.key)}
						<div class="flow-card">
							<div class="flow-section-head"><h3>{group.title}</h3></div>
							{#if group.rows.length === 0}
								<p class="sa-quiet">Nothing recorded yet.</p>
							{:else}
								<MetricBars
									bars={bars(group.rows, (row) => rowLabel(row.label))}
									label={`${group.title}, visitors in this range`}
									format={formatCount}
								/>
							{/if}
						</div>
					{/each}
				</div>
			</section>
		</div>
	{/if}
</div>
