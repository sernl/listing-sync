<script lang="ts">
	import { api, type DrainStats, type QueueItem } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import { createLedger } from '$lib/ledger';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { toast } from '$lib/toast';
	import {
		DRAINED_BODY,
		DRAINED_TITLE,
		TARGET_PLACEHOLDER,
		TARGET_REFUSAL,
		questionRows,
		tallyLine
	} from '$lib/pages/automations/questions';
	import '$lib/pages/automations/automations.css';

	let items = $state<QueueItem[]>([]);
	let stats = $state<DrainStats | null>(null);
	let loaded = $state(false);
	let drafts = $state<Record<string, string>>({});
	let busy = $state<string | null>(null);

	const questions = $derived(questionRows(items, Date.now()));

	async function refetch() {
		const [queue, drain] = await Promise.all([api.queue(), api.drainStats()]);
		items = queue.items;
		stats = drain;
		loaded = true;
	}

	$effect(() => {
		void refetch();
		// An import finishing is when new questions appear, so the queue is
		// refetched on the event that reports one rather than polled. Events
		// are kept by seq rather than read out of the store's rolling buffer,
		// which a busy tenant's item traffic would push them out of.
		const seen = new Set<number>();
		const ledger = createLedger((cursor) => new EventSource(`/v1/events/stream?cursor=${cursor}`));
		const unsubscribe = ledger.subscribe((state) => {
			let fresh = false;
			for (const event of state.events) {
				if (event.kind === 'ImportDrainMeasured' && !seen.has(event.seq)) {
					seen.add(event.seq);
					fresh = true;
				}
			}
			if (fresh) {
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
			toast('error', TARGET_REFUSAL);
			return;
		}
		busy = item.id;
		try {
			await api.resolve(item.id, segments);
			toast('info', 'Saved. Every later resource with this word will use it.');
			await refetch();
		} catch {
			toast('error', 'That answer was refused. The path may already be in use.');
		} finally {
			busy = null;
		}
	}

	async function noCounterpart(item: QueueItem) {
		const sure = confirm(
			'Record that this word has no match on that marketplace? Listings will ' +
				'leave it out from now on.'
		);
		if (!sure) {
			return;
		}
		busy = item.id;
		try {
			await api.noCounterpart(item.id);
			toast('info', 'Recorded. That word will be left out instead of holding things up.');
			await refetch();
		} finally {
			busy = null;
		}
	}
</script>

<div class="page">
	<PageHead
		icon="circle-question-mark"
		back={{ href: '/sync', label: 'Back to Marketplace Sync' }}
		title="Open questions"
		description="The few questions sync cannot answer for you."
	>
		{#snippet aside()}
			{#if stats}
				<span class="queue-tally">{tallyLine(stats.open, stats.resolved, stats.no_counterpart)}</span>
			{/if}
		{/snippet}
	</PageHead>

	<Panel title="Every question waiting" description="Each one you answer stays answered.">
		{#if !loaded}
			<p class="quiet">Loading the queue…</p>
		{:else if questions.length === 0}
			<Placeholder icon="circle-check" headline={DRAINED_TITLE} body={DRAINED_BODY} />
		{:else}
			{#each questions as question (question.id)}
				<div class="question-row">
					<div class="t">{question.title}</div>
					<div class="meta">{question.meta}</div>
					<div class="answer">
						<label class="sr-only" for={`target-${question.id}`}>Target path</label>
						<input
							id={`target-${question.id}`}
							type="text"
							placeholder={TARGET_PLACEHOLDER}
							bind:value={drafts[question.id]}
						/>
						<Button
							tier="primary"
							disabled={busy === question.id}
							reason={busy === question.id ? 'This answer is being recorded.' : undefined}
							onclick={() => void resolve(question.item)}
						>
							Resolve
						</Button>
						<Button
							tier="outline"
							disabled={busy === question.id}
							reason={busy === question.id ? 'This answer is being recorded.' : undefined}
							onclick={() => void noCounterpart(question.item)}
						>
							No counterpart
						</Button>
					</div>
				</div>
			{/each}
		{/if}
	</Panel>
</div>
