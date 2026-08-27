<script lang="ts">
	import { api, type DrainStats, type QueueItem } from '$lib/api';
	import { formatShare, gateWindow, readMeasurement, toRun, type DrainRun } from '$lib/drain';
	import { createLedger } from '$lib/ledger';
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
			'Record that this term has no counterpart? Listings will omit it in ' +
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

<div class="mb-4 flex items-center gap-4">
	<h1 class="text-xl font-semibold">Reconciliation</h1>
	{#if stats}
		<span class="text-sm text-slate-500">
			open {stats.open} · resolved {stats.resolved} · no counterpart {stats.no_counterpart}
		</span>
	{/if}
</div>

<section class="mb-6 rounded border border-slate-200 bg-white p-4">
	<div class="mb-2 flex items-baseline gap-3">
		<h2 class="text-sm font-medium">Import drain</h2>
		<span class="text-xs text-slate-500">
			the share of canonical terms each import raised a new item for
		</span>
	</div>

	{#if runs.length === 0}
		<p class="text-xs text-slate-500">
			No import has recorded a drain report yet. Each run records one, and
			the ledger keeps 30 days of them.
		</p>
	{:else}
		<p class="mb-3 text-xs text-slate-600">
			{#if gate.fall === null}
				{runs.length} of 10 migrations recorded; the gate compares the first
				against the tenth.
			{:else}
				First {formatShare(gate.first?.share ?? null)} → tenth
				{formatShare(gate.tenth?.share ?? null)}: a fall of
				{formatShare(gate.fall)}.
			{/if}
		</p>

		<div class="overflow-x-auto">
			<table class="w-full text-left text-xs">
				<thead class="text-slate-500">
					<tr>
						<th class="py-1 pr-3 font-medium">#</th>
						<th class="py-1 pr-3 font-medium">Direction</th>
						<th class="py-1 pr-3 font-medium">Rows</th>
						<th class="py-1 pr-3 font-medium">Terms seen</th>
						<th class="py-1 pr-3 font-medium">Unmapped</th>
						<th class="py-1 pr-3 font-medium">Covered</th>
						<th class="py-1 pr-3 font-medium">New</th>
						<th class="py-1 pr-3 font-medium">Already open</th>
						<th class="py-1 font-medium">Share</th>
					</tr>
				</thead>
				<tbody class="divide-y divide-slate-100">
					{#each runs as run, index (run.seq)}
						<tr class={index === 0 || index === 9 ? 'font-medium' : ''}>
							<td class="py-1 pr-3 text-slate-400">{index + 1}</td>
							<td class="py-1 pr-3">{run.source} → {run.target}</td>
							<td class="py-1 pr-3">{run.rows}</td>
							<td class="py-1 pr-3">{run.terms_seen}</td>
							<td class="py-1 pr-3 {run.terms_unmapped > 0 ? 'text-orange-700' : ''}">
								{run.terms_unmapped}
							</td>
							<td class="py-1 pr-3">{run.terms_covered}</td>
							<td class="py-1 pr-3">{run.items_new}</td>
							<td class="py-1 pr-3">{run.items_already_open}</td>
							<td class="py-1">{formatShare(run.share)}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>

		<p class="mt-2 text-xs text-slate-500">
			Unmapped terms never became canonical, so they are outside the share:
			a share that falls while that column rises is an ingest gap rather
			than a converging crosswalk.
		</p>
	{/if}

	{#if truncated}
		<p class="mt-2 rounded bg-amber-50 p-2 text-xs text-amber-800">
			The ledger was pruned past the start of this stream, so runs older
			than the 30-day window are not shown and the first row above may not
			be the first migration.
		</p>
	{/if}
</section>

{#if !loaded}
	<p class="text-slate-500">Loading the queue…</p>
{:else if items.length === 0}
	<p class="rounded border border-emerald-200 bg-emerald-50 p-4 text-sm text-emerald-800">
		The queue is drained. New items appear only when a listing carries a term
		with no translation yet — and each one you resolve stays resolved.
	</p>
{:else}
	<ul class="flex flex-col gap-3">
		{#each items as item (item.id)}
			<li class="rounded border border-slate-200 bg-white p-4">
				<div class="mb-2 flex items-center gap-3">
					<span class="font-mono text-xs text-slate-500">{item.term.slice(0, 8)}…</span>
					<span class="rounded bg-slate-100 px-2 py-0.5 text-xs">
						{item.kind} → {item.inventory}
					</span>
					<span class="text-xs text-slate-400">
						raised {new Date(item.raised_at).toLocaleDateString()}
					</span>
				</div>
				<div class="flex items-center gap-2">
					<input
						class="grow rounded border border-slate-300 px-3 py-1.5 text-sm"
						placeholder="Target path, e.g. Mathematics / Algebra"
						bind:value={drafts[item.id]}
					/>
					<button
						class="rounded bg-slate-900 px-3 py-1.5 text-sm text-white disabled:opacity-50"
						disabled={busy === item.id}
						onclick={() => resolve(item)}
					>
						Resolve
					</button>
					<button
						class="rounded border border-slate-300 px-3 py-1.5 text-sm disabled:opacity-50"
						disabled={busy === item.id}
						onclick={() => noCounterpart(item)}
					>
						No counterpart
					</button>
				</div>
			</li>
		{/each}
	</ul>
{/if}
