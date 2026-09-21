<script lang="ts">
	import { ApiFailure, api, type MappingHead } from '$lib/api';
	import { platformTitle } from '$lib/platforms';
	import { standingOf } from '$lib/tes-portfolio';
	import type { InventoryId } from '$lib/generated/vocab';
	import Note from '$lib/Note.svelte';

	let {
		open,
		product,
		title,
		mappings,
		onClose,
		onDeleted
	}: {
		open: boolean;
		product: string;
		title: string;
		mappings: MappingHead[];
		onClose: () => void;
		onDeleted: () => void;
	} = $props();

	let element = $state<HTMLDialogElement | null>(null);
	let removeFrom = $state<Set<InventoryId>>(new Set());
	let leaveLive = $state(false);
	let sending = $state(false);
	let refusal = $state<string | null>(null);

	// Only a bound mapping names a listing to remove. Electing an unbound one
	// asks us to remove something that was never created, which the server
	// refuses rather than treating as a silent no-op.
	const bound = $derived(mappings.filter((mapping) => standingOf(mapping) !== 'unsent'));

	// Guarded on the element's own state: `showModal` on a dialog that is
	// already modal throws, and this effect re-runs whenever the element is
	// bound as well as when `open` moves.
	$effect(() => {
		if (open && element !== null && !element.open) {
			removeFrom = new Set();
			leaveLive = false;
			refusal = null;
			element.showModal();
		} else if (!open) {
			element?.close();
		}
	});

	const leftStanding = $derived(
		bound.filter((mapping) => !removeFrom.has(mapping.inventory)).map((m) => m.inventory)
	);
	const needsConfirmation = $derived(leftStanding.length > 0);
	const blocked = $derived(needsConfirmation && !leaveLive);

	function toggle(inventory: InventoryId, on: boolean) {
		const next = new Set(removeFrom);
		if (on) {
			next.add(inventory);
		} else {
			next.delete(inventory);
		}
		removeFrom = next;
	}

	async function remove() {
		if (sending || blocked) return;
		sending = true;
		refusal = null;
		try {
			await api.deleteProduct(product, {
				remove_from: [...removeFrom],
				leave_live: leaveLive
			});
			onDeleted();
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure
					? failure.message
					: 'Deletion could not be confirmed. Check Resources and Sync before trying again.';
		} finally {
			sending = false;
		}
	}
</script>

<dialog
	bind:this={element}
	aria-labelledby="delete-title"
	onclose={onClose}
	oncancel={(event) => {
		if (sending) event.preventDefault();
	}}
>
	<div class="dialog-body">
		<h2 id="delete-title">Delete “{title}”</h2>
		<p>
			Deleting removes it from your Resources here. Whether it also disappears from a marketplace
			is a separate choice, and it is the irreversible one.
		</p>

		{#if bound.length === 0}
			<p class="quiet">
				Only the Teachouse resource is removed. No marketplace listing will be changed.
			</p>
		{:else}
			{#each bound as mapping (mapping.id)}
				<label class="choice">
					<input
						type="checkbox"
						checked={removeFrom.has(mapping.inventory)}
						disabled={sending}
						onchange={(event) => toggle(mapping.inventory, event.currentTarget.checked)}
					/>
					<span class="t">Also remove from {platformTitle(mapping.inventory)}</span>
					<span class="why bad">
						Removes the marketplace listing, including a live listing. This cannot be undone.
					</span>
				</label>
			{/each}

			{#if needsConfirmation}
				<div class="notice">
					Listings on {leftStanding.map(platformTitle).join(', ')} will remain unchanged.
					After deleting this resource, Teachouse will no longer track them.
				</div>
				<div class="inline-choices" style="margin-top: 10px">
					<label>
						<input
							type="checkbox"
							checked={leaveLive}
							disabled={sending}
							onchange={(event) => (leaveLive = event.currentTarget.checked)}
						/>
						Leave unselected marketplace listings unchanged
					</label>
				</div>
			{/if}
		{/if}

		<Note icon="refresh-cw">Check Updates for each marketplace removal.</Note>

		{#if refusal !== null}
			<p class="refusal">{refusal}</p>
		{/if}

		<div class="actions">
			<button class="btn" type="button" onclick={onClose} disabled={sending}>Keep it</button>
			<button class="btn danger" type="button" onclick={remove} disabled={sending || blocked}>
				{sending ? 'Deleting…' : 'Delete resource'}
			</button>
		</div>
	</div>
</dialog>
