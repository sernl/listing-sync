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
	let generation = 0;

	const questions = $derived(questionRows(items, Date.now()));

	async function refetch() {
		const current = ++generation;
		const [queue, drain] = await Promise.all([api.queue(), api.drainStats()]);
		if (current !== generation) return;
		items = queue.items;
		stats = drain;
		loaded = true;
	}

	$effect(() => {
		const ledger = createLedger((cursor) => new EventSource(`/v1/events/stream?cursor=${cursor}`));
		void refetch();
		let revision = 0;
		const unsubscribe = ledger.subscribe((state) => {
			if (state.revision === revision) return;
			revision = state.revision;
			if (state.kinds.has('resync') || state.kinds.has('ImportDrainMeasured')) {
				void refetch();
			}
		});
		return () => {
			generation += 1;
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
			toast('error', 'That answer was not saved. Something else may already use it.');
		} finally {
			busy = null;
		}
	}

	async function noCounterpart(item: QueueItem) {
		const sure = confirm(
			'Leave this word out on that marketplace from now on?'
		);
		if (!sure) {
			return;
		}
		busy = item.id;
		try {
			await api.noCounterpart(item.id);
			toast('info', 'Saved. That word will be left out from now on.');
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
		description="Words we could not match on a marketplace. Tell us where each one belongs."
	>
		{#snippet aside()}
			{#if stats}
				<span class="queue-tally">{tallyLine(stats.open, stats.resolved, stats.no_counterpart)}</span>
			{/if}
		{/snippet}
	</PageHead>

	<Panel title="Every question waiting" description="Each one you answer stays answered.">
		{#if !loaded}
			<p class="quiet">Loading…</p>
		{:else if questions.length === 0}
			<Placeholder icon="circle-check" headline={DRAINED_TITLE} body={DRAINED_BODY} />
		{:else}
			{#each questions as question (question.id)}
				<div class="question-row">
					<div class="t">{question.title}</div>
					<div class="meta">{question.meta}</div>
					<div class="answer">
						<label class="sr-only" for={`target-${question.id}`}>Where it belongs</label>
						<input
							id={`target-${question.id}`}
							type="text"
							placeholder={TARGET_PLACEHOLDER}
							bind:value={drafts[question.id]}
						/>
						<Button
							tier="primary"
							disabled={busy === question.id}
							reason={busy === question.id ? 'This answer is being saved.' : undefined}
							onclick={() => void resolve(question.item)}
						>
							Save answer
						</Button>
						<Button
							tier="outline"
							disabled={busy === question.id}
							reason={busy === question.id ? 'This answer is being saved.' : undefined}
							onclick={() => void noCounterpart(question.item)}
						>
							Leave it out
						</Button>
					</div>
				</div>
			{/each}
		{/if}
	</Panel>
</div>
