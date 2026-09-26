<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import { emptyState, formatPoints, formatShare, seriesByOrg } from '$lib/drain';
	import Explain from '$lib/Explain.svelte';
	import FlowStep from '$lib/FlowStep.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { SHORT_NAME } from '$lib/platforms';
	import StatusPill from '$lib/StatusPill.svelte';
	import { queryKeys } from '$lib/query';
	import { DRAIN_WORD, drainVerdict, type DrainVerdict } from '$lib/pages/admin/admin-view';
	import { deadLetterHeadline, topicLabel } from '$lib/pages/admin/dead-letters';
	import '$lib/flow.css';
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

	const FILTERS: readonly (DrainVerdict | 'all')[] = [
		'all',
		'flat',
		'draining',
		'short',
		'unmeasurable'
	];
	let filter = $state<DrainVerdict | 'all'>('all');
	const judged = $derived(
		readout.series.map((entry) => ({ entry, verdict: drainVerdict(entry.gate) }))
	);
	const shown = $derived(judged.filter((row) => filter === 'all' || row.verdict === filter));
</script>

<div class="page flow-page">
	<PageHead
		icon="chart-line"
		title="Import drain"
		description="Per account, the share of terms each import raised a new question for."
	/>

	<div class="flow">
		<FlowStep
			n={1}
			id="dead"
			title="Dead letters"
			hint="Messages the drainer gave up on, by topic."
			summary={dead.data === undefined ? undefined : deadLetterHeadline(dead.data.topics)}
			done={dead.data !== undefined && dead.data.topics.length === 0}
		>
			{#snippet aside()}
				<Explain title="Dead letters" label="">
					<p>
						A message is dead after twelve failed attempts, or when the relay refuses it for good.
						The drainer never retries it.
					</p>
					<p>
						Many organisations with one each points to the relay or its key; one organisation
						with many points to an address the relay refuses.
					</p>
				</Explain>
			{/snippet}
			{#if dead.isPending}
				<p class="quiet">Loading dead letters…</p>
			{:else if dead.isError}
				<p class="quiet">We could not load dead letters.</p>
			{:else if dead.data.topics.length === 0}
				<p class="quiet">{deadLetterHeadline(dead.data.topics)}</p>
			{:else}
				<div class="flow-table-wrap op-table op-keep">
					<table class="flow-table">
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
			{/if}
		</FlowStep>

		<FlowStep
			n={2}
			id="drain"
			title="Drain by account"
			hint="The share should fall from the first import to the tenth."
			summary={`${readout.series.length} accounts measured`}
		>
			{#snippet aside()}
				<Explain title="How the drain is read" label="">
					<p>
						Share is the fraction of canonical terms an import raised a new question for. The gate
						compares the first recorded migration with the tenth; the two are bold in each table.
					</p>
					<p>
						Unmapped terms never became canonical, so they are outside the share: a share that
						falls while Unmapped rises is an ingest gap rather than a converging crosswalk.
					</p>
					<p>
						The ledger is pruned past 30 days, so the first row may not be that organisation's
						first migration.
					</p>
				</Explain>
			{/snippet}

			{#if drain.isPending}
				<p class="quiet">Loading drain measurements…</p>
			{:else if drain.isError}
				<p class="quiet">We could not load drain measurements.</p>
			{:else if readout.series.length === 0}
				{@const empty = emptyState(readout.unreadable)}
				<Placeholder icon={empty.icon} headline={empty.headline} body={empty.body} />
			{:else}
				<div class="op-filters" role="group" aria-label="Show">
					{#each FILTERS as chip (chip)}
						<button
							type="button"
							class="op-chip"
							aria-pressed={filter === chip}
							onclick={() => (filter = chip)}
						>
							{chip === 'all' ? 'All' : DRAIN_WORD[chip].label.replace(/^./, (first) => first.toUpperCase())}
							<span class="c">
								{chip === 'all'
									? judged.length
									: judged.filter((row) => row.verdict === chip).length}
							</span>
						</button>
					{/each}
				</div>

				<div class="flow-table-wrap op-table">
					<table class="flow-table">
						<thead>
							<tr>
								<th>Account</th>
								<th>Status</th>
								<th class="num">Runs</th>
								<th class="num">First → tenth</th>
								<th class="num">Fall</th>
							</tr>
						</thead>
						<tbody>
							{#each shown as { entry, verdict } (entry.org)}
								<tr>
									<td data-label="Account">
										<a class="op-name" href={`#drain-${entry.org}`}>{entry.org_name}</a>
									</td>
									<td data-label="Status">
										<StatusPill tone={DRAIN_WORD[verdict].tone} label={DRAIN_WORD[verdict].label} />
									</td>
									<td class="num" data-label="Runs">{entry.runs.length}</td>
									<td class="num" data-label="First → tenth">
										{formatShare(entry.gate.first?.share ?? null)} → {formatShare(
											entry.gate.tenth?.share ?? null
										)}
									</td>
									<td class="num" data-label="Fall">
										{entry.gate.fall === null ? '—' : formatPoints(entry.gate.fall)}
									</td>
								</tr>
							{:else}
								<tr><td colspan="5" class="quiet">None under this filter.</td></tr>
							{/each}
						</tbody>
					</table>
				</div>

				{#each shown as { entry } (entry.org)}
					<details class="flow-more" id={`drain-${entry.org}`}>
						<summary>{entry.org_name} · {entry.runs.length} runs</summary>
						<div class="flow-table-wrap op-table">
							<table class="flow-table">
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
										<tr class:op-strong={index === 0 || index === 9}>
											<td class="num" data-label="#">{index + 1}</td>
											<td data-label="Direction">
												{SHORT_NAME[run.source]} → {SHORT_NAME[run.target]}
											</td>
											<td class="num" data-label="Rows">{run.rows}</td>
											<td class="num" data-label="Terms seen">{run.terms_seen}</td>
											<td
												class="num"
												class:op-flag={run.terms_unmapped > 0}
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
					</details>
				{/each}

				{#if drain.data?.truncated}
					<p class="flow-warn">
						The read stopped at 500 measurements. Accounts late in the alphabet may be cut short
						or missing.
					</p>
				{/if}
				{#if readout.unreadable > 0}
					<p class="flow-warn">
						{readout.unreadable === 1
							? '1 row could not be read and is not shown.'
							: `${readout.unreadable} rows could not be read and are not shown.`}
					</p>
				{/if}
			{/if}
		</FlowStep>
	</div>
</div>
