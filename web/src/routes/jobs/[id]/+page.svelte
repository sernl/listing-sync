<script lang="ts">
	import { page } from '$app/state';
	import { api, type ItemDetail, type ItemView, type JobView } from '$lib/api';
	import { segments } from '$lib/outcome';
	import { createLedger, type Ledger } from '$lib/ledger';
	import { toast } from '$lib/toast';

	const jobId = $derived(page.params.id ?? '');

	let job = $state<JobView | null>(null);
	let items = $state<ItemView[]>([]);
	let nextCursor = $state<string | null>(null);
	let expanded = $state<Record<string, ItemDetail>>({});
	let live = $state(false);
	let ledger: Ledger | null = null;

	async function refetch() {
		if (!jobId) {
			return;
		}
		job = await api.job(jobId);
		const first = await api.items(jobId);
		items = first.items;
		nextCursor = first.next_cursor;
	}

	async function more() {
		if (!jobId || !nextCursor) {
			return;
		}
		const next = await api.items(jobId, nextCursor);
		items = [...items, ...next.items];
		nextCursor = next.next_cursor;
	}

	$effect(() => {
		void refetch();
		ledger = createLedger(
			(cursor) => new EventSource(`/v1/events/stream?cursor=${cursor}`)
		);
		const unsubscribe = ledger.subscribe((state) => {
			live = state.connected;
			if (state.events.length > 0 || state.resyncs > 0) {
				void refetch();
			}
		});
		return () => {
			unsubscribe();
			ledger?.close();
		};
	});

	async function expand(item: ItemView) {
		if (expanded[item.item]) {
			const next = { ...expanded };
			delete next[item.item];
			expanded = next;
			return;
		}
		expanded = { ...expanded, [item.item]: await api.item(jobId, item.item) };
	}

	function download(detail: ItemDetail) {
		const blob = new Blob([JSON.stringify(detail, null, 2)], {
			type: 'application/json'
		});
		const url = URL.createObjectURL(blob);
		const anchor = document.createElement('a');
		anchor.href = url;
		anchor.download = `item-${detail.item.slice(0, 8)}.json`;
		anchor.click();
		URL.revokeObjectURL(url);
		toast('info', 'Item result downloaded.');
	}
</script>

{#if job}
	<div class="mb-4 flex items-center gap-3">
		<h1 class="text-xl font-semibold">Job {job.job.slice(0, 8)}…</h1>
		<span class="rounded bg-slate-100 px-2 py-0.5 text-xs">{job.inventory}</span>
		<span
			class="rounded px-2 py-0.5 text-xs
				{job.phase === 'settled' ? 'bg-emerald-100 text-emerald-800' : 'bg-sky-100 text-sky-800'}"
		>
			{job.phase}
		</span>
		<span class="text-xs text-slate-400">{live ? 'live' : 'reconnecting…'}</span>
	</div>

	<div class="mb-1 flex h-4 overflow-hidden rounded bg-slate-200" role="img"
		aria-label="outcome distribution">
		{#each segments(job.counts) as segment (segment.label)}
			<div
				class={segment.tone}
				style="width: {segment.share * 100}%"
				title="{segment.label}: {segment.count}"
			></div>
		{/each}
	</div>
	<p class="mb-6 text-xs text-slate-500">
		{#each segments(job.counts) as segment, index (segment.label)}
			{index > 0 ? ' · ' : ''}{segment.label} {segment.count}
		{/each}
		· total {job.counts.total}
	</p>

	<ul class="divide-y divide-slate-100 rounded border border-slate-200 bg-white">
		{#each items as item (item.item)}
			<li class="px-4 py-3">
				<button class="flex w-full items-center gap-3 text-left" onclick={() => expand(item)}>
					<span class="font-mono text-xs text-slate-500">{item.item.slice(0, 8)}…</span>
					<span class="rounded bg-slate-100 px-2 py-0.5 text-xs">{item.state}</span>
					{#if item.outcome}
						<span class="rounded bg-slate-100 px-2 py-0.5 text-xs">{item.outcome}</span>
					{/if}
					{#if item.blocked_on}
						<span class="rounded bg-orange-100 px-2 py-0.5 text-xs text-orange-800">
							blocked on {item.blocked_on}
						</span>
					{/if}
					{#if item.failure_code}
						<span class="rounded bg-red-100 px-2 py-0.5 text-xs text-red-800">
							{item.failure_code}
						</span>
					{/if}
					<span class="grow"></span>
					<span class="text-xs text-slate-400">attempts {item.attempt_count}</span>
				</button>
				{#if expanded[item.item]}
					{@const detail = expanded[item.item]}
					<div class="mt-3 rounded bg-slate-50 p-3">
						<div class="mb-2 flex items-center gap-3">
							<h2 class="text-sm font-medium">Step timeline</h2>
							<button
								class="rounded border border-slate-300 px-2 py-0.5 text-xs"
								onclick={() => download(detail)}
							>
								Download result
							</button>
						</div>
						{#if detail.events.length === 0}
							<p class="text-xs text-slate-500">No steps recorded yet.</p>
						{:else}
							<ol class="flex flex-col gap-1">
								{#each detail.events as event (event.org_seq)}
									<li class="flex items-baseline gap-2 text-xs">
										<span class="font-mono text-slate-400">#{event.org_seq}</span>
										<span class="font-medium">{event.kind}</span>
										<span class="text-slate-500">
											{new Date(event.created_at).toLocaleTimeString()}
										</span>
									</li>
								{/each}
							</ol>
						{/if}
						{#if detail.failure_detail}
							<p class="mt-2 text-xs text-red-700">{detail.failure_detail}</p>
						{/if}
					</div>
				{/if}
			</li>
		{/each}
	</ul>
	{#if nextCursor}
		<button class="mt-3 rounded border border-slate-300 px-3 py-1 text-sm" onclick={more}>
			Load more items
		</button>
	{/if}
{:else}
	<p class="text-slate-500">Loading the job…</p>
{/if}
