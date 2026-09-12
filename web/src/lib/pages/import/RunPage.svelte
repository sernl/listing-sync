<script lang="ts">
	import { page } from '$app/state';
	import {
		ApiFailure,
		api,
		type DuplicateDecision,
		type ImportRunView,
		type PairFieldChoice,
		type PairSide
	} from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { continueImportHere, desktopInvoker, startImportHere, stopImportHere } from '$lib/desktop';
	import { createLedger, type Ledger } from '$lib/ledger';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { FILES_STAY_ON_YOUR_COMPUTER } from '$lib/sync-request';
	import { NEEDS_THE_APP, startRefusal } from './import-view';
	import ReviewCards from './ReviewCards.svelte';
	import {
		READING_HAPPENS_ON_YOUR_COMPUTER,
		RUN_UNREAD,
		emptyItemsLine,
		importedHref,
		itemRows,
		runBadge,
		runName,
		selectionRows,
		selectionBlocked,
		settledLine,
		stageCopy,
		stageFrom
	} from './run-view';
	import './import.css';
	import './run.css';

	const runId = $derived(page.params.run ?? '');

	let view = $state<ImportRunView | null>(null);
	// Kept apart from `view` deliberately: a read that failed is not a run
	// with nothing in it, and this page exists to hold those two apart.
	let refusal = $state<string | null>(null);
	let live = $state(false);
	let ledger: Ledger | null = null;
	let generation = 0;
	let runEpoch = 0;

	function current(target: { id: string; epoch: number }): boolean {
		return target.id === runId && target.epoch === runEpoch;
	}

	// Read once: whether this console runs inside the desktop application does
	// not change while the page is open.
	const invoke = desktopInvoker();

	// The tick list's own state, keyed by locator. Held here rather than
	// derived from the items, because a seller half-way through ticking must
	// not have their choices rewritten by a refetch the ledger triggered.
	let chosen = $state<Set<string>>(new Set());
	let sending = $state(false);
	let declined = $state<string | null>(null);

	let committing = $state(false);
	let commitRefusal = $state<string | null>(null);
	let stopPending = $state(false);

	const stage = $derived(view === null ? null : stageFrom(view));
	const rows = $derived(view === null ? [] : itemRows(view));
	const offered = $derived(view === null ? [] : selectionRows(view));
	const chosenCount = $derived(offered.filter((item) => chosen.has(item.locator)).length);

	async function refetch() {
		if (!runId) {
			return;
		}
		const id = runId;
		const current = ++generation;
		try {
			const next = await api.importRun(id);
			if (current !== generation || id !== runId) return;
			view = next;
			refusal = null;
		} catch (caught) {
			if (current !== generation || id !== runId) return;
			refusal = caught instanceof ApiFailure ? caught.message : RUN_UNREAD;
		}
	}

	// The same liveness the request page beside this one uses: one event
	// stream per tab, and a refetch when the ledger moves or resyncs. The
	// payloads are never read — the snapshot is what the page renders, so a
	// projection here would be a second copy of the server's own counting.
	$effect(() => {
		const id = runId;
		void id;
		runEpoch += 1;
		view = null;
		refusal = null;
		chosen = new Set();
		sending = false;
		committing = false;
		declined = null;
		commitRefusal = null;
		stopPending = false;
		ledger = createLedger((cursor) => new EventSource(`/v1/events/stream?cursor=${cursor}`));
		void refetch();
		let revision = 0;
		const unsubscribe = ledger.subscribe((state) => {
			live = state.connected;
			if (state.revision === revision) return;
			revision = state.revision;
			if ([...state.kinds].some((kind) => kind === 'resync' || kind.startsWith('ImportRun'))) {
				void refetch();
			}
		});
		return () => {
			runEpoch += 1;
			generation += 1;
			unsubscribe();
			ledger?.close();
		};
	});

	function toggle(locator: string) {
		const next = new Set(chosen);
		if (next.has(locator)) {
			next.delete(locator);
		} else {
			next.add(locator);
		}
		chosen = next;
	}

	function selectAll() {
		chosen = new Set(offered.map((item) => item.locator));
	}

	/** The server freezes the selected rows before the device begins reading. */
	async function continueHere() {
		if (sending || chosenCount === 0) return;
		const target = { id: runId, epoch: runEpoch };
		const selection = chosenCount === offered.length
			? { all: true as const } : { locators: [...chosen] };
		sending = true;
		declined = null;
		try {
			const selected = await api.selectImportRun(target.id, selection);
			if (current(target)) view = selected;
			const outcome = await continueImportHere(invoke, target.id);
			if (current(target)) {
				declined = startRefusal(outcome) ?? (outcome.kind === 'unavailable' ? NEEDS_THE_APP : null);
			}
		} catch (caught) {
			if (current(target)) {
				declined = caught instanceof ApiFailure
					? caught.message : 'Your choice was not acknowledged. Check this import before retrying.';
			}
		} finally {
			if (current(target)) {
				sending = false;
				await refetch();
			}
		}
	}

	/** Confirmation is durable; the server owns the work after this request. */
	async function commit() {
		if (committing) return;
		const target = { id: runId, epoch: runEpoch };
		committing = true;
		commitRefusal = null;
		try {
			const confirmed = await api.confirmImportRun(target.id);
			if (current(target)) view = confirmed.run;
		} catch (caught) {
			if (current(target)) {
				commitRefusal = caught instanceof ApiFailure
					? caught.message : 'The confirmation was not acknowledged. Check the import before retrying.';
			}
		} finally {
			if (current(target)) {
				committing = false;
				await refetch();
			}
		}
	}

	async function decide(lo: string, hi: string, decision: DuplicateDecision) {
		const target = { id: runId, epoch: runEpoch };
		try {
			await api.decideDuplicate(lo, hi, decision);
			if (current(target)) commitRefusal = null;
		} catch (caught) {
			if (current(target)) {
				commitRefusal = caught instanceof ApiFailure ? caught.message : 'That answer was not acknowledged.';
			}
		}
		if (current(target)) await refetch();
	}

	async function resume() {
		if (sending || view === null) return;
		const target = { id: runId, epoch: runEpoch };
		const enumerated = view.execution.enumeration_complete;
		if (!window.confirm('Resume this import on this device? Any previous device will lose ownership.')) return;
		sending = true;
		declined = null;
		try {
			const outcome = enumerated
				? await continueImportHere(invoke, target.id, true)
				: await startImportHere(invoke, target.id, true);
			if (current(target)) {
				if (outcome.kind === 'refused') declined = outcome.detail;
				if (outcome.kind === 'unavailable') declined = NEEDS_THE_APP;
			}
		} finally {
			if (current(target)) {
				sending = false;
				await refetch();
			}
		}
	}

	async function abandon() {
		if (sending) return;
		const target = { id: runId, epoch: runEpoch };
		let locallyStopped = false;
		sending = true;
		declined = null;
		try {
			const local = await stopImportHere(invoke, target.id);
			if (local.kind === 'stopped') {
				locallyStopped = true;
				if (current(target)) {
					stopPending = local.serverPending;
				}
			}
			await api.abandonImportRun(target.id);
			if (current(target)) stopPending = false;
		} catch (caught) {
			if (current(target)) {
				declined = locallyStopped
					? 'Stopped on this device; server confirmation pending.'
					: caught instanceof ApiFailure ? caught.message : 'The stop request was not acknowledged.';
			}
		} finally {
			if (current(target)) {
				sending = false;
				await refetch();
			}
		}
	}

	function merged(keep: string, fields: PairFieldChoice): DuplicateDecision {
		return { verdict: 'same', keep, fields };
	}

	function sideOf(value: string): PairSide {
		return value === 'lo' ? 'lo' : 'hi';
	}
</script>

<div class="page">
	{#if view !== null && stage !== null}
		{@const run = view}
		{@const badge = runBadge(run)}
		{@const copy = stageCopy(stage)}
		<PageHead
			icon="download"
			back={{ href: '/import', label: 'Back to Import' }}
			title={run.source === null ? 'Import' : `Import from ${runName(run)}`}
			description={`Started ${new Date(run.created_at).toLocaleString('en-GB')}`}
		>
			{#snippet aside()}
				<StatusPill tone={badge.tone} label={badge.label} />
				<StatusPill tone={live ? 'ok' : 'soon'} label={live ? 'Updates connected' : 'Updates reconnecting'} />
			{/snippet}
		</PageHead>

		{#if refusal !== null}
			<Banner tone="bad" title="We could not read this import just now">
				{refusal} Below is the last state we read.
			</Banner>
		{/if}
		{#if declined !== null}
			<Banner tone={stopPending ? 'info' : 'bad'} title={stopPending ? 'Stopped locally' : 'This device did not carry on'}>{declined}</Banner>
		{/if}

		<Panel title="Where this import stands">
			<p class="import-stage">{copy.headline}</p>
			<p class="quiet">{copy.detail}</p>

			<p class="quiet">
				{run.execution.discovered} resources found.
				{#if run.execution.selected_total !== null}{run.execution.selected_total} selected.{/if}
			</p>
			{#if run.execution.reason !== null}
				<p class="run-bar">{run.execution.reason}</p>
			{/if}
			{#if run.execution.owner_device !== null}
				<p class="quiet">Reading device: {run.execution.owner_device} · attempt {run.execution.attempt}.</p>
			{/if}
			{#if run.execution.last_contact_at !== null}
				<p class="quiet">Last device contact: {new Date(run.execution.last_contact_at).toLocaleString('en-GB')}.</p>
			{/if}
			{#if run.execution.last_progress_at !== null}
				<p class="quiet">Last progress: {new Date(run.execution.last_progress_at).toLocaleString('en-GB')}.</p>
			{/if}
			{#if (stage === 'reading' || stage === 'committing') && run.execution.selected_total !== null && run.execution.selected_total > 0}
				{@const total = run.execution.selected_total}
				{@const progressed = stage === 'committing' ? run.counts.imported : run.execution.processed}
				<progress value={progressed} max={total} aria-label={stage === 'committing' ? 'Resources added' : 'Resources processed'}></progress>
				<p class="run-bar">
					{progressed} of {total} selected resources {stage === 'committing' ? 'added' : 'processed'}
					({Math.round(progressed / total * 100)}%).
				</p>
			{/if}

			{#if stage === 'done' || stage === 'failed'}
				<p class="run-bar">{settledLine(run.counts)}</p>
				<div class="actions">
					<Button tier="primary" icon="layout-list" href={importedHref(run.source)}>
						Open them in Resources
					</Button>
					{#if stage === 'failed' && run.source !== null}
						<Button href={`/import?source=${encodeURIComponent(run.source)}&retry=${encodeURIComponent(run.id)}`}>Start a new attempt</Button>
					{/if}
				</div>
			{:else}
				<div class="actions">
					{#if (stage === 'waiting' || stage === 'interrupted') && invoke !== null}
						<Button tier="primary" disabled={sending} onclick={() => void resume()}>Resume on this device</Button>
					{/if}
					<Button
						danger
						disabled={sending}
						reason={sending ? 'An answer is on its way.' : undefined}
						onclick={() => void abandon()}
					>
						Stop this import
					</Button>
				</div>
			{/if}
		</Panel>

		{#if stage === 'selecting'}
			<Panel
				title="Choose what to bring across"
				description="Only what you tick is opened and read. Everything else is left where it is."
			>
				<div class="run-picks">
					<Button small onclick={selectAll} disabled={offered.length === 0} reason={offered.length === 0 ? 'Nothing has been listed yet.' : undefined}>
						Select all
					</Button>
					<Button small tier="quiet" onclick={() => (chosen = new Set())}>Clear</Button>
					<span class="run-chosen">{chosenCount} of {offered.length} chosen</span>
				</div>

				<ul class="run-list">
					{#each offered as item (item.locator)}
						<li>
							<label class="run-pick">
								<input
									type="checkbox"
									checked={chosen.has(item.locator)}
									onchange={() => toggle(item.locator)}
								/>
								<span class="t">{item.name}</span>
								<span class="p">{item.price ?? '—'}</span>
								<StatusPill tone={item.tone} label={item.label} />
							</label>
						</li>
					{/each}
				</ul>

				{@const blocked = selectionBlocked(chosenCount, sending)}
				<div class="actions">
					<Button
						tier="primary"
						icon="download"
						disabled={blocked !== null}
						reason={blocked ?? undefined}
						onclick={() => void continueHere()}
					>
						{sending ? 'Sending your choice…' : 'Continue'}
					</Button>
				</div>
				<p class="foot-note">{READING_HAPPENS_ON_YOUR_COMPUTER}</p>
			</Panel>
		{/if}

		{#if run.review_pairs.length > 0}
			<ReviewCards
				pairs={run.review_pairs}
				onsame={(lo, hi, keep, fields) => void decide(lo, hi, merged(keep, fields))}
				ondifferent={(lo, hi) => void decide(lo, hi, { verdict: 'different' })}
				onlater={(lo, hi) => void decide(lo, hi, { verdict: 'parked' })}
			/>
		{/if}

		{#if stage === 'reviewing' || stage === 'confirming' || stage === 'committing'}
			<Panel
				title="Add them to your catalogue"
				description="Everything that is ready is created here. Anything you have left for later waits."
			>
				{#if commitRefusal !== null}
					<Banner tone="bad" title="That did not finish">{commitRefusal}</Banner>
				{/if}
				{@const ready = run.counts.matched}
				<div class="actions">
					<Button
						tier="primary"
						icon="circle-plus"
						disabled={committing || run.execution.commit_authorised || ready === 0}
						reason={run.execution.commit_authorised
							? 'Already confirmed. The server will continue when any remaining questions are answered.'
							: committing ? 'Sending your confirmation.'
								: ready === 0 ? 'Answer the pairs above before adding these resources.' : undefined}
						onclick={() => void commit()}
					>
						{run.execution.commit_authorised ? 'Confirmed' : committing ? 'Confirming…' : 'Add to catalogue'}
					</Button>
				</div>
			</Panel>
		{/if}

		<Panel
			title="Resources"
			description="Each resource this import reached, in the order your shop listed them."
		>
			{#if rows.length === 0}
				<p class="quiet">{emptyItemsLine(stage)}</p>
			{/if}
			{#each rows as row (row.locator)}
				<div class="import-listing">
					<span class="mark"><StatusPill tone={row.tone} label={row.label} /></span>
					<span class="what">
						<span class="t">{row.name}</span>
						{#if row.reason !== ''}
							<span class="w">{row.reason}</span>
						{/if}
					</span>
					{#if row.price !== null}
						<span class="run-price">{row.price}</span>
					{/if}
					{#if row.product !== null}
						<a class="run-open" href={`/resources/${row.product}`}>Open</a>
					{/if}
					<span class="ord">#{row.ordinal}</span>
				</div>
			{/each}
			<p class="foot-note">{FILES_STAY_ON_YOUR_COMPUTER}</p>
		</Panel>
	{:else if refusal !== null}
		<PageHead
			icon="download"
			back={{ href: '/import', label: 'Back to Import' }}
			title="Import"
			description="We could not read this import."
		/>
		<Panel>
			<Placeholder icon="download" headline="We could not read this import" body={refusal}>
				{#snippet actions()}
					<Button href="/import">Back to Import</Button>
				{/snippet}
			</Placeholder>
		</Panel>
	{:else}
		<p class="quiet">Loading the import…</p>
	{/if}
</div>
