<script lang="ts">
	import { api, type DrainStats, type QueueItem } from '$lib/api';
	import { toast } from '$lib/toast';

	let items = $state<QueueItem[]>([]);
	let stats = $state<DrainStats | null>(null);
	let loaded = $state(false);
	let drafts = $state<Record<string, string>>({});
	let busy = $state<string | null>(null);

	async function refetch() {
		const [queue, drain] = await Promise.all([api.queue(), api.drainStats()]);
		items = queue.items;
		stats = drain;
		loaded = true;
	}

	$effect(() => {
		void refetch();
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
