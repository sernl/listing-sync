<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import { emptyState, formatPoints, formatShare, seriesByOrg } from '$lib/drain';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { SHORT_NAME } from '$lib/platforms';
	import StatusPill from '$lib/StatusPill.svelte';
	import { queryKeys } from '$lib/query';
	import { deadLetterHeadline, topicLabel } from '$lib/pages/admin/dead-letters';
	import '$lib/pages/admin/admin.css';

	// The whole view rather than its rows: `truncated` is the half of the
	// answer that says whether the rows are the whole platform.
	const drain = createQuery(() => ({
		queryKey: queryKeys.adminImportDrain,
		queryFn: () => api.adminImportDrain()
	}));

	const readout = $derived(seriesByOrg(drain.data?.rows ?? []));

	// Its own read, so a fault in one does not blank the other: the two share
	// this page and nothing else.
	const dead = createQuery(() => ({
		queryKey: queryKeys.adminDeadLetters,
		queryFn: () => api.adminDeadLetters()
	}));
</script>

<div class="page">
	<PageHead
		icon="chart-line"
		title="Import drain"
		description="Per account, the share of canonical terms each import raised a new question for. It should fall as the account's crosswalk fills in."
	>
		{#snippet aside()}
			<StatusPill tone="soon" label="all accounts" />
		{/snippet}
	</PageHead>

	<!-- Counted rather than listed: which topic and how many tenants is the
	     whole operator question, and a row would carry a run summary an
	     operator has no reading for. -->
	<Panel title="Dead letters">
		{#if dead.isPending}
			<p class="quiet">Loading dead letters…</p>
		{:else if dead.isError}
			<p class="quiet">We could not load dead letters.</p>
		{:else}
			<p class="s">{deadLetterHeadline(dead.data.topics)}</p>
			{#if dead.data.topics.length > 0}
				<div class="op-table">
					<table>
						<thead>
							<tr>
								<th>Topic</th>
								<th class="num">Messages</th>
								<th class="num">Organisations</th>
							</tr>
						</thead>
						<tbody>
							{#each dead.data.topics as entry (entry.topic)}
								<tr>
									<td data-label="Topic">{topicLabel(entry.topic)}</td>
									<td class="num op-flag" data-label="Messages">{entry.messages}</td>
									<td class="num" data-label="Organisations">{entry.orgs}</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
				<p class="foot-note">
					A message is dead after twelve failed attempts, or when the relay refuses it for good.
					The drainer never retries it. Many organisations with one each points to the relay or
					its key; one organisation with many points to an address the relay refuses.
				</p>
			{/if}
		{/if}
	</Panel>

	{#if drain.isPending}
		<Panel><p class="quiet">Loading drain measurements…</p></Panel>
	{:else if drain.isError}
		<Panel><p class="quiet">We could not load drain measurements.</p></Panel>
	{:else if readout.series.length === 0}
		{@const empty = emptyState(readout.unreadable)}
		<Panel>
			<Placeholder icon={empty.icon} headline={empty.headline} body={empty.body} />
		</Panel>
	{:else}
		{#each readout.series as entry (entry.org)}
			<Panel title={entry.org_name}>
				<p class="s">
					{#if entry.gate.gap === 'short'}
						{entry.runs.length} of 10 migrations recorded; the gate compares the first against the
						tenth.
					{:else if entry.gate.gap === 'unmeasurable'}
						{entry.runs.length} migrations recorded, but the first or the tenth projected no
						canonical term, so the gate has no share at that end to compare.
					{:else}
						First {formatShare(entry.gate.first?.share ?? null)} → tenth
						{formatShare(entry.gate.tenth?.share ?? null)}: a fall of {formatPoints(
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
									<td data-label="Direction"
										>{SHORT_NAME[run.source]} → {SHORT_NAME[run.target]}</td
									>
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
		{#if drain.data?.truncated}
			<p class="foot-note op-unreadable">
				The read stops at 500 measurements and hit that limit. Rows are ordered by
				organisation name, so an account late in the alphabet may be cut short or missing.
			</p>
		{/if}
	{/if}

	<!-- The mixed case only: with no series at all the placeholder above is
	     already the report of this fault, and two lines saying it is one too
	     many. -->
	{#if readout.series.length > 0 && readout.unreadable > 0}
		<p class="foot-note op-unreadable">
			{readout.unreadable === 1
				? '1 row could not be read and is not shown.'
				: `${readout.unreadable} rows could not be read and are not shown.`}
		</p>
	{/if}
</div>
