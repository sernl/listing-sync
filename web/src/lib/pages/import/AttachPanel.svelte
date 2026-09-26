<script lang="ts">
	import {
		ApiFailure,
		api,
		type BatchStateView,
		type ImportRowView,
		type UploadedView
	} from '$lib/api';
	import { quotaSentence } from '$lib/authoring';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Icon from '$lib/Icon.svelte';
	import Pagination from '$lib/Pagination.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import {
		attachedRows,
		awaitingRows,
		batchClosedSay,
		headroomLine,
		matchFiles,
		rowLabel,
		type WaitingRow
	} from './sheet-view';
	import { pageCount, pageSummary } from './run-view';

	let {
		batch,
		rows,
		onRow,
		onRefresh
	}: {
		batch: string;
		rows: ImportRowView[];
		/** One row as the bind answered it, with the batch's own new state.
		 *  Applied rather than re-read, which is why the bind answers a row at
		 *  all: a drop of forty files would otherwise re-read the whole batch
		 *  forty times. */
		onRow: (row: ImportRowView, state: BatchStateView) => void;
		/** Read the batch again. Used only after an unbind, which answers 204
		 *  and so carries no row to apply. */
		onRefresh: () => Promise<void>;
	} = $props();

	const base = $props.id();
	const inputId = `${base}-files`;

	let uploaded = $state<UploadedView | null>(null);
	let unplaced = $state<{ id: string; file: File; reason: string; chosen: string }[]>([]);
	let sending = $state<string | null>(null);
	let refusal = $state<string | null>(null);
	let over = $state(false);

	const waiting = $derived(awaitingRows(rows));
	const held = $derived(attachedRows(rows));
	const headroom = $derived(headroomLine(uploaded));
	const busy = $derived(sending !== null);

	/** How many rows of each list are drawn at once.
	 *
	 *  `waiting` and `held` themselves stay whole: the filename matcher below
	 *  places a dropped file against every row still waiting for one, and the
	 *  picker for a file that matched nothing must be able to name any of
	 *  them. A file whose row is on page three would otherwise come back as
	 *  "no row named this file", which is the defect paging must not
	 *  introduce. Only the drawing is paged. */
	const PER_PAGE = 25;

	let waitingPage = $state(1);
	let heldPage = $state(1);

	const waitingPages = $derived(pageCount(waiting.length, PER_PAGE));
	const heldPages = $derived(pageCount(held.length, PER_PAGE));
	// Binding a file moves a row from one list to the other, so both lists
	// shrink and grow under the seller: a page index past the end is clamped
	// rather than left showing nothing.
	const waitingAt = $derived(Math.min(waitingPage, waitingPages));
	const heldAt = $derived(Math.min(heldPage, heldPages));
	const waitingShown = $derived(
		waiting.slice((waitingAt - 1) * PER_PAGE, waitingAt * PER_PAGE)
	);

	// The strip of files that matched no row, or more than one. Each entry
	// carries a picker naming every row still waiting, so the strip is the
	// most expensive list on this panel to draw — and the one a bulk drop of
	// two hundred files fills. Paged for that reason; the picker's options
	// stay whole, because the row a file belongs to may be any of them.
	let unplacedPage = $state(1);
	const unplacedPages = $derived(pageCount(unplaced.length, PER_PAGE));
	const unplacedAt = $derived(Math.min(unplacedPage, unplacedPages));
	const unplacedShown = $derived(
		unplaced.slice((unplacedAt - 1) * PER_PAGE, unplacedAt * PER_PAGE)
	);
	const heldShown = $derived(held.slice((heldAt - 1) * PER_PAGE, heldAt * PER_PAGE));

	/** An archive that unpacked into several files, which one row cannot hold.
	 *
	 *  Unreachable while every upload here is sent `keep_whole`, and stated
	 *  rather than assumed away: a row carries exactly one payload handle, and
	 *  choosing one of several would be this page picking the seller's file for
	 *  them. */
	const SEVERAL_PARTS =
		'That upload turned into several files, but each row takes one. Zip them together and drop the zip.';

	function waitingOf(row: ImportRowView): WaitingRow {
		return { sheet: row.sheet, ordinal: row.ordinal, fileName: row.file_name };
	}

	/** How a row is named in the picker's value. Split at the last colon,
	 *  because a tab title may hold anything the seller renamed it to. */
	function rowValue(row: ImportRowView): string {
		return `${row.sheet}:${row.ordinal}`;
	}

	function rowFrom(value: string): { sheet: string; ordinal: number } | null {
		const cut = value.lastIndexOf(':');
		if (cut <= 0) {
			return null;
		}
		const ordinal = Number(value.slice(cut + 1));
		return Number.isFinite(ordinal) ? { sheet: value.slice(0, cut), ordinal } : null;
	}

	function refusalOf(failure: unknown): string {
		if (!(failure instanceof ApiFailure)) {
			return 'That did not finish. Nothing was changed.';
		}
		if (failure.status === 0) {
			return 'That did not reach us. Nothing was saved. Try again.';
		}
		if (failure.code() === 'quota_exceeded') {
			return quotaSentence(failure.body?.errors[0]?.detail) ?? failure.message;
		}
		if (failure.code() === 'blob_store_unavailable') {
			return 'We cannot store files right now, so nothing was added.';
		}
		if (failure.status === 404) {
			return 'That row is no longer part of this import. Reload the page.';
		}
		if (failure.status === 409) {
			return batchClosedSay(failure.body?.errors[0]?.detail) ?? failure.message;
		}
		return failure.message;
	}

	function hold(file: File, reason: string) {
		unplaced = [
			...unplaced,
			{ id: `${file.name}|${crypto.randomUUID()}`, file, reason, chosen: '' }
		];
	}

	async function place(file: File, sheet: string, ordinal: number): Promise<boolean> {
		sending = file.name;
		try {
			// `keep_whole` on every tab, deliberately. A row holds exactly one
			// payload handle, so a mode that can answer several would not fit
			// the row this file is being bound to.
			const stored = await api.upload(file, 'keep_whole');
			uploaded = stored;
			if (stored.payload.length !== 1) {
				hold(file, SEVERAL_PARTS);
				return false;
			}
			const bound = await api.bindImportFile(batch, sheet, ordinal, {
				payload: stored.payload[0],
				cover: stored.cover
			});
			onRow(bound.row, bound.batch_state);
			return true;
		} catch (failure) {
			refusal = refusalOf(failure);
			return false;
		} finally {
			sending = null;
		}
	}

	/** Take a drop, one file at a time.
	 *
	 *  Sequentially rather than in parallel: every upload answers this
	 *  organisation's storage headroom, and forty in flight at once would each
	 *  answer a figure that was already stale when it was composed. */
	async function take(files: readonly File[]) {
		refusal = null;
		const verdicts = matchFiles(
			files.map((file) => file.name),
			waiting.map(waitingOf)
		);
		for (const [index, file] of files.entries()) {
			const verdict = verdicts[index];
			if (verdict.kind === 'unplaced') {
				hold(file, verdict.reason);
				continue;
			}
			await place(file, verdict.row.sheet, verdict.row.ordinal);
		}
	}

	function chosen(event: Event & { currentTarget: HTMLInputElement }) {
		const files = Array.from(event.currentTarget.files ?? []);
		event.currentTarget.value = '';
		if (files.length > 0) {
			void take(files);
		}
	}

	function dropped(event: DragEvent) {
		event.preventDefault();
		over = false;
		const files = Array.from(event.dataTransfer?.files ?? []);
		if (files.length > 0) {
			void take(files);
		}
	}

	async function placeByHand(entry: { id: string; file: File; chosen: string }) {
		const row = rowFrom(entry.chosen);
		if (row === null) {
			return;
		}
		if (await place(entry.file, row.sheet, row.ordinal)) {
			unplaced = unplaced.filter((other) => other.id !== entry.id);
		}
	}

	function drop(id: string) {
		unplaced = unplaced.filter((other) => other.id !== id);
	}

	async function unbind(row: ImportRowView) {
		refusal = null;
		sending = row.file_name ?? rowLabel(row);
		try {
			await api.unbindImportFile(batch, row.sheet, row.ordinal);
		} catch (failure) {
			refusal = refusalOf(failure);
			return;
		} finally {
			sending = null;
		}
		// The unbind answers 204, so there is no row to apply: the batch is read
		// again rather than this page deciding what the row now says.
		await onRefresh();
	}
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<label
	class="drop sh-drop {over ? 'over' : ''}"
	for={inputId}
	ondragover={(event) => {
		event.preventDefault();
		over = true;
	}}
	ondragleave={() => (over = false)}
	ondrop={dropped}
>
	<span class="sh-drop-icon" aria-hidden="true"><Icon name="upload" size={22} /></span>
	<b>{busy ? `Uploading ${sending}…` : 'Drop the files here, or choose them'}</b>
	{waiting.length === 0
		? 'Every row has its file.'
		: `${waiting.length} ${waiting.length === 1 ? 'row is' : 'rows are'} waiting for a file.`}
	<input id={inputId} type="file" multiple disabled={busy} onchange={chosen} />
</label>

{#if headroom !== null}
	<p class="sh-note">{headroom}</p>
{/if}

{#if refusal !== null}
	<Banner tone="bad" title="That file was not added">{refusal}</Banner>
{/if}

{#if unplaced.length > 0}
	<div class="sh-strip">
		{#each unplacedShown as entry (entry.id)}
			<div class="sh-unplaced">
				<span class="n">{entry.file.name}</span>
				<span class="why">{entry.reason}</span>
				<select
					aria-label={`The row ${entry.file.name} belongs to`}
					bind:value={entry.chosen}
					disabled={busy || waiting.length === 0}
				>
					<option value="">Choose a row…</option>
					{#each waiting as row (rowValue(row))}
						<option value={rowValue(row)}>
							{rowLabel(row)}{row.file_name === null ? '' : ` — ${row.file_name}`}
						</option>
					{/each}
				</select>
				<Button
					small
					disabled={busy || entry.chosen === ''}
					reason={entry.chosen === '' ? 'Choose the row this file belongs to.' : undefined}
					onclick={() => void placeByHand(entry)}
				>
					Place
				</Button>
				<Button
					small
					tier="quiet"
					disabled={busy}
					reason={busy ? 'A file is being uploaded.' : undefined}
					onclick={() => drop(entry.id)}
				>
					Leave out
				</Button>
			</div>
		{/each}
	</div>
	{#if unplaced.length > PER_PAGE}
		<Pagination
			page={unplacedAt}
			hasNext={unplacedAt < unplacedPages}
			label="Files waiting to be placed"
			summary={`${pageSummary((unplacedAt - 1) * PER_PAGE, unplacedShown.length, unplaced.length, 'files to place')} · Page ${unplacedAt} of ${unplacedPages}`}
			onprevious={() => (unplacedPage = unplacedAt - 1)}
			onnext={() => (unplacedPage = unplacedAt + 1)}
		/>
	{/if}
{/if}

{#if held.length > 0 || waiting.length > 0}
	<ul class="sh-files">
		{#each waitingShown as row (rowValue(row))}
			<li>
				<span class="sh-ord" title={rowLabel(row)}>{row.ordinal}</span>
				<span class="sh-files-what">
					<span class="res-name">{row.title ?? row.file_name ?? rowLabel(row)}</span>
					<span class="sh-file">
						{row.file_name ?? 'No file named in this row. Place one by hand.'}
					</span>
				</span>
				<StatusPill tone="warn" label="Waiting" />
			</li>
		{/each}
		{#each heldShown as row (rowValue(row))}
			<li>
				<span class="sh-ord" title={rowLabel(row)}>{row.ordinal}</span>
				<span class="sh-files-what">
					<span class="res-name">{row.title ?? row.file_name ?? rowLabel(row)}</span>
					{#if row.file_name !== null}<span class="sh-file">{row.file_name}</span>{/if}
				</span>
				<span class="sh-files-acts">
					<StatusPill tone="ok" label="Added" />
					<Button
						small
						tier="quiet"
						danger
						icon="x"
						label="Remove"
						disabled={busy}
						reason={busy ? 'A file is being uploaded.' : undefined}
						onclick={() => void unbind(row)}
					>
						Remove
					</Button>
				</span>
			</li>
		{/each}
	</ul>
{/if}

{#if waiting.length > PER_PAGE}
	<Pagination
		page={waitingAt}
		hasNext={waitingAt < waitingPages}
		label="Rows waiting for a file"
		summary={`${pageSummary((waitingAt - 1) * PER_PAGE, waitingShown.length, waiting.length, 'rows waiting')} · Page ${waitingAt} of ${waitingPages}`}
		onprevious={() => (waitingPage = waitingAt - 1)}
		onnext={() => (waitingPage = waitingAt + 1)}
	/>
{/if}

{#if held.length > PER_PAGE}
	<Pagination
		page={heldAt}
		hasNext={heldAt < heldPages}
		label="Rows with a file"
		summary={`${pageSummary((heldAt - 1) * PER_PAGE, heldShown.length, held.length, 'rows with a file')} · Page ${heldAt} of ${heldPages}`}
		onprevious={() => (heldPage = heldAt - 1)}
		onnext={() => (heldPage = heldAt + 1)}
	/>
{/if}
