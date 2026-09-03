<script lang="ts">
	import { api, type DrainStats, type QueueItem } from '$lib/api';
	import { formatShare, gateWindow, readMeasurement, toRun, type DrainRun } from '$lib/drain';
	import { createLedger } from '$lib/ledger';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { toast } from '$lib/toast';

	let items = $state<QueueItem[]>([]);
	let stats = $state<DrainStats | null>(null);
	let loaded = $state(false);
	let drafts = $state<Record<string, string>>({});
	let busy = $state<string | null>(null);
	let runs = $state<DrainRun[]>([]);
	let truncated = $state(false);

	const gate = $derived(gateWindow(runs));

	async function refetch() {
		const [queue, drain] = await Promise.all([api.queue(), api.drainStats()]);
		items = queue.items;
		stats = drain;
		loaded = true;
	}

	$effect(() => {
		void refetch();
		// The stream replays the whole un-pruned ledger from cursor zero, so
		// the series rebuilds on load without a route of its own. Events are
		// kept by seq here rather than read out of the store's rolling buffer,
		// which a busy tenant's item traffic would push them out of.
		const seen = new Map<number, DrainRun>();
		let resyncs = 0;
		const ledger = createLedger(
			(cursor) => new EventSource(`/v1/events/stream?cursor=${cursor}`)
		);
		const unsubscribe = ledger.subscribe((state) => {
			if (state.resyncs !== resyncs) {
				resyncs = state.resyncs;
				truncated = true;
			}
			let fresh = false;
			for (const event of state.events) {
				if (event.kind !== 'ImportDrainMeasured' || seen.has(event.seq)) {
					continue;
				}
				const measurement = readMeasurement(event.payload);
				if (measurement) {
					seen.set(event.seq, toRun(event.seq, measurement));
					fresh = true;
				}
			}
			if (fresh) {
				runs = [...seen.values()].sort((left, right) => left.seq - right.seq);
				void refetch();
			}
		});
		return () => {
			unsubscribe();
			ledger.close();
		};
	});

	async function resolve(item: QueueItem) {
		const draft = (drafts[item.id] ?? '').trim();
		const segments = draft
			.split('/')
			.map((part) => part.trim())
			.filter((part) => part.length > 0);
		if (segments.length === 0) {
			toast('error', 'Give the target path as segments separated by “/”.');
			return;
		}
		busy = item.id;
		try {
			await api.resolve(item.id, segments);
			toast('info', 'Resolved; every later product carrying this term finds the edge.');
			await refetch();
		} catch {
			toast('error', 'The resolution was refused; the path may already be claimed.');
		} finally {
			busy = null;
		}
	}

	async function noCounterpart(item: QueueItem) {
		const sure = confirm(
			'Record that this term has no counterpart? Items will omit it in ' +
				'this inventory from now on.'
		);
		if (!sure) {
			return;
		}
		busy = item.id;
		try {
			await api.noCounterpart(item.id);
			toast('info', 'Recorded; the term is omitted rather than blocking.');
			await refetch();
		} finally {
			busy = null;
		}
	}
</script>

<div class="page">
	<PageHead
		icon="☰"
		title="Reconciliation"
		description="The few questions sync cannot answer for you."
	>
		{#snippet aside()}
			{#if stats}
				<span class="tag-note">
					open {stats.open} · resolved {stats.resolved} · no counterpart {stats.no_counterpart}
				</span>
			{/if}
		{/snippet}
	</PageHead>

	<Panel
		title="Import drain"
		description="The share of canonical terms each import raised a new item for."
	>
		{#if runs.length === 0}
			<p class="quiet">
				No import has recorded a drain report yet. Each run records one, and the ledger keeps
				30 days of them.
			</p>
		{:else}
			<p class="s">
				{#if gate.fall === null}
					{runs.length} of 10 migrations recorded; the gate compares the first against the
					tenth.
				{:else}
					First {formatShare(gate.first?.share ?? null)} → tenth
					{formatShare(gate.tenth?.share ?? null)}: a fall of {formatShare(gate.fall)}.
				{/if}
			</p>

			<div class="tbl-wrap">
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
						{#each runs as run, index (run.seq)}
							<tr class={index === 0 || index === 9 ? 'gate-row' : ''}>
								<td class="num">{index + 1}</td>
								<td>{run.source} → {run.target}</td>
								<td class="num">{run.rows}</td>
								<td class="num">{run.terms_seen}</td>
								<td class="num {run.terms_unmapped > 0 ? 'flag' : ''}">
									{run.terms_unmapped}
								</td>
								<td class="num">{run.terms_covered}</td>
								<td class="num">{run.items_new}</td>
								<td class="num">{run.items_already_open}</td>
								<td class="num">{formatShare(run.share)}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>

			<p class="foot-note">
				Unmapped terms never became canonical, so they are outside the share: a share that
				falls while that column rises is an ingest gap rather than a converging crosswalk.
			</p>
		{/if}

		{#if truncated}
			<p class="notice">
				The ledger was pruned past the start of this stream, so runs older than the 30-day
				window are not shown and the first row above may not be the first migration.
			</p>
		{/if}
	</Panel>

	<Panel title="Open questions" description="Each one you answer stays answered.">
		{#if !loaded}
			<p class="quiet">Loading the queue…</p>
		{:else if items.length === 0}
			<div class="placeholder">
				<span class="big" aria-hidden="true">✓</span>
				<b>The queue is drained</b>
				<p>
					New items appear only when a listing carries a term with no translation yet — and
					each one you resolve stays resolved.
				</p>
			</div>
		{:else}
			{#each items as item (item.id)}
				<div class="question">
					<div class="head-row">
						<span class="mono" title={item.term}>{item.term.slice(0, 8)}…</span>
						<span class="badge">{item.kind} → {item.inventory}</span>
						<span class="grow"></span>
						<span class="when">
							raised {new Date(item.raised_at).toLocaleDateString()}
						</span>
					</div>
					<div class="actions">
						<label class="sr-only" for={`target-${item.id}`}>Target path</label>
						<input
							id={`target-${item.id}`}
							class="grow"
							type="text"
							placeholder="Target path, e.g. Mathematics / Algebra"
							bind:value={drafts[item.id]}
						/>
						<button class="cta" disabled={busy === item.id} onclick={() => resolve(item)}>
							Resolve
						</button>
						<button
							class="btn"
							disabled={busy === item.id}
							onclick={() => noCounterpart(item)}
						>
							No counterpart
						</button>
					</div>
				</div>
			{/each}
		{/if}
	</Panel>
</div>
