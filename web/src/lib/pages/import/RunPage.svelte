<script lang="ts">
	import { page } from '$app/state';
	import {
		ApiFailure,
		api,
		type DuplicateDecision,
		type ImportRunView,
		type PairFieldChoice,
		type PairSide,
		type RunCommitAck
	} from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { continueImportHere, desktopInvoker } from '$lib/desktop';
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
		inPlay,
		itemRows,
		progressLine,
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

	// Read once: whether this console runs inside the desktop application does
	// not change while the page is open.
	const invoke = desktopInvoker();

	// The tick list's own state, keyed by locator. Held here rather than
	// derived from the items, because a seller half-way through ticking must
	// not have their choices rewritten by a refetch the ledger triggered.
	let chosen = $state<Set<string>>(new Set());
	let sending = $state(false);
	let declined = $state<string | null>(null);

	// The commit loop, in the shape the spreadsheet commit established: one
	// chunk per call, so a closed browser loses only the chunk in flight.
	let committing = $state(false);
	let ack = $state<RunCommitAck | null>(null);
	let commitRefusal = $state<string | null>(null);

	const stage = $derived(view === null ? null : stageFrom(view));
	const rows = $derived(view === null ? [] : itemRows(view));
	const offered = $derived(view === null ? [] : selectionRows(view));
	const chosenCount = $derived(offered.filter((item) => chosen.has(item.locator)).length);

	async function refetch() {
		if (!runId) {
			return;
		}
		try {
			view = await api.importRun(runId);
			refusal = null;
		} catch (caught) {
			refusal = caught instanceof ApiFailure ? caught.message : RUN_UNREAD;
		}
	}

	// The same liveness the request page beside this one uses: one event
	// stream per tab, and a refetch when the ledger moves or resyncs. The
	// payloads are never read — the snapshot is what the page renders, so a
	// projection here would be a second copy of the server's own counting.
	$effect(() => {
		void refetch();
		ledger = createLedger((cursor) => new EventSource(`/v1/events/stream?cursor=${cursor}`));
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

	/** Send the tick list, then ask this computer to read what was ticked.
	 *
	 *  Whether every resource was ticked is sent as `all` rather than as every
	 *  locator: a shop that grew between the enumeration and the tick is still
	 *  "all of it", and the server is the side that knows what all of it is
	 *  now. */
	async function continueHere() {
		if (sending || chosenCount === 0) {
			return;
		}
		sending = true;
		declined = null;
		try {
			const everything = chosenCount === offered.length;
			view = await api.selectImportRun(
				runId,
				everything ? { all: true } : { locators: [...chosen] }
			);
			const outcome = await continueImportHere(invoke, runId);
			declined = startRefusal(outcome) ?? (outcome.kind === 'unavailable' ? NEEDS_THE_APP : null);
		} catch (caught) {
			declined =
				caught instanceof ApiFailure
					? caught.message
					: 'Your choice did not reach us. Nothing has been read.';
		} finally {
			sending = false;
			await refetch();
		}
	}

	/** The chunk loop. Each call is one chunk, and the loop stops the moment
	 *  the server says it is complete, so nothing here decides when a commit
	 *  is finished. */
	async function commit() {
		if (committing) {
			return;
		}
		committing = true;
		commitRefusal = null;
		try {
			for (;;) {
				const next = await api.commitImportRun(runId);
				ack = next;
				if (next.complete) {
					break;
				}
			}
		} catch (caught) {
			commitRefusal =
				caught instanceof ApiFailure
					? caught.message
					: 'That chunk did not reach us. Nothing more was added.';
		} finally {
			committing = false;
			await refetch();
		}
	}

	async function decide(lo: string, hi: string, decision: DuplicateDecision) {
		try {
			await api.decideDuplicate(lo, hi, decision);
			commitRefusal = null;
		} catch (caught) {
			commitRefusal =
				caught instanceof ApiFailure ? caught.message : 'That answer did not reach us.';
		}
		await refetch();
	}

	async function abandon() {
		if (sending) {
			return;
		}
		sending = true;
		try {
			view = await api.abandonImportRun(runId);
		} catch (caught) {
			declined =
				caught instanceof ApiFailure ? caught.message : 'That did not reach us.';
		} finally {
			sending = false;
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
		{@const badge = runBadge(run.state)}
		{@const copy = stageCopy(stage)}
		<PageHead
			icon="download"
			back={{ href: '/import', label: 'Back to Import' }}
			title={run.source === null ? 'Import' : `Import from ${runName(run)}`}
			description={`Started ${new Date(run.created_at).toLocaleString('en-GB')}`}
		>
			{#snippet aside()}
				<StatusPill tone={badge.tone} label={badge.label} />
				<StatusPill tone={live ? 'ok' : 'soon'} label={live ? 'Live' : 'Reconnecting'} />
			{/snippet}
		</PageHead>

		{#if refusal !== null}
			<Banner tone="bad" title="We could not read this import just now">
				{refusal} Below is the last state we read.
			</Banner>
		{/if}
		{#if declined !== null}
			<Banner tone="bad" title="This computer did not carry on">{declined}</Banner>
		{/if}

		<Panel title="Where this import stands">
			<p class="import-stage">{copy.headline}</p>
			<p class="quiet">{copy.detail}</p>

			{#if stage === 'reading' || stage === 'reviewing' || stage === 'committing'}
				<p class="run-bar">{progressLine(run.counts, inPlay(run.counts))}</p>
			{/if}

			{#if stage === 'done' || stage === 'failed'}
				<p class="run-bar">{settledLine(run.counts)}</p>
				<div class="actions">
					<Button tier="primary" icon="layout-list" href={importedHref(run.source)}>
						Open them in Resources
					</Button>
				</div>
			{:else}
				<div class="actions">
					<Button
						danger
						disabled={sending}
						reason={sending ? 'An answer is on its way.' : undefined}
						onclick={() => void abandon()}
					>
						Give up on this import
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

		{#if stage === 'reviewing' || stage === 'committing'}
			<Panel
				title="Add them to your catalogue"
				description="Everything that is ready is created here. Anything you have left for later waits."
			>
				{#if commitRefusal !== null}
					<Banner tone="bad" title="That did not finish">{commitRefusal}</Banner>
				{/if}
				{#if ack !== null}
					<p class="quiet">
						{ack.applied} added, {ack.remaining} to go.
					</p>
				{/if}
				{@const ready = run.counts.matched}
				<div class="actions">
					<Button
						tier="primary"
						icon="circle-plus"
						disabled={committing || ready === 0}
						reason={committing
							? 'Adding them now.'
							: ready === 0
								? 'Answer the pairs above and these are ready to add.'
								: undefined}
						onclick={() => void commit()}
					>
						{committing ? 'Adding…' : 'Add to catalogue'}
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
