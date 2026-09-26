<script lang="ts">
	import { page } from '$app/state';
	import {
		ApiFailure,
		api,
		type BatchStateView,
		type CommitAck,
		type DuplicateDecision,
		type ImportBatchDetailView,
		type ImportRowView,
		type RowStateView
	} from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { utcInstant } from '$lib/elapsed';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import FlowActionBar from '$lib/FlowActionBar.svelte';
	import FlowStep from '$lib/FlowStep.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Pagination from '$lib/Pagination.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import Stepper, { type StepMark } from '$lib/Stepper.svelte';
	import { tick } from 'svelte';
	import AttachPanel from './AttachPanel.svelte';
	import ReviewCards from './ReviewCards.svelte';
	import {
		AWAITING_TITLE,
		BATCH_UNREAD,
		COMMIT_STALLED,
		NOTHING_SENT,
		REFUSED_WITHOUT_REASON,
		type AwaitingFiles,
		awaitingFrom,
		awaitingSay,
		batchClosedSay,
		presentStage,
		previewSentence,
		progressFrom,
		progressLine,
		reportRows,
		stageOf,
		tallyRows,
		warningLine
	} from './sheet-view';
	import { pageCount, pageSummary } from './run-view';
	import '$lib/flow.css';
	import './import.css';
	import './sheet.css';

	const batchId = $derived(page.params.batch ?? '');

	let detail = $state<ImportBatchDetailView | null>(null);
	// Three flags rather than one, deliberately. A read that failed is not an
	// import that is gone, and neither is an import with nothing in it: a
	// seller following a link to a batch the sweep cleared is told it is gone,
	// and one whose read broke is told nothing was changed.
	let unread = $state(false);
	let gone = $state(false);

	let refusal = $state<string | null>(null);
	// Whether pressing again could get further. A refusal the seller must act
	// on first carries no Retry, because a control that cannot work is worse
	// than none.
	let resumable = $state(false);
	// The rows the commit refused for want of bytes, as the server named them.
	// Its own state rather than a sentence in `refusal`, because this one
	// carries the way out of it.
	let missingFiles = $state<AwaitingFiles | null>(null);
	let confirming = $state(false);
	let adding = $state(false);
	let running = $state(false);
	let settling = $state(false);
	let ack = $state<CommitAck | null>(null);

	const stage = $derived(detail === null ? null : stageOf(detail));
	const shown = $derived(stage === null ? null : presentStage(stage));
	const rows = $derived(detail === null ? [] : detail.rows);
	const report = $derived(reportRows(rows));
	const tally = $derived(tallyRows(rows));
	const progress = $derived(progressFrom(ack));

	/** How many report rows are drawn at once.
	 *
	 *  The snapshot itself stays whole, and deliberately: `tallyRows`,
	 *  `previewOf`, `awaitingRows` and the filename matcher are all partitions
	 *  over every row of the batch, and fed a page they would state a figure
	 *  about the seller's catalogue that is not true. So this pages what is
	 *  drawn and nothing else — the counts above the list still speak for all
	 *  five hundred rows a sheet may carry. */
	const PER_PAGE = 25;

	let reportPage = $state(1);
	let reportSearch = $state('');
	let reportState = $state<RowStateView | ''>('');

	const matching = $derived(
		report.filter((entry) => {
			if (reportState !== '' && entry.state !== reportState) {
				return false;
			}
			const term = reportSearch.trim().toLowerCase();
			if (term === '') {
				return true;
			}
			return [entry.name, entry.fileName, entry.label].some(
				(field) => field !== null && field.toLowerCase().includes(term)
			);
		})
	);
	const reportPages = $derived(pageCount(matching.length, PER_PAGE));
	const reportAt = $derived(Math.min(reportPage, reportPages));
	const reportSlice = $derived(
		matching.slice((reportAt - 1) * PER_PAGE, reportAt * PER_PAGE)
	);
	const reportFiltered = $derived(reportSearch.trim() !== '' || reportState !== '');

	/** The day an unfinished import is swept, in UTC, because that is the clock
	 *  the server computed it on. The whole instant would be precision this
	 *  sentence does not carry. */
	const expiryDay = $derived(detail === null ? null : utcInstant(detail.expires_at).slice(0, 10));

	async function refetch() {
		if (batchId === '') {
			return;
		}
		try {
			detail = await api.importBatch(batchId);
			unread = false;
			gone = false;
		} catch (failure) {
			detail = null;
			gone = failure instanceof ApiFailure && failure.status === 404;
			unread = !gone;
		}
	}

	$effect(() => {
		void refetch();
	});

	/** The server's own sentence, where it sent one worth reading. */
	function statedBy(failure: ApiFailure): string {
		const stated = failure.body?.errors?.[0]?.message;
		return typeof stated === 'string' && stated.trim() !== '' ? stated : REFUSED_WITHOUT_REASON;
	}

	function refusalOf(failure: unknown): string {
		if (!(failure instanceof ApiFailure)) {
			return 'That did not reach us. Nothing was changed.';
		}
		if (failure.status === 409) {
			// A batch that has settled says what the seller does about it in
			// this console's own words; any other 409 keeps the server's.
			return batchClosedSay(failure.body?.errors?.[0]?.detail) ?? statedBy(failure);
		}
		return statedBy(failure);
	}

	/** What a refusal said about rows still holding no file.
	 *
	 *  Only a 409 carries D32's gate, so no other status is read for it: a
	 *  field looked for where it is never sent is a field that is absent for
	 *  the wrong reason. */
	function missingFilesOf(failure: unknown): AwaitingFiles | null {
		if (!(failure instanceof ApiFailure) || failure.status !== 409) {
			return null;
		}
		return awaitingFrom(failure.body?.errors?.[0]?.detail);
	}

	/** Whether pressing again could get any further.
	 *
	 *  A request that never reached us and a fault on our side are both states
	 *  a second attempt can leave. A refusal the seller must act on first is
	 *  not: the batch has settled, or the rows are waiting on them. */
	function canRetry(failure: unknown): boolean {
		return !(failure instanceof ApiFailure) || failure.status === 0 || failure.status >= 500;
	}

	/** Applies one bound row in place, which is what the bind answers a row
	 *  for. The batch's own state travels with it, so the page's face follows
	 *  the first file without a second read. */
	function applyRow(bound: ImportRowView, state: BatchStateView) {
		if (detail === null) {
			return;
		}
		detail = {
			...detail,
			state,
			rows: detail.rows.map((row) =>
				row.sheet === bound.sheet && row.ordinal === bound.ordinal ? bound : row
			)
		};
		// The refused list named the rows as they stood before this file. One
		// of them may be the row just bound, so the list is dropped rather than
		// left standing as an account of rows that has moved on.
		missingFiles = null;
	}

	/** Moves the page's face to where the chunk said the batch now stands.
	 *
	 *  The acknowledgement carries the state for exactly this, so the meter
	 *  appears on the first chunk rather than after a second read. */
	function applyState(state: BatchStateView) {
		if (detail === null) {
			return;
		}
		detail = { ...detail, state };
	}

	/** Commits chunk after chunk until the server says the batch is complete.
	 *
	 *  The loop is here rather than on the server because the chunk is what
	 *  makes a closed browser lose only the work in flight; a request that ran
	 *  the whole import would have nothing to resume from.
	 *
	 *  No re-read between chunks. The bar is drawn from the acknowledgement's
	 *  own `remaining` and `total`, and the chunk states where the batch now
	 *  stands, so a second read would be asking for what was just answered —
	 *  and a read that failed under a commit still running would blank the page
	 *  the seller is watching it on. */
	async function commit() {
		if (running) {
			return;
		}
		running = true;
		confirming = false;
		refusal = null;
		resumable = false;
		missingFiles = null;
		try {
			// Null rather than a sentinel figure: no chunk has answered yet,
			// and a first chunk that leaves nothing to do is complete rather
			// than stalled, which a zero could not tell apart.
			let before: number | null = null;
			for (;;) {
				const next = await api.commitImport(batchId);
				ack = next;
				applyState(next.batch_state);
				if (next.complete) {
					break;
				}
				if (before !== null && next.remaining >= before) {
					refusal = COMMIT_STALLED;
					resumable = true;
					break;
				}
				before = next.remaining;
			}
		} catch (failure) {
			const waiting = missingFilesOf(failure);
			missingFiles = waiting;
			if (waiting === null) {
				refusal = refusalOf(failure);
				resumable = canRetry(failure);
			}
		} finally {
			running = false;
			await refetch();
		}
	}

	async function abandon() {
		if (settling) {
			return;
		}
		settling = true;
		refusal = null;
		resumable = false;
		try {
			await api.abandonImport(batchId);
		} catch (failure) {
			refusal = refusalOf(failure);
		} finally {
			settling = false;
			await refetch();
		}
	}

	/** Answer one pair the matcher parked.
	 *
	 *  The batch is re-read rather than patched in place: a verdict can free
	 *  the commit, merge two resources or skip a row, and which of those it
	 *  did is the server's to say. */
	async function decide(lo: string, hi: string, decision: DuplicateDecision) {
		try {
			await api.decideDuplicate(lo, hi, decision);
			refusal = null;
		} catch (failure) {
			refusal = refusalOf(failure);
		}
		await refetch();
	}

	// ------------------------------------------------------------ the flow

	/** Whether the batch has moved past each of Check → Attach files →
	 *  Import. */
	const steps = $derived.by((): StepMark[] => {
		const kind = stage?.kind ?? 'unrecognised';
		const early = kind === 'parsed' || kind === 'attaching';
		const awaiting = stage !== null && 'awaiting' in stage ? stage.awaiting : 0;
		return [
			{ id: 'check', label: 'Check', done: kind !== 'parsed' || adding || awaiting === 0 },
			{ id: 'attach', label: 'Attach files', done: !early || awaiting === 0 },
			{ id: 'import', label: 'Import', done: kind === 'imported' || kind === 'failed' || kind === 'abandoned' }
		];
	});

	/** Whether the sheet had more than one tab, which is when a row number
	 *  needs its tab's name beside it. */
	const manySheets = $derived(new Set(rows.map((row) => row.sheet)).size > 1);

	/** Open the attach step and bring it into view. */
	async function openAttach() {
		missingFiles = null;
		adding = true;
		await tick();
		document.getElementById('step-attach')?.scrollIntoView({ behavior: 'smooth', block: 'start' });
	}
</script>

<!-- Both actions are declared here and handed to the banner as a prop, so a
     banner with nothing to offer carries no empty action slot. -->
{#snippet retryCommit()}
	<Button
		disabled={running}
		reason={running ? 'Adding your resources.' : undefined}
		onclick={() => void commit()}
	>
		Try again
	</Button>
{/snippet}

{#snippet addFiles()}
	<Button tier="primary" icon="plus" onclick={() => void openAttach()}>Add the files</Button>
{/snippet}

<!-- The sheet's findings as a compact table: the seller's own row number,
     what the row is, where it is going, and where it stands. A row's
     problems sit under it in warn ink. -->
{#snippet reportTable(entries: ReturnType<typeof reportRows>)}
	<div class="flow-table-wrap">
		<table class="flow-table sh-table">
			<thead>
				<tr>
					<th>Row</th>
					<th>Resource</th>
					<th class="sh-where">To</th>
					<th>Status</th>
				</tr>
			</thead>
			<tbody>
				{#each entries as entry (entry.key)}
					{@const troubled = entry.problems.length > 0 || entry.failureDetail !== null}
					<tr class:troubled>
						<td class="marks sh-num" title={entry.label}>
							<span class="sh-ord">{entry.ordinal}</span>
							{#if manySheets}<span class="sh-tab">{entry.sheet}</span>{/if}
						</td>
						<td>
							{#if entry.href !== null}
								<a href={entry.href}><span class="res-name">{entry.name ?? entry.label}</span></a>
							{:else}
								<span class="res-name">{entry.name ?? entry.label}</span>
							{/if}
							{#if entry.fileName !== null}
								<span class="sh-file">{entry.fileName}{entry.attached ? ' · added' : ''}</span>
							{/if}
							{#if troubled}
								<ul class="sh-probs">
									{#each entry.problems as problem, index (`${entry.key}:${index}`)}
										<li><span class="col">{problem.column}:</span> {problem.problem}</li>
									{/each}
									{#if entry.failureDetail !== null}
										<li>{entry.failureDetail}</li>
									{/if}
								</ul>
							{/if}
						</td>
						<td class="marks sh-where">{entry.marketplace ?? '—'}</td>
						<td class="marks"><StatusPill tone={entry.tone} label={entry.stateLabel} /></td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/snippet}

<!-- The controls above the report. They narrow and page what is drawn; the
     figures, the preview sentence and the file matcher all still read every
     row of the batch. -->
{#snippet reportControls()}
	<div class="import-filters">
		<div class="wide">
			<Field label="Search rows" id="sheet-search">
				<input
					id="sheet-search"
					type="search"
					bind:value={reportSearch}
					placeholder="Title, filename or row"
					oninput={() => (reportPage = 1)}
				/>
			</Field>
		</div>
		<Field label="Status" id="sheet-state">
			<select id="sheet-state" bind:value={reportState} onchange={() => (reportPage = 1)}>
				<option value="">Any status</option>
				<option value="parsed">Ready</option>
				<option value="attached">File added</option>
				<option value="creating">Adding</option>
				<option value="created">Added</option>
				<option value="published">Published</option>
				<option value="failed">Problem</option>
				<option value="skipped">Left out</option>
			</select>
		</Field>
		{#if reportFiltered}
			<Button
				small
				tier="quiet"
				icon="x"
				onclick={() => {
					reportSearch = '';
					reportState = '';
					reportPage = 1;
				}}
			>
				Clear
			</Button>
		{/if}
	</div>
	{#if matching.length === 0}
		<p class="quiet">
			{reportFiltered ? 'No row matches these filters.' : 'This import has no rows.'}
		</p>
	{/if}
{/snippet}

{#snippet reportPager()}
	{#if matching.length > PER_PAGE}
		<Pagination
			page={reportAt}
			hasNext={reportAt < reportPages}
			label="Sheet rows"
			summary={`${pageSummary((reportAt - 1) * PER_PAGE, reportSlice.length, matching.length, 'rows')} · Page ${reportAt} of ${reportPages}`}
			onprevious={() => (reportPage = reportAt - 1)}
			onnext={() => (reportPage = reportAt + 1)}
		/>
	{/if}
{/snippet}

<!-- Cancel, beside the step's primary wherever the batch is still open. -->
{#snippet cancelButton()}
	<Button
		danger
		tier="quiet"
		disabled={settling}
		reason={settling ? 'Cancelling this import.' : undefined}
		onclick={() => void abandon()}
	>
		Cancel import
	</Button>
{/snippet}

<!-- The import step's controls, for whichever stage the batch is in. Drawn
     in the step's sticky footer and in the phone's action bar. -->
{#snippet importControls()}
	{#if stage !== null}
		{#if stage.kind === 'parsed' || stage.kind === 'attaching'}
			{#if confirming}
				<Button tier="primary" icon="circle-check" disabled={running} reason={running ? 'Adding your resources.' : undefined} onclick={() => void commit()}>
					{running ? 'Importing…' : 'Import them'}
				</Button>
				<Button tier="quiet" onclick={() => (confirming = false)}>Not yet</Button>
			{:else if stage.awaiting > 0 && stage.kind === 'parsed' && !adding}
				<Button tier="primary" icon="plus" onclick={() => void openAttach()}>
					Add the {stage.awaiting} {stage.awaiting === 1 ? 'file' : 'files'}
				</Button>
			{:else}
				<Button
					tier="primary"
					icon="circle-check"
					disabled={stage.awaiting > 0 || running}
					reason={stage.awaiting > 0
						? `${stage.awaiting} ${stage.awaiting === 1 ? 'row is' : 'rows are'} still waiting for a file.`
						: undefined}
					onclick={() => (confirming = true)}
				>
					Import {stage.preview.create} {stage.preview.create === 1 ? 'resource' : 'resources'}
				</Button>
			{/if}
			{@render cancelButton()}
		{:else if stage.kind === 'review'}
			<Button
				tier="primary"
				icon="circle-check"
				disabled
				reason={`${stage.pairs} ${stage.pairs === 1 ? 'pair is' : 'pairs are'} waiting on your answer above.`}
			>
				Carry on importing
			</Button>
			{@render cancelButton()}
		{:else if stage.kind === 'importing'}
			<Button
				tier="primary"
				disabled={running}
				reason={running ? 'Adding your resources.' : undefined}
				onclick={() => void commit()}
			>
				{running ? 'Importing…' : 'Carry on importing'}
			</Button>
		{/if}
	{/if}
{/snippet}

<div class="page flow-page has-bar">
	{#if detail !== null && shown !== null && stage !== null}
		{@const early = stage.kind === 'parsed' || stage.kind === 'attaching'}
		{@const open = early || stage.kind === 'review' || stage.kind === 'importing'}
		<PageHead
			icon="layout-list"
			back={{ href: '/import', label: 'Back to Import' }}
			title={detail.source_name}
			description={shown.line}
		>
			{#snippet aside()}
				<StatusPill tone={shown.tone} label={shown.label} />
			{/snippet}
		</PageHead>

		<div class="flow">
			<Stepper {steps} label="Spreadsheet import steps" />

			{#if missingFiles !== null}
				<Banner
					tone="warn"
					title={AWAITING_TITLE}
					action={stage.kind === 'parsed' && !adding ? addFiles : undefined}
				>
					{awaitingSay(missingFiles)}
				</Banner>
			{/if}

			{#if refusal !== null}
				<Banner
					tone="bad"
					title="That did not go through"
					action={resumable ? retryCommit : undefined}
				>
					{refusal}
				</Banner>
			{/if}

			<FlowStep
				n={1}
				id="check"
				title="Check"
				hint={early ? 'Read what we found in your sheet.' : 'Your rows and where each ended up.'}
				summary="{detail.row_count} rows · {detail.failed_count} with problems"
				done={steps[0].done}
				open={!(early && adding)}
			>
				<p class="sh-tally">
					<span><b>{detail.row_count}</b> rows</span>
					<span><b>{tally.attached}</b> files added</span>
					<span class:sh-warn={detail.failed_count > 0}><b>{detail.failed_count}</b> with problems</span>
					<span><b>{detail.live_count}</b> to publish</span>
				</p>
				{#each detail.warnings as warning, index (`${warning.kind}:${index}`)}
					<p class="flow-warn">{warningLine(warning)}</p>
				{/each}
				{@render reportControls()}
				{#if matching.length > 0}
					{@render reportTable(reportSlice)}
				{/if}
				{@render reportPager()}
			</FlowStep>

			{#if early}
				<FlowStep
					n={2}
					id="attach"
					title="Attach files"
					hint="Drop the files your sheet lists. We match each one to its row."
					summary={stage.awaiting === 0
						? 'Every file is added'
						: `${stage.awaiting} ${stage.awaiting === 1 ? 'file' : 'files'} to add`}
					done={steps[1].done}
					open={stage.kind === 'attaching' || adding}
				>
					<AttachPanel batch={batchId} {rows} onRow={applyRow} onRefresh={refetch} />
				</FlowStep>
			{/if}

			<FlowStep
				n={3}
				id="import"
				title="Import"
				hint={stage.kind === 'review'
					? shown.line
					: stage.kind === 'importing'
						? 'Adding your resources a few at a time. You can leave this page.'
						: early
							? confirming
								? previewSentence(stage.preview)
								: 'Add them to your catalogue. Nothing goes to a marketplace.'
							: shown.line}
				done={steps[2].done}
				footer={open ? importControls : undefined}
			>
				{#snippet aside()}
					<Explain title="What importing does" label="">
						<p>{NOTHING_SENT}</p>
						{#if early}
							<p>An import left unfinished is cleared on {expiryDay}, along with the files added to it.</p>
						{/if}
					</Explain>
				{/snippet}

				{#if stage.kind === 'review'}
					<ReviewCards
						pairs={detail.run?.review_pairs ?? []}
						onsame={(lo, hi, keep, fields) =>
							void decide(lo, hi, { verdict: 'same', keep, fields })}
						ondifferent={(lo, hi) => void decide(lo, hi, { verdict: 'different' })}
						onlater={(lo, hi) => void decide(lo, hi, { verdict: 'parked' })}
					/>
				{:else if stage.kind === 'importing'}
					<div
						class="sh-meter"
						role="progressbar"
						aria-label="Import progress"
						aria-valuenow={progress.kind === 'unstarted' ? undefined : progress.settled}
						aria-valuemin={0}
						aria-valuemax={progress.kind === 'unstarted' ? undefined : progress.total}
					>
						{#if progress.kind === 'running' && progress.fraction !== null}
							<div class="sh-fill" style="width: {Math.round(progress.fraction * 100)}%"></div>
						{/if}
					</div>
					<p class="sh-note">{progressLine(progress)}</p>
				{:else if early}
					<p class="imp-line">
						<span class="sh-big">{stage.preview.create}</span>
						{stage.preview.create === 1 ? 'resource' : 'resources'} ready to add
					</p>
				{/if}
			</FlowStep>
		</div>

		{#if open}
			<FlowActionBar>
				{@render importControls()}
			</FlowActionBar>
		{/if}
	{:else if unread}
		<PageHead
			icon="layout-list"
			back={{ href: '/import', label: 'Back to Import' }}
			title="We could not load this import"
			description="Nothing here has been changed."
		/>
		<Banner tone="bad" title="We could not load this import">{BATCH_UNREAD}</Banner>
	{:else if gone}
		<PageHead
			icon="layout-list"
			back={{ href: '/import', label: 'Back to Import' }}
			title="No such import"
			description="It may have been cancelled, or cleared after it expired."
		/>
	{:else}
		<p class="quiet">Loading this import…</p>
	{/if}
</div>
