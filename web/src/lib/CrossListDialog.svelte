<script lang="ts">
	import { ApiFailure, api, type PublishIntent } from '$lib/api';
	import { bulkTarget, type InventoryRow } from '$lib/inventory';
	import { AUTHORABLE_PLATFORMS, platformTitle } from '$lib/platforms';
	import type { InventoryId } from '$lib/generated/vocab';

	let {
		open,
		rows,
		onClose,
		onStarted
	}: {
		open: boolean;
		/** The selected rows, in the order the table shows them. */
		rows: InventoryRow[];
		onClose: () => void;
		onStarted: (runs: { inventory: InventoryId; job: string }[]) => void;
	} = $props();

	let element = $state<HTMLDialogElement | null>(null);
	let intent = $state<PublishIntent>('draft');
	let ticked = $state<Set<InventoryId>>(new Set());
	let sending = $state(false);
	let refusal = $state<string | null>(null);

	// One key per marketplace per intent, minted once: a retry after a failure
	// reuses it, so the retry and the double-click are the same job.
	let keys: Record<string, string> = {};

	$effect(() => {
		if (open && element !== null && !element.open) {
			element.showModal();
		} else if (!open) {
			element?.close();
		}
	});

	const targets = $derived(
		AUTHORABLE_PLATFORMS.map((inventory) => bulkTarget(rows, inventory))
	);

	function toggle(inventory: InventoryId, on: boolean) {
		const next = new Set(ticked);
		if (on) {
			next.add(inventory);
		} else {
			next.delete(inventory);
		}
		ticked = next;
	}

	async function send() {
		const chosen = targets.filter(
			(target) => ticked.has(target.inventory) && target.mappings.length > 0
		);
		if (chosen.length === 0) {
			return;
		}
		sending = true;
		refusal = null;
		const runs: { inventory: InventoryId; job: string }[] = [];
		try {
			// One job per marketplace, because a job carries exactly one
			// inventory. Sequential so a refusal names the marketplace it belongs
			// to and the ones before it stay enqueued.
			for (const target of chosen) {
				const key = (keys[`${target.inventory}:${intent}`] ??= crypto.randomUUID());
				const created = await api.createJob(target.inventory, target.mappings, key, intent);
				runs.push({ inventory: target.inventory, job: created.job });
			}
			keys = {};
			ticked = new Set();
			onStarted(runs);
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure
					? failure.message
					: 'The send did not start. Nothing was enqueued twice; try again.';
			if (runs.length > 0) {
				onStarted(runs);
			}
		} finally {
			sending = false;
		}
	}
</script>

<dialog bind:this={element} aria-labelledby="cross-list-title" onclose={onClose}>
	<div class="dialog-body">
		<h2 id="cross-list-title">Cross-list {rows.length} {rows.length === 1 ? 'item' : 'items'}</h2>
		<p>
			Pick where this send goes. Each line says how many of the selected items that
			marketplace already carries a mapping for, counted from your own records without contacting
			anyone.
		</p>

		<div class="inline-choices">
			<label>
				<input
					type="radio"
					name="cross-list-intent"
					checked={intent === 'draft'}
					onchange={() => (intent = 'draft')}
				/>
				Leave as drafts
			</label>
			<label>
				<input
					type="radio"
					name="cross-list-intent"
					checked={intent === 'live'}
					onchange={() => (intent = 'live')}
				/>
				Publish live
			</label>
		</div>
		<p class="foot-note">
			Draft is the default. A live publish on a Tes site cannot be reversed by us: neither editing
			a published listing nor taking one back to draft is a transition we have captured.
		</p>

		{#each targets as target (target.inventory)}
			<label class="choice {target.mappings.length === 0 ? 'off' : ''}">
				<input
					type="checkbox"
					checked={ticked.has(target.inventory)}
					disabled={sending || target.mappings.length === 0}
					onchange={(event) => toggle(target.inventory, event.currentTarget.checked)}
				/>
				<span class="t">{platformTitle(target.inventory)}</span>
				<span class="why {target.mappings.length === 0 ? 'bad' : 'ok'}">
					{#if target.mappings.length === 0}
						none of the selected items is mapped here
					{:else if target.skipped === 0}
						{target.mappings.length} of {rows.length}
					{:else}
						{target.mappings.length} of {rows.length}; {target.skipped} passed over, not mapped here
					{/if}
				</span>
			</label>
		{/each}

		<p class="foot-note">
			An item that is not mapped to a marketplace is passed over rather than added to it:
			marketplaces are chosen when a draft is created, and there is no endpoint yet that adds one
			afterwards. Work for Tes and TPT runs on your own device, so a send waits while that device
			is off.
		</p>

		{#if refusal !== null}
			<p class="refusal">{refusal}</p>
		{/if}

		<div class="actions">
			<button class="btn" type="button" onclick={onClose} disabled={sending}>Cancel</button>
			<button class="cta" type="button" onclick={send} disabled={sending || ticked.size === 0}>
				{sending ? 'Starting…' : ticked.size === 0 ? 'Send' : `Send to ${ticked.size}`}
			</button>
		</div>
	</div>
</dialog>
