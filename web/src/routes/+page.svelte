<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { allPages, api } from '$lib/api';
	import {
		ACTIVITY_LIMIT,
		RECENT_LIMIT,
		activityRows,
		attention,
		liveTally,
		recentlyUpdated,
		salesCapture,
		syncTally
	} from '$lib/dashboard';
	import { agoLabel } from '$lib/elapsed';
	import { badgesFor, formatPrice, rowStatus } from '$lib/listings-view';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import StatCard from '$lib/StatCard.svelte';
	import { queryKeys } from '$lib/query';
	import type { MappingHead } from '$lib/api';

	const catalogue = createQuery(() => ({
		queryKey: queryKeys.products,
		queryFn: () => allPages(api.products, (view) => view.products)
	}));
	const mappings = createQuery(() => ({
		queryKey: queryKeys.mappings,
		queryFn: () => api.mappings().then((view) => view.mappings)
	}));
	const captured = createQuery(() => ({
		queryKey: queryKeys.analytics,
		queryFn: () => api.analytics()
	}));
	const connections = createQuery(() => ({
		queryKey: queryKeys.connections,
		queryFn: () => api.connections()
	}));
	const drain = createQuery(() => ({
		queryKey: queryKeys.drainStats,
		queryFn: () => api.drainStats()
	}));
	const halts = createQuery(() => ({
		queryKey: queryKeys.status,
		queryFn: () => api.status()
	}));
	// The jobs list serves heads with no phase and no counts, so the newest few
	// are read in full. Bounded rather than paginated: this panel is the last
	// handful of runs, and the sync page is where every run lives.
	const activity = createQuery(() => ({
		queryKey: queryKeys.activity,
		queryFn: async () => {
			const first = await api.jobs();
			return Promise.all(first.jobs.slice(0, ACTIVITY_LIMIT).map((head) => api.job(head.job)));
		}
	}));

	const tally = $derived(liveTally(mappings.data ?? []));
	const runs = $derived(activity.data ?? []);
	const sync = $derived(syncTally(runs));
	const rows = $derived(activityRows(runs));
	const capture = $derived(salesCapture(captured.data?.listings ?? [], Date.now()));
	const openQuestions = $derived(drain.data?.open ?? 0);

	const attentionItems = $derived(
		attention({
			connections: connections.data?.connections ?? [],
			statuses: halts.data?.inventories ?? [],
			openQuestions,
			tally: sync
		})
	);

	const byProduct = $derived.by(() => {
		const index = new Map<string, MappingHead[]>();
		for (const mapping of mappings.data ?? []) {
			index.set(mapping.product, [...(index.get(mapping.product) ?? []), mapping]);
		}
		return index;
	});

	const recent = $derived.by(() => {
		const now = Date.now();
		return recentlyUpdated(catalogue.data ?? [], RECENT_LIMIT).map((product) => {
			const owned = byProduct.get(product.id) ?? [];
			return {
				product,
				badges: badgesFor(owned),
				status: rowStatus(owned),
				price: formatPrice(product.price),
				updated: agoLabel(product.updated_at, now)
			};
		});
	});

	const summary = $derived(
		mappings.isPending
			? 'Reading your workspace…'
			: mappings.isError
				? 'Your catalogue could not be read just now.'
				: `${tally.live} live ${tally.live === 1 ? 'listing' : 'listings'} across ` +
					`${tally.marketplaces} ${tally.marketplaces === 1 ? 'marketplace' : 'marketplaces'}.`
	);

	function figure(value: number | null): string {
		return value === null ? '—' : String(value);
	}
</script>

<div class="page">
	<PageHead icon="▦" title="Your workspace" description={summary} />

	<div class="cards">
		<StatCard
			icon="▤"
			tone={tally.live > 0 ? 'ok' : ''}
			tag={`${tally.marketplaces} ${tally.marketplaces === 1 ? 'marketplace' : 'marketplaces'}`}
			label="Live listings"
			sub={`${tally.total} ${tally.total === 1 ? 'mapping' : 'mappings'} in all`}
		>
			{tally.live}
		</StatCard>

		<StatCard
			icon="⇄"
			tag={`newest ${sync.runs} ${sync.runs === 1 ? 'run' : 'runs'}`}
			label="Syncs running"
			sub={`${sync.pending} ${sync.pending === 1 ? 'item' : 'items'} queued or in flight`}
		>
			{sync.active}
		</StatCard>

		<StatCard
			icon={sync.failed > 0 ? '✕' : '✓'}
			tone={sync.failed > 0 ? 'bad' : 'ok'}
			tag={sync.failed > 0 ? 'needs you' : 'all clear'}
			label="Failed writes"
			sub={sync.failed > 0
				? `across ${sync.failingRuns} of the ${sync.runs} newest runs`
				: 'nothing failed in the runs read'}
		>
			{sync.failed}
		</StatCard>

		<StatCard
			icon="◔"
			tag="captured"
			label="Sales"
			sub={capture.age ?? 'nothing captured yet'}
		>
			{figure(capture.sales)}
		</StatCard>
	</div>

	<div class="band">
		<Panel
			title="Sync activity"
			description="What the engine did on its schedule — you never babysit it."
		>
			{#snippet more()}
				<a class="more" href="/sync">View all</a>
			{/snippet}
			{#if activity.isPending}
				<p class="quiet">Reading the newest runs…</p>
			{:else if activity.isError}
				<p class="quiet">The runs could not be read.</p>
			{:else if rows.length === 0}
				<div class="clear">
					<span class="big" aria-hidden="true">⇄</span>
					No sync has run yet. Start one from Listings and its progress appears here.
				</div>
			{:else}
				{#each rows as row (row.job)}
					<a class="job" href={`/sync/${row.job}`}>
						<span class="pill {row.tone}">{row.label}</span>
						<span class="what">
							<span class="t" title={row.job}>{row.inventory} · {row.job.slice(0, 8)}…</span>
							<span class="w">{row.detail}</span>
						</span>
						<span class="when">{agoLabel(row.created_at, Date.now())}</span>
					</a>
				{/each}
			{/if}
		</Panel>

		<Panel title="Needs your attention" description="Only what a person has to decide.">
			{#if attentionItems.length === 0}
				<div class="clear">
					<span class="big" aria-hidden="true">✓</span>
					Nothing is waiting on you. Connections are healthy, no write failed in the runs
					read, and the reconciliation queue is drained.
				</div>
			{:else}
				{#each attentionItems as item (item.key)}
					<div class="attn {item.tone === 'warn' ? 'warn' : ''}">
						<div class="t">{item.title}</div>
						<p>{item.body}</p>
						<a class="act" href={item.action.href}>{item.action.label}</a>
					</div>
				{/each}
			{/if}
		</Panel>
	</div>

	<Panel
		title="Recently updated listings"
		description="Your five most recently touched resources."
	>
		{#snippet more()}
			<a class="more" href="/listings">View all</a>
		{/snippet}
		{#if catalogue.isPending}
			<p class="quiet">Reading the catalogue…</p>
		{:else if catalogue.isError}
			<p class="quiet">The catalogue could not be read.</p>
		{:else if recent.length === 0}
			<div class="clear">
				<span class="big" aria-hidden="true">▤</span>
				No products yet. Once the pipeline ingests one it appears here.
			</div>
		{:else}
			<div class="tbl-wrap">
				<table>
					<thead>
						<tr>
							<th>Listing</th>
							<th>Platforms</th>
							<th>Status</th>
							<th class="num">Price</th>
							<th class="num">Updated</th>
						</tr>
					</thead>
					<tbody>
						{#each recent as row (row.product.id)}
							<tr>
								<td class="title-cell">
									<div class="t" title={row.product.title}>{row.product.title}</div>
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
								<td class="num">{row.updated}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
	</Panel>
</div>
