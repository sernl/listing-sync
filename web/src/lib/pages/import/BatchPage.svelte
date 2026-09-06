<script lang="ts">
	import { page } from '$app/state';
	import {
		ApiFailure,
		api,
		type BatchStateView,
		type CommitAck,
		type ImportBatchDetailView,
		type ImportRowView
	} from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { utcInstant } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import AttachPanel from './AttachPanel.svelte';
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
</script>

<!-- Both actions are declared here and handed to the banner as a prop, so a
     banner with nothing to offer carries no empty action slot. -->
{#snippet retryCommit()}
	<Button
		disabled={running}
		reason={running ? 'A batch of rows is being created.' : undefined}
		onclick={() => void commit()}
	>
		Try again
	</Button>
{/snippet}

{#snippet addFiles()}
	<Button
		tier="primary"
		icon="plus"
		onclick={() => {
			missingFiles = null;
			adding = true;
		}}
	>
		Add the files
	</Button>
{/snippet}

{#snippet reportList(entries: ReturnType<typeof reportRows>)}
	{#each entries as entry (entry.key)}
		<div class="sh-row">
			<span class="sh-who">
				<span class="t">{entry.name ?? entry.label}</span>
				<span class="w">
					{entry.label}{entry.marketplace === null ? '' : ` — ${entry.marketplace}`}
				</span>
			</span>
			<span class="sh-at">
				{#if entry.attached}<StatusPill tone="flat" label="File added" />{/if}
				<StatusPill tone={entry.tone} label={entry.stateLabel} />
				{#if entry.href !== null}
					<Button small href={entry.href}>Open</Button>
				{/if}
			</span>
			{#if entry.problems.length > 0 || entry.failureDetail !== null}
				<ul class="sh-probs">
					{#each entry.problems as problem, index (`${entry.key}:${index}`)}
						<li><span class="col">{problem.column}:</span> {problem.problem}</li>
					{/each}
					{#if entry.failureDetail !== null}
						<li>{entry.failureDetail}</li>
					{/if}
				</ul>
			{/if}
		</div>
	{/each}
{/snippet}

<div class="page">
	{#if detail !== null && shown !== null && stage !== null}
		<PageHead
			icon="layout-list"
			back={{ href: '/import', label: 'Back to Import' }}
			title={detail.source_name}
			description="A spreadsheet import: what it read, what it will create, and what it refused."
		>
			{#snippet aside()}
				<StatusPill tone={shown.tone} label={shown.label} />
			{/snippet}
		</PageHead>

		<p class="sh-lead">{shown.line}</p>

		{#if missingFiles !== null}
			<!-- The way out is offered only from the report face, because it is
			     the one face the attach panel is not already on. -->
			<Banner
				tone="warn"
				title={AWAITING_TITLE}
				action={stage.kind === 'parsed' ? addFiles : undefined}
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

		<div class="sh-figs">
			<div class="sh-fig"><b>{detail.row_count}</b><span>rows read</span></div>
			<div class="sh-fig"><b>{tally.attached}</b><span>files added</span></div>
			<div class="sh-fig"><b>{detail.failed_count}</b><span>refused when read</span></div>
			<div class="sh-fig"><b>{detail.live_count}</b><span>asked to go live</span></div>
		</div>

		{#each detail.warnings as warning, index (`${warning.kind}:${index}`)}
			<Banner tone="warn" title={warning.kind === 'new_label' ? 'A new label' : 'Not connected'}>
				{warningLine(warning)}
			</Banner>
		{/each}

		{#if stage.kind === 'parsed' && !adding}
			<Panel title="What your sheet said" description="Every row, in the order you filled them.">
				{@render reportList(report)}
			</Panel>

			{#if confirming}
				<Banner tone="info" title="Ready to import">
					{previewSentence(stage.preview)}
					{NOTHING_SENT}
					{#snippet action()}
						<Button tier="primary" disabled={running} onclick={() => void commit()}>
							Import them
						</Button>
					{/snippet}
				</Banner>
			{/if}

			<div class="sh-acts">
				{#if stage.awaiting > 0}
					<Button tier="primary" icon="plus" onclick={() => (adding = true)}>
						Add the {stage.awaiting}
						{stage.awaiting === 1 ? 'file' : 'files'} it names
					</Button>
				{:else}
					<Button
						tier="primary"
						icon="circle-check"
						disabled={running || confirming}
						reason={confirming ? 'Confirm above.' : undefined}
						onclick={() => (confirming = true)}
					>
						Import {stage.preview.create}
						{stage.preview.create === 1 ? 'resource' : 'resources'}
					</Button>
				{/if}
				<Button
					danger
					disabled={settling}
					reason={settling ? 'Giving this import up.' : undefined}
					onclick={() => void abandon()}
				>
					Give this import up
				</Button>
			</div>
			<p class="sh-note">
				An import left unfinished is cleared on {expiryDay}, along with the files added to it.
			</p>
		{:else if stage.kind === 'attaching' || (stage.kind === 'parsed' && adding)}
			<Panel
				title="Add the files"
				description="Every row that names a marketplace needs the file buyers download."
			>
				<AttachPanel batch={batchId} {rows} onRow={applyRow} onRefresh={refetch} />
			</Panel>

			{#if confirming}
				<Banner tone="info" title="Ready to import">
					{previewSentence(stage.preview)}
					{NOTHING_SENT}
					{#snippet action()}
						<Button tier="primary" disabled={running} onclick={() => void commit()}>
							Import them
						</Button>
					{/snippet}
				</Banner>
			{/if}

			<div class="sh-acts">
				<Button
					tier="primary"
					icon="circle-check"
					disabled={stage.awaiting > 0 || running || confirming}
					reason={stage.awaiting > 0
						? `${stage.awaiting} ${stage.awaiting === 1 ? 'row is' : 'rows are'} still waiting for a file.`
						: confirming
							? 'Confirm above.'
							: undefined}
					onclick={() => (confirming = true)}
				>
					Import these resources
				</Button>
				<Button
					danger
					disabled={settling}
					reason={settling ? 'Giving this import up.' : undefined}
					onclick={() => void abandon()}
				>
					Give this import up
				</Button>
			</div>
			<p class="sh-note">
				An import left unfinished is cleared on {expiryDay}, along with the files added to it.
			</p>
		{:else if stage.kind === 'importing'}
			<Panel title="Creating your resources">
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
				<div class="sh-acts">
					<Button
						tier="primary"
						disabled={running}
						reason={running ? 'A batch of rows is being created.' : undefined}
						onclick={() => void commit()}
					>
						{running ? 'Importing…' : 'Carry on importing'}
					</Button>
				</div>
				<p class="sh-note">{NOTHING_SENT}</p>
			</Panel>
		{:else if stage.kind === 'unrecognised'}
			<Panel title="This import is in a state this page does not know">
				<p class="sh-note">{shown.line}</p>
			</Panel>
		{:else}
			<Panel title="What was created" description="Every row, and where it ended up.">
				{@render reportList(report)}
			</Panel>
			<p class="sh-note">{NOTHING_SENT}</p>
		{/if}
	{:else if unread}
		<PageHead
			icon="layout-list"
			back={{ href: '/import', label: 'Back to Import' }}
			title="This import could not be read"
			description="Nothing here has been changed."
		/>
		<Banner tone="bad" title="This import could not be read">{BATCH_UNREAD}</Banner>
	{:else if gone}
		<PageHead
			icon="layout-list"
			back={{ href: '/import', label: 'Back to Import' }}
			title="No such import"
			description="It may have been given up, or cleared after it expired."
		/>
	{:else}
		<p class="sh-lead">Reading this import…</p>
	{/if}
</div>
