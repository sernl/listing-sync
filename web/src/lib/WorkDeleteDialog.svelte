<script lang="ts">
	// Deleting work history: one import, one migration, one publishing run, or
	// a selection of them. The same dialog for one row and for many, because
	// the sentence a seller has to read is the same sentence and a second
	// dialog for the plural case is where the two drift apart.
	//
	// Not `DeleteDialog` or `BulkDeleteDialog`: those delete resources and
	// offer to remove live marketplace listings with them. This one removes
	// nothing outside the seller's own history, and saying so is most of what
	// it is for.
	import { ApiFailure, type JobDeletionView } from '$lib/api';
	import Note from '$lib/Note.svelte';
	import {
		WORK_DELETE_IN_FLIGHT,
		WORK_DELETE_KEEPS,
		answered,
		countWord,
		outcomeLines,
		withFailure,
		withStatus,
		type WorkDeleteOutcome,
		type WorkItem,
		type WorkNoun
	} from '$lib/work-delete';

	let {
		open,
		items,
		noun,
		remove,
		onClose,
		onsettled
	}: {
		open: boolean;
		/** The rows this Delete would act on, in the order the list shows
		 *  them. One for a row's own control, the selection for a bulk. */
		items: WorkItem[];
		noun: WorkNoun;
		/** The one server call, supplied by the caller because the three
		 *  histories have three endpoints and this dialog knows none of
		 *  them. */
		remove: (id: string) => Promise<JobDeletionView>;
		onClose: () => void;
		/** What happened, handed back as soon as the run finishes so the caller
		 *  can refresh its list and drop the rows that were accepted while
		 *  keeping the refusals ticked. Called once per run, including a
		 *  retry. */
		onsettled: (outcome: WorkDeleteOutcome) => void;
	} = $props();

	let element = $state<HTMLDialogElement | null>(null);
	let sending = $state(false);
	let done = $state(0);
	let outcome = $state<WorkDeleteOutcome>({
		deleted: [],
		stopping: [],
		needsReview: [],
		failed: []
	});

	// Guarded on the element's own state: `showModal` on a dialog that is
	// already modal throws, and this effect re-runs whenever the element is
	// bound as well as when `open` moves.
	$effect(() => {
		if (open && element !== null && !element.open) {
			element.showModal();
		} else if (!open) {
			element?.close();
		}
	});

	$effect(() => {
		if (!open) {
			outcome = { deleted: [], stopping: [], needsReview: [], failed: [] };
			done = 0;
		}
	});

	const reported = $derived(outcomeLines(outcome, noun));
	const anything = $derived(answered(outcome) > 0);
	// What a retry would act on: the rows the server never answered about.
	// The accepted ones are not offered again — repeating a delete is
	// idempotent on the server, but a control that re-sends a row already
	// stopping reads as though the first press did nothing.
	const stuck = $derived(
		outcome.failed
			.map((failure) => items.find((item) => item.id === failure.id))
			.filter((item): item is WorkItem => item !== undefined)
	);

	async function run(wanted: WorkItem[]) {
		if (wanted.length === 0 || sending) {
			return;
		}
		sending = true;
		done = 0;
		// A retry starts from the refusals only, so the successes already
		// reported stay reported rather than being re-sent or forgotten.
		let result: WorkDeleteOutcome = { ...outcome, failed: [] };
		try {
			// One call per row, in the order the list shows them, and
			// sequential on purpose: a refusal then names the row it belongs
			// to, and the rows before it are settled rather than left in an
			// unknown state by a fan-out that half-succeeded.
			for (const item of wanted) {
				try {
					const answer = await remove(item.id);
					result = withStatus(result, item.id, answer.status);
				} catch (failure) {
					result = withFailure(
						result,
						item,
						failure instanceof ApiFailure
							? failure.message
							: 'The response was lost. Check the refreshed history before retrying.'
					);
				}
				done += 1;
				outcome = result;
			}
		} finally {
			sending = false;
			outcome = result;
			onsettled(result);
		}
	}
</script>

<dialog
	bind:this={element}
	aria-labelledby="work-delete-title"
	onclose={onClose}
	oncancel={(event) => {
		if (sending) event.preventDefault();
	}}
>
	<div class="dialog-body">
		<h2 id="work-delete-title">
			{items.length === 1
				? `Delete “${items[0]?.label ?? ''}”`
				: `Delete ${countWord(items.length, noun)}`}
		</h2>

		<p>{WORK_DELETE_KEEPS}</p>
		<Note>{WORK_DELETE_IN_FLIGHT}</Note>

		{#if items.length > 1}
			<!-- Named rather than counted. A selection survives turning the
			     page, so some of these rows are not on screen and a bare
			     figure would be the seller's only description of what they
			     are about to delete. -->
			<ul class="work-delete-list">
				{#each items as item (item.id)}
					<li>{item.label}</li>
				{/each}
			</ul>
		{/if}

		{#if anything}
			<!-- What the server actually answered, arm by arm. A 202 is not a
			     deletion and a row kept for review is still in the history,
			     so neither is reported as one. -->
			<div class="notice">
				{#each reported as line (line)}
					<p>{line}</p>
				{/each}
			</div>
		{/if}

		<div class="actions">
			<button class="btn" type="button" onclick={onClose} disabled={sending}>
				{anything ? 'Close' : items.length === 1 ? 'Keep it' : 'Keep them'}
			</button>
			{#if stuck.length > 0 && !sending}
				<button class="btn" type="button" onclick={() => void run(stuck)}>
					Try {countWord(stuck.length, noun)} again
				</button>
			{:else if !anything || sending}
				<button
					class="btn danger"
					type="button"
					onclick={() => void run(items)}
					disabled={sending || items.length === 0}
				>
					{sending
						? `Deleting ${done} of ${items.length}…`
						: items.length === 1
							? 'Delete it'
							: `Delete ${countWord(items.length, noun)}`}
				</button>
			{/if}
		</div>
	</div>
</dialog>

<style>
	/* The rows a bulk Delete names. Bounded in height on purpose: a seller
	   may have ticked forty runs across four pages, and a list that pushed
	   the confirm off the sheet would be a dialog with no way to answer it.
	   Tokens only. */
	.work-delete-list {
		margin: 0 0 12px;
		padding-left: 18px;
		max-height: 168px;
		overflow-y: auto;
		font-size: 12.5px;
		line-height: 1.5;
		color: var(--muted);
	}
</style>
