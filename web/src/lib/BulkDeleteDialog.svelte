<script lang="ts">
	import { ApiFailure, api } from '$lib/api';
	import type { InventoryRow } from '$lib/inventory';
	import { platformTitle } from '$lib/platforms';
	import { standingOf } from '$lib/tes-portfolio';
	import type { InventoryId } from '$lib/generated/vocab';
	import Note from '$lib/Note.svelte';

	let {
		open,
		rows,
		onClose,
		onDeleted,
		onPartial
	}: {
		open: boolean;
		/** The selected rows, in the order the table shows them. */
		rows: InventoryRow[];
		onClose: () => void;
		onDeleted: (deleted: number) => void;
		onPartial: (deleted: number) => void;
	} = $props();

	let element = $state<HTMLDialogElement | null>(null);
	let removeFrom = $state<Set<InventoryId>>(new Set());
	let leaveLive = $state(false);
	let sending = $state(false);
	let refusal = $state<string | null>(null);
	let done = $state(0);

	$effect(() => {
		if (open && element !== null && !element.open) {
			removeFrom = new Set();
			leaveLive = false;
			refusal = null;
			done = 0;
			element.showModal();
		} else if (!open) {
			element?.close();
		}
	});

	$effect(() => {
		if (!open) {
			removeFrom = new Set();
			leaveLive = false;
			refusal = null;
			done = 0;
		}
	});

	/** The mappings a marketplace actually holds a listing for. An unbound
	 *  mapping names nothing to remove, which the server refuses rather than
	 *  treating as a silent no-op. */
	function boundOf(row: InventoryRow): InventoryId[] {
		return [...row.mapped.values()]
			.filter((mapping) => standingOf(mapping) !== 'unsent')
			.map((mapping) => mapping.inventory);
	}

	const standing = $derived(
		rows.flatMap((row) => boundOf(row).map((inventory) => ({ row, inventory })))
	);
	const withListings = $derived(rows.filter((row) => boundOf(row).length > 0).length);
	const platforms = $derived([...new Set(standing.map((entry) => entry.inventory))]);
	const leftStanding = $derived(platforms.filter((inventory) => !removeFrom.has(inventory)));
	const blocked = $derived(leftStanding.length > 0 && !leaveLive);
	const removalCount = $derived(
		standing.filter((entry) => removeFrom.has(entry.inventory)).length
	);

	function toggle(inventory: InventoryId, on: boolean) {
		const next = new Set(removeFrom);
		if (on) {
			next.add(inventory);
		} else {
			next.delete(inventory);
		}
		removeFrom = next;
	}

	async function run() {
		if (sending || blocked || rows.length === 0 || refusal !== null) {
			return;
		}
		sending = true;
		refusal = null;
		let deleted = 0;
		try {
			// Sequential rather than parallel: a refusal then names the item it
			// belongs to, and every item before it is already gone rather than
			// left in an unknown state by a batch that half-succeeded.
			for (const row of rows) {
				const selected = boundOf(row).filter((inventory) => removeFrom.has(inventory));
				await api.deleteProduct(row.product.id, {
					remove_from: selected,
					leave_live: leaveLive
				});
				deleted += 1;
				done = deleted;
			}
			onDeleted(deleted);
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure
					? `${failure.message} Deleted ${deleted} of ${rows.length}.`
					: `Deleted ${deleted} of ${rows.length}. We couldn't confirm the last one, so check Resources and Updates before you try it again.`;
			onPartial(deleted);
		} finally {
			sending = false;
		}
	}
</script>

<dialog
	bind:this={element}
	aria-labelledby="bulk-delete-title"
	onclose={onClose}
	oncancel={(event) => {
		if (sending) event.preventDefault();
	}}
>
	<div class="dialog-body">
		<h2 id="bulk-delete-title">
			{#if refusal !== null}
				Deleting stopped
			{:else}
				Delete {rows.length} {rows.length === 1 ? 'resource' : 'resources'}
			{/if}
		</h2>
		<p>Tick a marketplace below to remove its listings too.</p>

		{#if refusal === null}
		{#if withListings === 0}
			<Note>None of these resources is listed on a marketplace.</Note>
		{:else}
			<Note>
				{withListings} of {rows.length}
				{withListings === 1 ? 'is' : 'are'} on a marketplace, across {platforms
					.map((inventory) => platformTitle(inventory))
					.join(', ')}, for {standing.length}
				{standing.length === 1 ? 'listing' : 'listings'} in total.
			</Note>
		{/if}

		{#if platforms.length > 0}
			<div class="inline-choices">
				{#each platforms as inventory (inventory)}
					<label>
						<input
							type="checkbox"
							checked={removeFrom.has(inventory)}
							disabled={sending}
							onchange={(event) => toggle(inventory, event.currentTarget.checked)}
						/>
						Also remove from {platformTitle(inventory)}
					</label>
				{/each}
			</div>
			{#if leftStanding.length > 0}
				<div class="notice">
					Your listings on {leftStanding.map(platformTitle).join(', ')} stay as they are.
					Teachouse stops tracking them once these resources are deleted.
				</div>
				<div class="inline-choices">
					<label>
						<input type="checkbox" bind:checked={leaveLive} disabled={sending} />
						Leave your other marketplace listings as they are
					</label>
				</div>
			{/if}
			<Note icon="refresh-cw">
				Follow the {removalCount}
				{removalCount === 1 ? 'removal' : 'removals'} on Updates.
			</Note>
		{/if}
		{/if}

		{#if refusal !== null}
			<p class="refusal">{refusal}</p>
		{/if}

		<div class="actions">
			<button class="btn" type="button" onclick={onClose} disabled={sending}
				>{refusal !== null ? 'Close' : 'Keep them'}</button
			>
			{#if refusal === null}
				<button
					class="btn danger"
					type="button"
					onclick={run}
					disabled={sending || blocked || rows.length === 0}
				>
					{sending ? `Deleting ${done} of ${rows.length}…` : `Delete ${rows.length}`}
				</button>
			{/if}
		</div>
	</div>
</dialog>
