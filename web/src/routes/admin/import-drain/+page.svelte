<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import { emptyState, formatShare, seriesByOrg } from '$lib/drain';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { queryKeys } from '$lib/query';
	import '$lib/pages/admin/admin.css';

	const drain = createQuery(() => ({
		queryKey: queryKeys.adminImportDrain,
		queryFn: () => api.adminImportDrain().then((view) => view.rows)
	}));

	const readout = $derived(seriesByOrg(drain.data ?? []));
</script>

<div class="page">
	<PageHead
		icon="chart-line"
		title="Import drain"
		description="The share of canonical terms each import raised a new question for, per tenant: it should fall as a tenant's crosswalk fills in."
	>
		{#snippet aside()}
			<StatusPill tone="soon" label="every tenant" />
		{/snippet}
	</PageHead>

	{#if drain.isPending}
		<Panel><p class="quiet">Reading the drain measurements…</p></Panel>
	{:else if drain.isError}
		<Panel><p class="quiet">The drain measurements could not be read.</p></Panel>
	{:else if readout.series.length === 0}
		{@const empty = emptyState(readout.unreadable)}
		<Panel>
			<Placeholder icon={empty.icon} headline={empty.headline} body={empty.body} />
		</Panel>
	{:else}
		{#each readout.series as entry (entry.org)}
			<Panel title={entry.org_name}>
				<p class="s">
					{#if entry.gate.fall === null}
						{entry.runs.length} of 10 migrations recorded; the gate compares the first against the
						tenth.
					{:else}
						First {formatShare(entry.gate.first?.share ?? null)} → tenth
						{formatShare(entry.gate.tenth?.share ?? null)}: a fall of {formatShare(
							entry.gate.fall
						)}.
					{/if}
				</p>

				<div class="op-table">
					<table>
						<thead>
							<tr>
								<th class="num">#</th>
								<th>Direction</th>
								<th class="num">Rows</th>
								<th class="num">Terms seen</th>
								<th class="num">Unmapped</th>
								<th class="num">Covered</th>
								<th class="num">New</th>
								<th class="num">Already open</th>
								<th class="num">Share</th>
							</tr>
						</thead>
						<tbody>
							{#each entry.runs as run, index (run.seq)}
								<tr class={index === 0 || index === 9 ? 'op-gate-row' : ''}>
									<td class="num" data-label="#">{index + 1}</td>
									<td data-label="Direction">{run.source} → {run.target}</td>
									<td class="num" data-label="Rows">{run.rows}</td>
									<td class="num" data-label="Terms seen">{run.terms_seen}</td>
									<td
										class="num {run.terms_unmapped > 0 ? 'op-flag' : ''}"
										data-label="Unmapped">{run.terms_unmapped}</td
									>
									<td class="num" data-label="Covered">{run.terms_covered}</td>
									<td class="num" data-label="New">{run.items_new}</td>
									<td class="num" data-label="Already open">{run.items_already_open}</td>
									<td class="num" data-label="Share">{formatShare(run.share)}</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			</Panel>
		{/each}

		<p class="foot-note">
			Unmapped terms never became canonical, so they are outside the share: a share that falls
			while that column rises is an ingest gap rather than a converging crosswalk.
		</p>
		<p class="foot-note">
			The ledger is pruned past the 30-day window, so runs older than it are not shown and
			the first row above may not be that organisation's first migration.
		</p>
	{/if}

	{#if readout.unreadable > 0}
		<p class="foot-note op-unreadable">
			{readout.unreadable === 1
				? '1 row could not be read and is not shown.'
				: `${readout.unreadable} rows could not be read and are not shown.`}
		</p>
	{/if}
</div>
