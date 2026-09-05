<script lang="ts">
	import { ApiFailure, api } from '$lib/api';
	import type { InventoryRow } from '$lib/inventory';
	import { platformTitle } from '$lib/platforms';
	import { standingOf } from '$lib/tes-portfolio';
	import type { InventoryId } from '$lib/generated/vocab';

	let {
		open,
		rows,
		onClose,
		onDeleted
	}: {
		open: boolean;
		/** The selected rows, in the order the table shows them. */
		rows: InventoryRow[];
		onClose: () => void;
		onDeleted: (deleted: number) => void;
	} = $props();

	let element = $state<HTMLDialogElement | null>(null);
	/** Undefaulted on purpose. The single-item dialog defaults to removing
	 *  everywhere, and defaulting the same way over a selection would fire a
	 *  removal per bound mapping per item from one click; defaulting the other
	 *  way would abandon live listings silently. Neither is a choice this
	 *  console makes for the seller, so the confirm stays disabled until they
	 *  make it. */
	let policy = $state<'remove' | 'leave' | null>(null);
	let sending = $state(false);
	let refusal = $state<string | null>(null);
	let done = $state(0);

	$effect(() => {
		if (open && element !== null && !element.open) {
			element.showModal();
		} else if (!open) {
			element?.close();
		}
	});

	$effect(() => {
		if (!open) {
			policy = null;
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

	async function run() {
		if (policy === null) {
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
				const removeFrom = policy === 'remove' ? boundOf(row) : [];
				await api.deleteProduct(row.product.id, {
					remove_from: removeFrom,
					leave_live: policy === 'leave'
				});
				deleted += 1;
				done = deleted;
			}
			onDeleted(deleted);
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure
					? `${failure.message} ${deleted} of ${rows.length} were deleted before this.`
					: `The delete stopped after ${deleted} of ${rows.length}. The rest are untouched.`;
			if (deleted > 0) {
				onDeleted(deleted);
			}
		} finally {
			sending = false;
		}
	}
</script>

<dialog bind:this={element} aria-labelledby="bulk-delete-title" onclose={onClose}>
	<div class="dialog-body">
		<h2 id="bulk-delete-title">
			Delete {rows.length}
			{rows.length === 1 ? 'item' : 'items'}
		</h2>
		<p>
			This removes {rows.length === 1 ? 'it' : 'them'} from your Resources here, and it cannot be
			undone. What happens to the listings already on a marketplace is the choice below, and there
			is no default.
		</p>

		{#if withListings === 0}
			<p class="foot-note">
				None of the selected {rows.length === 1 ? 'item is' : 'items are'} on a marketplace yet, so
				there is nothing standing to decide about. Either choice does the same thing here.
			</p>
		{:else}
			<p class="foot-note">
				{withListings} of {rows.length}
				{withListings === 1 ? 'is' : 'are'} on a marketplace, across {platforms
					.map((inventory) => platformTitle(inventory))
					.join(', ')}, for {standing.length}
				{standing.length === 1 ? 'listing' : 'listings'} in total.
			</p>
		{/if}

		<div class="inline-choices">
			<label>
				<input
					type="radio"
					name="bulk-delete-policy"
					checked={policy === 'leave'}
					disabled={sending}
					onchange={() => (policy = 'leave')}
				/>
				Leave the listings standing
			</label>
			<label>
				<input
					type="radio"
					name="bulk-delete-policy"
					checked={policy === 'remove'}
					disabled={sending}
					onchange={() => (policy = 'remove')}
				/>
				Remove them from every marketplace too
			</label>
		</div>
		<p class="foot-note">
			Leaving them standing is what a reselling tool does, and it means those listings keep selling
			with nothing here tracking them. Removing them enqueues one removal per listing on the same
			ledger every other write travels, so {standing.length}
			{standing.length === 1 ? 'write' : 'writes'} would start from this one click, and work for Tes
			and TPT waits while your own device is off.
		</p>

		{#if refusal !== null}
			<p class="refusal">{refusal}</p>
		{/if}

		<div class="actions">
			<button class="btn" type="button" onclick={onClose} disabled={sending}>Keep them</button>
			<button
				class="btn danger"
				type="button"
				onclick={run}
				disabled={sending || policy === null || rows.length === 0}
			>
				{sending ? `Deleting ${done} of ${rows.length}…` : `Delete ${rows.length}`}
			</button>
		</div>
	</div>
</dialog>
