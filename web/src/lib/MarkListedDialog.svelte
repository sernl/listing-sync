<script lang="ts">
	import { ApiFailure, api } from '$lib/api';
	import type { InventoryRow } from '$lib/inventory';
	import { AUTHORABLE_PLATFORMS, platformTitle } from '$lib/platforms';
	import type { InventoryId } from '$lib/generated/vocab';
	import Note from '$lib/Note.svelte';

	let {
		open,
		rows,
		onClose,
		onBound
	}: {
		open: boolean;
		/** The selected rows, in the order the table shows them. */
		rows: InventoryRow[];
		onClose: () => void;
		onBound: (bound: number) => void;
	} = $props();

	let element = $state<HTMLDialogElement | null>(null);
	let inventory = $state<InventoryId>(AUTHORABLE_PLATFORMS[0]);
	let urls = $state<Record<string, string>>({});
	let sending = $state(false);
	/** What each item's paste came to, once it has been tried. Kept per item so
	 *  one bad link does not discard the others: a bulk adopt of twelve items
	 *  where one URL is wrong should bind eleven and say which one failed. */
	let refusals = $state<Record<string, string>>({});

	$effect(() => {
		if (open && element !== null && !element.open) {
			element.showModal();
		} else if (!open) {
			element?.close();
		}
	});

	/** Whether this item already binds a listing on the chosen marketplace, in
	 *  which case there is nothing to adopt and the row says so instead of
	 *  offering a field that would be refused. */
	function alreadyBound(row: InventoryRow): boolean {
		return row.mapped.get(inventory)?.binding_state === 'bound';
	}

	const pending = $derived(rows.filter((row) => !alreadyBound(row)));
	const filled = $derived(
		pending.filter((row) => (urls[row.product.id] ?? '').trim().length > 0)
	);

	async function bind() {
		if (filled.length === 0) {
			return;
		}
		sending = true;
		refusals = {};
		let bound = 0;
		const failed: Record<string, string> = {};
		for (const row of filled) {
			const product = row.product.id;
			try {
				// The item may not reach this marketplace at all yet, which is the
				// ordinary case for a catalogue being adopted: the item was authored
				// here and the listing already exists there. Adding the marketplace
				// first is the same catalogue write cross-listing makes.
				const held = row.mapped.get(inventory);
				const mapping = held ?? (await api.addMapping(product, inventory));
				await api.bindMapping(mapping.id, (urls[product] ?? '').trim());
				bound += 1;
			} catch (failure) {
				failed[product] =
					failure instanceof ApiFailure
						? failure.message
						: 'That listing could not be linked.';
			}
		}
		refusals = failed;
		sending = false;
		if (bound > 0) {
			onBound(bound);
		}
	}
</script>

<dialog bind:this={element} aria-labelledby="mark-listed-title" onclose={onClose}>
	<div class="dialog-body">
		<h2 id="mark-listed-title">
			Mark {rows.length} {rows.length === 1 ? 'resource' : 'resources'} as listed
		</h2>
		<p>
			Paste the link to each listing on the marketplace. Nothing on the marketplace changes.
		</p>

		<label class="field">
			<span class="label">Marketplace</span>
			<select bind:value={inventory} disabled={sending}>
				{#each AUTHORABLE_PLATFORMS as option (option)}
					<option value={option}>{platformTitle(option)}</option>
				{/each}
			</select>
		</label>

		{#each rows as row (row.product.id)}
			<div class="choice">
				<span class="t">{row.product.title}</span>
				{#if alreadyBound(row)}
					<span class="why ok">already linked here</span>
				{:else}
					<input
						type="url"
						placeholder="https://…"
						disabled={sending}
						value={urls[row.product.id] ?? ''}
						oninput={(event) => (urls = { ...urls, [row.product.id]: event.currentTarget.value })}
					/>
					{#if refusals[row.product.id]}
						<span class="why bad">{refusals[row.product.id]}</span>
					{/if}
				{/if}
			</div>
		{/each}

		<Note icon="triangle-alert">Check each link. A wrong link connects the wrong listing.</Note>

		<div class="actions">
			<button class="btn" type="button" onclick={onClose} disabled={sending}>Cancel</button>
			<button class="cta" type="button" onclick={bind} disabled={sending || filled.length === 0}>
				{sending ? 'Linking…' : filled.length === 0 ? 'Link' : `Link ${filled.length}`}
			</button>
		</div>
	</div>
</dialog>

<style>
	.choice {
		display: flex;
		flex-direction: column;
		gap: 4px;
		padding: 8px 0;
		border-top: 1px solid var(--line);
	}

	.choice .t {
		font-weight: 600;
	}

	.choice input {
		width: 100%;
	}

	.why {
		font-size: 12px;
	}

	.why.ok {
		color: var(--muted);
	}

	.why.bad {
		color: var(--bad-ink);
	}
</style>
