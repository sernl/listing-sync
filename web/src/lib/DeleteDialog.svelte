<script lang="ts">
	import { ApiFailure, api, type MappingHead } from '$lib/api';
	import { platformTitle } from '$lib/platforms';
	import { standingOf } from '$lib/tes-portfolio';
	import type { InventoryId } from '$lib/generated/vocab';

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
	let seeded = false;
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
			element.showModal();
		} else if (!open) {
			element?.close();
		}
	});

	// Removing everywhere is the default, because a local delete that leaves
	// bound mappings behind leaves live listings nobody tracks.
	$effect(() => {
		if (open && !seeded && bound.length > 0) {
			seeded = true;
			removeFrom = new Set(bound.map((mapping) => mapping.inventory));
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
					: 'The listing was not deleted. Nothing was removed anywhere.';
		} finally {
			sending = false;
		}
	}
</script>

<dialog bind:this={element} aria-labelledby="delete-title" onclose={onClose}>
	<div class="dialog-body">
		<h2 id="delete-title">Delete “{title}”</h2>
		<p>
			Deleting removes it from your catalogue here. Whether it also disappears from a marketplace
			is a separate choice, and it is the irreversible one.
		</p>

		{#if bound.length === 0}
			<p class="quiet">
				This listing is not on any marketplace yet, so only your own copy is deleted.
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
					<span class="t">{platformTitle(mapping.inventory)}</span>
					<span class="why bad">
						Deletes the live listing there. We cannot undo that, and the sales history behind it
						belongs to the marketplace.
					</span>
				</label>
			{/each}

			{#if needsConfirmation}
				<div class="notice">
					{leftStanding.map(platformTitle).join(', ')} would keep a listing that nothing here tracks
					any more.
				</div>
				<div class="inline-choices" style="margin-top: 10px">
					<label>
						<input
							type="checkbox"
							checked={leaveLive}
							disabled={sending}
							onchange={(event) => (leaveLive = event.currentTarget.checked)}
						/>
						Leave those listings alone, and delete only my copy
					</label>
				</div>
			{/if}
		{/if}

		<p class="foot-note">
			Each removal runs as its own job. Whether it succeeded shows on that run, alongside every
			other write.
		</p>

		{#if refusal !== null}
			<p class="refusal">{refusal}</p>
		{/if}

		<div class="actions">
			<button class="btn" type="button" onclick={onClose} disabled={sending}>Keep it</button>
			<button class="btn danger" type="button" onclick={remove} disabled={sending || blocked}>
				{sending ? 'Deleting…' : 'Delete listing'}
			</button>
		</div>
	</div>
</dialog>
