<script lang="ts">
	import { api, type DrainStats, type QueueItem } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { formatShare, gateWindow, readMeasurement, toRun, type DrainRun } from '$lib/drain';
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
	let runs = $state<DrainRun[]>([]);
	let truncated = $state(false);

	const gate = $derived(gateWindow(runs));
	const questions = $derived(questionRows(items, Date.now()));

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
		const ledger = createLedger((cursor) => new EventSource(`/v1/events/stream?cursor=${cursor}`));
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
			toast('error', TARGET_REFUSAL);
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

	<Panel
		title="Import drain"
		description="The share of canonical terms each import raised a new question for."
	>
		{#if runs.length === 0}
			<p class="quiet">
				No import has recorded a drain report yet. Each run records one, and the ledger keeps 30
				days of them.
			</p>
		{:else}
			<p class="s">
				{#if gate.fall === null}
					{runs.length} of 10 migrations recorded; the gate compares the first against the tenth.
				{:else}
					First {formatShare(gate.first?.share ?? null)} → tenth
					{formatShare(gate.tenth?.share ?? null)}: a fall of {formatShare(gate.fall)}.
				{/if}
			</p>

			<div class="drain-wrap">
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
								<td class="num {run.terms_unmapped > 0 ? 'flag' : ''}">{run.terms_unmapped}</td>
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
				Unmapped terms never became canonical, so they are outside the share: a share that falls
				while that column rises is an ingest gap rather than a converging crosswalk.
			</p>
		{/if}

		{#if truncated}
			<Banner tone="warn">
				The ledger was pruned past the start of this stream, so runs older than the 30-day window
				are not shown and the first row above may not be the first migration.
			</Banner>
		{/if}
	</Panel>
</div>
