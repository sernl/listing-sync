<script lang="ts">
	import { page } from '$app/state';
	import { api, type ItemDetail, type ItemView, type JobView } from '$lib/api';
	import { agoLabel } from '$lib/elapsed';
	import { gateLabel } from '$lib/gates';
	import { createLedger, type Ledger } from '$lib/ledger';
	import { segments } from '$lib/outcome';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
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

<div class="page">
	{#if job}
		{@const run = job}
		<PageHead
			icon="⇄"
			title={`Run ${run.job.slice(0, 8)}…`}
			description={`${run.inventory} · started ${agoLabel(run.created_at, Date.now())}`}
		>
			{#snippet aside()}
				<span class="pill {run.phase === 'settled' ? 'ok' : 'run'}">{run.phase}</span>
				<span class="tag-note">{live ? 'live' : 'reconnecting…'}</span>
			{/snippet}
		</PageHead>

		<Panel
			title="Outcomes"
			description="Every outcome the ledger recorded, kept apart rather than collapsed into a verdict."
		>
			<div class="outcome" role="img" aria-label="outcome distribution">
				{#each segments(run.counts) as segment (segment.label)}
					<div
						class={segment.tone}
						style="width: {segment.share * 100}%"
						title="{segment.label}: {segment.count}"
					></div>
				{/each}
			</div>
			<p class="foot-note">
				{#each segments(run.counts) as segment (segment.label)}
					{segment.label}
					{segment.count} ·
				{/each}
				total {run.counts.total}
			</p>
		</Panel>

		<Panel title="Items" description="Open one for its step timeline and its stored result.">
			{#if items.length === 0}
				<p class="quiet">No items recorded on this run yet.</p>
			{/if}
			{#each items as item (item.item)}
				<div class="item">
					<button class="item-head" onclick={() => expand(item)}>
						<span class="mono" title={item.item}>{item.item.slice(0, 8)}…</span>
						<span class="pill mut">{item.state}</span>
						{#if item.outcome}
							<span class="pill mut">{item.outcome}</span>
						{/if}
						{#if item.blocked_on}
							<span class="pill run">blocked on {gateLabel(item.blocked_on)}</span>
						{/if}
						{#if item.failure_code}
							<span class="pill bad">{item.failure_code}</span>
						{/if}
						<span class="grow"></span>
						<span class="when">attempts {item.attempt_count}</span>
					</button>
					{#if expanded[item.item]}
						{@const detail = expanded[item.item]}
						<div class="item-detail">
							<div class="head-row">
								<h2>Step timeline</h2>
								<span class="grow"></span>
								<button class="btn small" onclick={() => download(detail)}>
									Download result
								</button>
							</div>
							{#if detail.events.length === 0}
								<p class="quiet">No steps recorded yet.</p>
							{:else}
								<ol class="steps">
									{#each detail.events as event (event.org_seq)}
										<li>
											<span class="mono">#{event.org_seq}</span>
											<b>{event.kind}</b>
											<span class="s">{new Date(event.created_at).toLocaleTimeString()}</span>
										</li>
									{/each}
								</ol>
							{/if}
							{#if detail.failure_detail}
								<p class="refusal">{detail.failure_detail}</p>
							{/if}
						</div>
					{/if}
				</div>
			{/each}
			{#if nextCursor}
				<div class="actions">
					<button class="btn" onclick={more}>Load more items</button>
				</div>
			{/if}
		</Panel>
	{:else}
		<p class="quiet">Loading the run…</p>
	{/if}
</div>
