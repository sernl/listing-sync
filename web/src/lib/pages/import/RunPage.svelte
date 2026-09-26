<script lang="ts">
	import { untrack } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import {
		ApiFailure,
		api,
		type DuplicateDecision,
		type ImportRunItemOrder,
		type ImportRunItemView,
		type ImportRunView,
		type PairFieldChoice,
		type PairSide
	} from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import {
		DEVICE_SIGNED_OUT,
		continueImportHere,
		desktopInvoker,
		startImportHere,
		stopImportHere
	} from '$lib/desktop';
	import Field from '$lib/Field.svelte';
	import { createLedger, type Ledger } from '$lib/ledger';
	import { machineHere } from '$lib/machine.svelte';
	import { SIGN_BACK_IN, signedOutHere } from '$lib/machine-here';
	import Note from '$lib/Note.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Pagination from '$lib/Pagination.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { FILES_STAY_ON_YOUR_COMPUTER } from '$lib/sync-request';
	import {
		deleteRefusal,
		retainedBadge,
		type WorkDeleteOutcome,
		type WorkItem
	} from '$lib/work-delete';
	import WorkDeleteDialog from '$lib/WorkDeleteDialog.svelte';
	import { NEEDS_THE_APP, startRefusal } from './import-view';
	import ReviewCards from './ReviewCards.svelte';
	import {
		NO_ITEM_MATCHES,
		READING_HAPPENS_ON_YOUR_COMPUTER,
		RUN_UNREAD,
		countsLine,
		deviceConditionIsCurrent,
		emptyItemsLine,
		importedHref,
		itemRows,
		pageCount,
		pageSummary,
		reasonLine,
		runBadge,
		runName,
		selectionBlocked,
		selectionRows,
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
	// not have their choices rewritten by a refetch the ledger triggered —
	// and because the items are now one page of the run, so a selection
	// derived from them would lose every resource the seller ticked on a page
	// they have since left.
	let chosen = $state<Set<string>>(new Set());
	// The seller's separate, explicit "every resource in this import". The
	// only thing that sends `{ all: true }`. It is never inferred from a
	// ticked page: with twenty-five rows on screen, "I ticked all of these"
	// and "I want the whole shop" are different sentences, and reading the
	// first as the second imported shops nobody asked for.
	let wholeRun = $state(false);
	let sending = $state(false);
	let declined = $state<string | null>(null);
	// Latched when this computer answers that it has been signed out, so the
	// impossible action is withdrawn rather than offered again. The pre-press
	// fact is `machineHere.revoked`; this covers the window between a
	// revocation and this console's next check-in.
	let signedOut = $state(false);

	let committing = $state(false);
	let commitRefusal = $state<string | null>(null);
	let stopPending = $state(false);

	/** How many resources one page of the list holds. The server's own
	 *  default, restated so the pager can count pages without a round trip. */
	const PER_PAGE = 25;

	// The window on the run's resources, in two parts, and the split is the
	// point. `itemPage` and the filters are what the seller has *asked* for
	// and what the controls show; `shownPage` is the window the rows on
	// screen actually came from. A read that failed leaves the old rows up,
	// and they must not be labelled with a page they are not from.
	let itemPage = $state(1);
	let search = $state('');
	let searching = $state('');
	let itemState = $state<ImportRunItemView['state'] | ''>('');
	let itemOrder = $state<ImportRunItemOrder>('title');
	let itemsBusy = $state(false);
	let shownPage = $state(1);
	// The window a failed read was for, so Retry asks for that one again
	// rather than for whatever the controls have drifted to.
	let failedFor = $state<string | null>(null);
	let debounce: ReturnType<typeof setTimeout> | null = null;

	function cancelDebounce() {
		if (debounce !== null) {
			clearTimeout(debounce);
			debounce = null;
		}
	}

	const stage = $derived(view === null ? null : stageFrom(view));
	const rows = $derived(view === null ? [] : itemRows(view.items));
	// The resources on this page that are still a choice. Not the whole run:
	// the run's own figure is `counts.listed`, which is what the summary and
	// the whole-run control speak for.
	const offeredHere = $derived(view === null ? [] : selectionRows(view.items));
	const chosenHere = $derived(offeredHere.filter((item) => chosen.has(item.locator)).length);
	const listedTotal = $derived(view?.counts.listed ?? 0);
	const elsewhere = $derived(chosen.size - chosenHere);
	const itemsTotal = $derived(view?.items_total ?? 0);
	const pages = $derived(pageCount(itemsTotal, PER_PAGE));
	const filtered = $derived(searching !== '' || itemState !== '');

	/** This run, while a Delete is being confirmed for it, or null while none
	 *  is. The same dialog the history list uses: one import deleted from its
	 *  own page and one deleted from the list are the same act. */
	let deleting = $state<WorkItem[] | null>(null);

	const IMPORTS = { one: 'import', many: 'imports' };

	// Return accepted deletions to history, where stopping rows remain visible.
	// Re-read other outcomes: a lost response does not prove nothing changed.
	async function settledDelete(outcome: WorkDeleteOutcome) {
		if (outcome.deleted.length > 0 || outcome.stopping.length > 0) {
			deleting = null;
			await goto('/import');
			return;
		}
		await refetch();
	}

	async function refetch() {
		if (!runId) {
			return;
		}
		const id = runId;
		const asked = { page: itemPage, q: searching, state: itemState, order: itemOrder };
		const current = ++generation;
		itemsBusy = true;
		try {
			const next = await api.importRun(id, {
				offset: (asked.page - 1) * PER_PAGE,
				limit: PER_PAGE,
				q: asked.q === '' ? null : asked.q,
				state: asked.state === '' ? null : asked.state,
				order: asked.order
			});
			if (current !== generation || id !== runId) return;
			view = next;
			shownPage = asked.page;
			refusal = null;
			failedFor = null;
		} catch (caught) {
			if (current !== generation || id !== runId) return;
			// The rows already in hand stay, and `shownPage` stays with them:
			// a read that failed must not blank a list the seller is choosing
			// from, and must not relabel it as a page it never held.
			refusal = caught instanceof ApiFailure ? caught.message : RUN_UNREAD;
			failedFor = asked.q === '' ? `page ${asked.page}` : `page ${asked.page} of “${asked.q}”`;
		} finally {
			if (current === generation) itemsBusy = false;
		}
	}

	/** Move to a page, or change what the pages are cut from.
	 *
	 *  A narrowing resets to the first page, because page four of one search
	 *  is not page four of another. Nothing here touches `chosen`: the
	 *  selection is held by locator and survives every page, search and
	 *  filter, which is the whole point of holding it by locator. */
	function show(next: number) {
		itemPage = Math.min(Math.max(1, next), pages);
		void refetch();
	}

	function narrow() {
		itemPage = 1;
		void refetch();
	}

	function typed(value: string) {
		search = value;
		cancelDebounce();
		// Fenced on the run: a timer that fires after the seller has left, or
		// after they cleared the search, must not put a stale term back or
		// throw the list to page one behind them.
		const epoch = runEpoch;
		const asked = value;
		debounce = setTimeout(() => {
			debounce = null;
			if (epoch !== runEpoch || asked !== search) return;
			searching = search.trim();
			narrow();
		}, 250);
	}

	function clearFilters() {
		cancelDebounce();
		search = '';
		searching = '';
		itemState = '';
		itemOrder = 'title';
		narrow();
	}

	// The same liveness the request page beside this one uses: one event
	// stream per tab, and a refetch when the ledger moves or resyncs. The
	// payloads are never read — the snapshot is what the page renders, so a
	// projection here would be a second copy of the server's own counting.
	//
	// Everything below the first line runs untracked, and that is load-bearing:
	// `refetch` reads the page, the search and the filters, so a tracked body
	// would make this effect a dependent of them — and then changing a page
	// would re-run the initialisation that clears the selection and resets to
	// page one. The run is the only thing this effect is about.
	$effect(() => {
		const id = runId;
		return untrack(() => {
			void id;
			runEpoch += 1;
			view = null;
			refusal = null;
			chosen = new Set();
			wholeRun = false;
			sending = false;
			committing = false;
			declined = null;
			signedOut = false;
			commitRefusal = null;
			stopPending = false;
			// A different run is a different list: its pages, its search and its
			// filters all start again. This is the only place the selection is
			// cleared, which is what lets it survive every page change below.
			cancelDebounce();
			itemPage = 1;
			shownPage = 1;
			failedFor = null;
			search = '';
			searching = '';
			itemState = '';
			itemOrder = 'title';
			ledger = createLedger((cursor) => new EventSource(`/v1/events/stream?cursor=${cursor}`));
			void refetch();
			let revision = 0;
			const unsubscribe = ledger.subscribe((state) => {
				live = state.connected;
				if (state.revision === revision) return;
				revision = state.revision;
				if ([...state.kinds].some((kind) => kind === 'resync' || kind.startsWith('ImportRun'))) {
					// Re-reads the page the seller is on, under the filters they set,
					// and keeps their ticks. A background refresh that jumped to page
					// one or appended a fresh page would move the list out from under
					// someone in the middle of choosing.
					void refetch();
				}
			});
			return () => {
				runEpoch += 1;
				generation += 1;
				cancelDebounce();
				unsubscribe();
				ledger?.close();
			};
		});
	});

	// The latch is cleared by a restoration, never by the absence of one.
	//
	// `machineHere.revoked` is only ever true when a check-in reached the
	// server, so the edge from true to false is an authoritative "this
	// machine is signed back in" — the seller's own explicit act on the
	// Machines row. Reading the flag as false on its own would clear the
	// latch every time this effect ran, including in the window between a
	// revocation and the next check-in, which is exactly the window the latch
	// exists to cover.
	let wasRevoked = false;
	$effect(() => {
		const revoked = machineHere.revoked;
		untrack(() => {
			if (wasRevoked && !revoked) signedOut = false;
			wasRevoked = revoked;
		});
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

	/** Add this page's remaining choices to the selection.
	 *
	 *  Adds; never replaces. The seller has ticked things on other pages and
	 *  this control is about the rows in front of them. */
	function selectPage() {
		const next = new Set(chosen);
		for (const item of offeredHere) {
			next.add(item.locator);
		}
		chosen = next;
	}

	function clearSelection() {
		chosen = new Set();
		wholeRun = false;
	}

	/** The server freezes the selected rows before the device begins reading.
	 *
	 *  Two spellings, and the page never guesses which one the seller meant:
	 *  `{ all: true }` goes only when they pressed the whole-import control,
	 *  and every other press sends the locators they actually ticked. The
	 *  shortcut this replaced — "as many ticked as offered, so send all" —
	 *  was reading a full page as a full shop. */
	async function continueHere() {
		if (sending || (!wholeRun && chosen.size === 0)) return;
		const target = { id: runId, epoch: runEpoch };
		const selection = wholeRun ? { all: true as const } : { locators: [...chosen] };
		sending = true;
		declined = null;
		try {
			await api.selectImportRun(target.id, selection);
			// The answer carries the run with its own first page of items, which
			// is not the page or the filter the seller is standing on. The
			// refetch below reads their window instead, so the list they are
			// looking at never jumps under them.
			const outcome = await continueImportHere(invoke, target.id);
			if (current(target)) {
				declined = startRefusal(outcome) ?? (outcome.kind === 'unavailable' ? NEEDS_THE_APP : null);
				if (declined === DEVICE_SIGNED_OUT) signedOut = true;
			}
		} catch (caught) {
			if (current(target)) {
				declined = caught instanceof ApiFailure
					? caught.message : 'We did not hear back about your choice. Check this import before you try again.';
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
			await api.confirmImportRun(target.id);
		} catch (caught) {
			if (current(target)) {
				commitRefusal = caught instanceof ApiFailure
					? caught.message : 'We did not hear back. Check this import before you try again.';
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
				commitRefusal = caught instanceof ApiFailure ? caught.message : 'We did not hear back about that answer.';
			}
		}
		if (current(target)) await refetch();
	}

	async function resume() {
		if (sending || view === null) return;
		const target = { id: runId, epoch: runEpoch };
		const enumerated = view.execution.enumeration_complete;
		if (!window.confirm('Resume this import on this device? It will stop on any other device.')) return;
		sending = true;
		declined = null;
		try {
			const outcome = enumerated
				? await continueImportHere(invoke, target.id, true)
				: await startImportHere(invoke, target.id, true);
			if (current(target)) {
				if (outcome.kind === 'refused') {
					declined = outcome.detail;
					// A computer that has been signed out cannot run this import,
					// and pressing again cannot change that. The control is
					// withdrawn rather than left to fail a second time.
					if (outcome.detail === DEVICE_SIGNED_OUT) signedOut = true;
				}
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
					? 'Stopped on this device. We are still confirming it.'
					: caught instanceof ApiFailure ? caught.message : 'We did not hear back about stopping.';
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
				{@const going = retainedBadge(run.deletion_status)}
				<StatusPill tone={going?.tone ?? badge.tone} label={going?.label ?? badge.label} />
				<StatusPill tone={live ? 'ok' : 'soon'} label={live ? 'Live updates on' : 'Reconnecting'} />
			{/snippet}
		</PageHead>

		{#if refusal !== null}
			<Banner tone="bad" title="We could not load this import just now">
				{refusal}
				{failedFor === null
					? 'Below is what we last loaded.'
					: `We could not load ${failedFor}. Below are the resources we last loaded.`}
				{#snippet action()}
					<Button tier="outline" small disabled={itemsBusy} reason={itemsBusy ? 'Loading.' : undefined} onclick={() => void refetch()}>
						Try again
					</Button>
				{/snippet}
			</Banner>
		{/if}
		{#if declined !== null}
			<Banner tone={stopPending ? 'info' : 'bad'} title={stopPending ? 'Stopped on this device' : 'This device did not carry on'}>{declined}</Banner>
		{/if}

		<Panel title="How this import is going">
			<p class="import-stage">{copy.headline}</p>
			<p class="quiet">{copy.detail}</p>

			<p class="quiet">{countsLine(run.counts, run.read_total)}</p>
			{@const why = deviceConditionIsCurrent(run) ? reasonLine(run.execution.reason_code, run.execution.reason) : null}
			{#if why !== null}
				<p class="run-bar">{why}</p>
			{/if}
			{#if machineHere.revoked}
				<Banner tone="warn" title="This computer is signed out">
					{signedOutHere(null)}
					{#snippet action()}
						<Button tier="outline" small href="/marketplaces">{SIGN_BACK_IN}</Button>
					{/snippet}
				</Banner>
			{/if}

			<!-- Identifiers, attempts and timestamps are diagnostics, not the
			     answer to "what is happening": they sit behind a disclosure so
			     the sentences above are what the page says. -->
			{#if run.execution.owner_device !== null || run.execution.last_contact_at !== null || run.execution.last_progress_at !== null || run.execution.reason_code !== null}
				<details class="run-tech">
					<summary>Technical details</summary>
					{#if !deviceConditionIsCurrent(run) && run.execution.reason_code !== null}
						<p class="quiet">Earlier device issue: {run.execution.reason_code}.</p>
					{/if}
					{#if run.execution.owner_device !== null}
						<p class="quiet">Importing on: {run.execution.owner_device} · try {run.execution.attempt}.</p>
					{/if}
					{#if run.execution.last_contact_at !== null}
						<p class="quiet">Last heard from the device: {new Date(run.execution.last_contact_at).toLocaleString('en-GB')}.</p>
					{/if}
					{#if run.execution.last_progress_at !== null}
						<p class="quiet">Last progress: {new Date(run.execution.last_progress_at).toLocaleString('en-GB')}.</p>
					{/if}
				</details>
			{/if}
			{#if (stage === 'reading' || stage === 'committing') && run.execution.selected_total !== null && run.execution.selected_total > 0}
				{@const total = run.execution.selected_total}
				{@const progressed = stage === 'committing' ? run.counts.imported : run.execution.processed}
				<progress value={progressed} max={total} aria-label={stage === 'committing' ? 'Resources added' : 'Resources read'}></progress>
				<p class="run-bar">
					{progressed} of {total} selected resources {stage === 'committing' ? 'added' : 'read'}
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
						<Button href={`/import?source=${encodeURIComponent(run.source)}&retry=${encodeURIComponent(run.id)}`}>Try again</Button>
					{/if}
					{@render deleteRun(run)}
				</div>
			{:else}
				<div class="actions">
					<!-- Resume is offered only where it could work. A computer the
					     seller signed out cannot take this run, so the control is
					     absent rather than disabled-with-an-excuse or, worse,
					     offered and refused on press. -->
					{#if (stage === 'waiting' || stage === 'interrupted') && invoke !== null && !machineHere.revoked && !signedOut}
						<Button tier="primary" disabled={sending} onclick={() => void resume()}>Resume on this device</Button>
					{/if}
					<!-- Stop and Delete are different acts and both are offered.
					     Stop leaves the import in the history to be read or
					     resumed; Delete takes the record away once new work has
					     been fenced. Neither touches what was already imported. -->
					<Button
						danger
						disabled={sending}
						reason={sending ? 'An answer is on its way.' : undefined}
						onclick={() => void abandon()}
					>
						Stop this import
					</Button>
					{@render deleteRun(run)}
				</div>
			{/if}
		</Panel>

		{#if stage === 'selecting'}
			<Panel title="Choose what to import">
				{@render filterRow('Search resources')}

				{#if wholeRun}
					<Banner tone="info" title="All resources selected">
						All {listedTotal} resources in this shop will be imported.
						{#snippet action()}
							<Button tier="outline" small onclick={() => (wholeRun = false)}>
								Choose individually instead
							</Button>
						{/snippet}
					</Banner>
				{:else}
					<div class="run-picks">
						<Button
							small
							onclick={selectPage}
							disabled={offeredHere.length === 0}
							reason={offeredHere.length === 0 ? 'Nothing on this page can be chosen.' : undefined}
						>
							Select this page
						</Button>
						<Button
							small
							tier="quiet"
							onclick={() => (wholeRun = true)}
							disabled={listedTotal === 0}
							reason={listedTotal === 0 ? 'No resources found yet.' : undefined}
						>
							Select all resources in this import
						</Button>
						<Button small tier="quiet" onclick={clearSelection} disabled={chosen.size === 0} reason={chosen.size === 0 ? 'Nothing is chosen yet.' : undefined}>
							Clear selection
						</Button>
						<span class="run-chosen" role="status" aria-live="polite">
							{chosen.size} selected{elsewhere > 0 ? `, ${elsewhere} of them not shown here` : ''}
						</span>
					</div>
				{/if}

				{#if rows.length === 0}
					<p class="quiet">{filtered ? NO_ITEM_MATCHES : emptyItemsLine(stage)}</p>
				{/if}
				<ul class="run-list">
					{#each rows as item (item.locator)}
						<li>
							<label class="run-pick">
								<input
									type="checkbox"
									checked={wholeRun || chosen.has(item.locator)}
									disabled={wholeRun || item.state !== 'listed'}
									onchange={() => toggle(item.locator)}
								/>
								{@render cover(item.coverUrl, item.name)}
								<span class="t">{item.name}</span>
								<span class="p">{item.price ?? '—'}</span>
								<StatusPill tone={item.tone} label={item.label} />
							</label>
						</li>
					{/each}
				</ul>

				{@render pager('Resources to choose from')}

				{@const blocked = wholeRun
					? sending ? 'Sending your choice.' : null
					: selectionBlocked(chosen.size, sending)}
				<div class="actions">
					<Button
						tier="primary"
						icon="download"
						disabled={blocked !== null}
						reason={blocked ?? undefined}
						onclick={() => void continueHere()}
					>
						{sending
							? 'Sending your choice…'
							: wholeRun
								? `Import all ${listedTotal} resources`
								: `Import ${chosen.size} ${chosen.size === 1 ? 'resource' : 'resources'}`}
					</Button>
				</div>
				<Note icon="lock">{READING_HAPPENS_ON_YOUR_COMPUTER}</Note>
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
				title="Add them to Resources"
				description="Every resource that is ready gets added."
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
							? 'Already confirmed. We carry on once you answer any questions left.'
							: committing ? 'Sending your confirmation.'
								: ready === 0 ? 'Answer the pairs above first.' : undefined}
						onclick={() => void commit()}
					>
						{run.execution.commit_authorised ? 'Confirmed' : committing ? 'Confirming…' : 'Add to Resources'}
					</Button>
				</div>
			</Panel>
		{/if}

		<!-- While the seller is choosing, the tick list above is this list:
		     every resource is still listed, so drawing it twice would be two
		     pagers over one set of rows. -->
		{#if stage !== 'selecting'}
			<Panel
				title="Resources"
				description="Every resource in this import."
			>
				{@render filterRow('Search resources')}
				{#if rows.length === 0}
					<p class="quiet">{filtered ? NO_ITEM_MATCHES : emptyItemsLine(stage)}</p>
				{/if}
				{#each rows as row (row.locator)}
					<div class="import-listing">
						<span class="mark"><StatusPill tone={row.tone} label={row.label} /></span>
						{@render cover(row.coverUrl, row.name)}
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
				{@render pager('Resources')}
				<Note icon="lock">{FILES_STAY_ON_YOUR_COMPUTER}</Note>
			</Panel>
		{/if}
	{:else if refusal !== null}
		<PageHead
			icon="download"
			back={{ href: '/import', label: 'Back to Import' }}
			title="Import"
			description="We could not load this import."
		/>
		<Panel>
			<Placeholder icon="download" headline="We could not load this import" body={refusal}>
				{#snippet actions()}
					<Button href="/import">Back to Import</Button>
				{/snippet}
			</Placeholder>
		</Panel>
	{:else}
		<p class="quiet">Loading the import…</p>
	{/if}
</div>

<!-- One filter row and one pager, rendered by both lists, so the tick list
     and the resource list are the same list with the same controls. -->
{#snippet filterRow(label: string)}
	<div class="import-filters">
		<div class="wide">
			<Field label={label} id="run-search" hint="Title or web address.">
				<input
					id="run-search"
					type="search"
					value={search}
					placeholder="Search by title"
					oninput={(event) => typed(event.currentTarget.value)}
				/>
			</Field>
		</div>
		<Field label="Status" id="run-state">
			<select id="run-state" bind:value={itemState} onchange={narrow}>
				<option value="">Any status</option>
				<option value="listed">Found</option>
				<option value="selected">Chosen</option>
				<option value="read">Read</option>
				<option value="matched">Ready</option>
				<option value="review">Needs you</option>
				<option value="imported">In Resources</option>
				<option value="skipped">Left out</option>
				<option value="failed">Problem</option>
			</select>
		</Field>
		<Field label="Order by" id="run-order">
			<select id="run-order" bind:value={itemOrder} onchange={narrow}>
				<option value="title">Title A–Z</option>
				<option value="listed">Shop order</option>
			</select>
		</Field>
		{#if filtered || itemOrder !== 'title'}
			<Button small tier="quiet" onclick={clearFilters}>Clear filters</Button>
		{/if}
	</div>
{/snippet}

{#snippet pager(label: string)}
	<Pagination
		page={shownPage}
		hasNext={shownPage < pages}
		busy={itemsBusy}
		label={label}
		summary={`${pageSummary(view?.items_offset ?? 0, rows.length, itemsTotal, 'resources')} · Page ${shownPage} of ${pages}`}
		onprevious={() => show(shownPage - 1)}
		onnext={() => show(shownPage + 1)}
	/>
{/snippet}

<!-- A real cover where the read produced one, and a neutral slot where it
     did not. The slot is also what a broken image collapses to, so a cover
     that fails to load is an empty frame rather than a torn icon — and never
     a claim that the marketplace supplied nothing. -->
{#snippet cover(url: string | null, name: string)}
	{#if url !== null}
		<img
			class="run-cover"
			src={url}
			alt={`Cover of ${name}`}
			width="64"
			height="48"
			loading="lazy"
			onerror={(event) => event.currentTarget.classList.add('gone')}
		/>
	{:else}
		<span class="run-cover none" aria-hidden="true"></span>
	{/if}
{/snippet}

<!-- Delete, offered wherever the run's own actions are. Withdrawn as a
     press once the run is already on its way out: the server would accept
     the call and nothing the seller can see would change, which reads as a
     control that does nothing. -->
{#snippet deleteRun(run: ImportRunView)}
	{@const refusal = deleteRefusal(run.deletion_status)}
	<Button
		danger
		disabled={refusal !== null}
		reason={refusal ?? undefined}
		onclick={() => (deleting = [{ id: run.id, label: runName(run) }])}
	>
		Delete this import
	</Button>
{/snippet}

<WorkDeleteDialog
	open={deleting !== null}
	items={deleting ?? []}
	noun={IMPORTS}
	remove={api.deleteImportRun}
	onClose={() => (deleting = null)}
	onsettled={(outcome) => void settledDelete(outcome)}
/>
