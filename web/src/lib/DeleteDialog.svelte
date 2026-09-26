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
					: "We couldn't confirm the delete. Check Resources and Updates before you try again.";
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
			This removes it from your Resources. Removing it from a marketplace is a separate choice, and
			it can't be undone.
		</p>

		{#if bound.length === 0}
			<p class="quiet">
				Only the Teachouse resource is deleted. Your marketplace listings stay as they are.
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
						Removes the listing there, even if it is live. This can't be undone.
					</span>
				</label>
			{/each}

			{#if needsConfirmation}
				<div class="notice">
					Your listings on {leftStanding.map(platformTitle).join(', ')} stay as they are.
					Teachouse stops tracking them once this resource is deleted.
				</div>
				<div class="inline-choices" style="margin-top: 10px">
					<label>
						<input
							type="checkbox"
							checked={leaveLive}
							disabled={sending}
							onchange={(event) => (leaveLive = event.currentTarget.checked)}
						/>
						Leave your other marketplace listings as they are
					</label>
				</div>
			{/if}
		{/if}

		<Note icon="refresh-cw">Follow each marketplace removal on Updates.</Note>

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
