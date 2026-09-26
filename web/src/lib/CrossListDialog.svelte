<script lang="ts">
	import { ApiFailure, api, type PublishIntent } from '$lib/api';
	import { bulkTarget, type BulkTarget, type InventoryRow } from '$lib/inventory';
	import { AUTHORABLE_PLATFORMS, platformTitle } from '$lib/platforms';
	import type { InventoryId } from '$lib/generated/vocab';
	import { MAPPINGS_HREF, PRICING_HREF } from '$lib/pages/automations/seller-rules';
	import Note from '$lib/Note.svelte';

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

	// The mappings this dialog minted, keyed by item and marketplace. A send
	// that added mappings and then failed to enqueue must not add them again
	// on the retry: the server refuses a second add, so the retry would fail
	// on work the first attempt already did.
	let added: Record<string, string> = {};

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

	/** Every mapping the send needs on one marketplace, mapping the items that
	 *  lack one first. The add is the ordinary catalogue write: it mints an
	 *  unbound mapping and contacts nobody, so the job that follows travels the
	 *  path a mapping chosen at create time travels. */
	async function mappingsFor(target: BulkTarget): Promise<string[]> {
		const minted: string[] = [];
		for (const product of target.unmapped) {
			const held = added[`${product}:${target.inventory}`];
			if (held !== undefined) {
				minted.push(held);
				continue;
			}
			const mapping = await api.addMapping(product, target.inventory);
			added[`${product}:${target.inventory}`] = mapping.id;
			minted.push(mapping.id);
		}
		return [...target.mappings, ...minted];
	}

	async function send() {
		const chosen = targets.filter(
			(target) =>
				ticked.has(target.inventory) &&
				target.mappings.length + target.unmapped.length > 0
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
				const mappings = await mappingsFor(target);
				const key = (keys[`${target.inventory}:${intent}`] ??= crypto.randomUUID());
				const created = await api.createJob(target.inventory, mappings, key, intent);
				runs.push({ inventory: target.inventory, job: created.job });
			}
			keys = {};
			added = {};
			ticked = new Set();
			onStarted(runs);
		} catch (failure) {
			refusal = refusalOf(failure);
			if (runs.length > 0) {
				onStarted(runs);
			}
		} finally {
			sending = false;
		}
	}

	function refusalOf(failure: unknown): string {
		if (!(failure instanceof ApiFailure)) {
			return 'Sending did not start. Try again; nothing will be sent twice.';
		}
		if (failure.code() === 'mapping_already_exists') {
			return 'One of these resources is already on that marketplace. Reload the page and try again.';
		}
		return failure.message;
	}
</script>

<dialog bind:this={element} aria-labelledby="cross-list-title" onclose={onClose}>
	<div class="dialog-body">
		<h2 id="cross-list-title">Cross-list {rows.length} {rows.length === 1 ? 'resource' : 'resources'}</h2>
		<p>
			Pick where to send these. Each line shows how many are already set up there and how many
			will be added.
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
		<Note icon="triangle-alert">You can't undo a live publish to Tes from here.</Note>

		{#each targets as target (target.inventory)}
			{@const reaches = target.mappings.length + target.unmapped.length}
			<label class="choice {reaches === 0 ? 'off' : ''}">
				<input
					type="checkbox"
					checked={ticked.has(target.inventory)}
					disabled={sending || reaches === 0}
					onchange={(event) => toggle(target.inventory, event.currentTarget.checked)}
				/>
				<span class="t">{platformTitle(target.inventory)}</span>
				<span class="why {reaches === 0 ? 'bad' : 'ok'}">
					{#if reaches === 0}
						nothing selected
					{:else if target.unmapped.length === 0}
						{target.mappings.length} of {rows.length}, all already set up here
					{:else if target.mappings.length === 0}
						{target.unmapped.length} of {rows.length}, each set up here first
					{:else}
						{target.mappings.length} of {rows.length} already set up here; {target.unmapped.length}
						set up first
					{/if}
				</span>
			</label>
		{/each}

		<Note>Any resource not yet set up for a marketplace is set up first.</Note>
		<Note icon="laptop">Sending to Tes and TPT waits until your computer is on.</Note>
		<Note icon="files"
			>See your files and which computer has them on <a href="/resources/files">Your machines' files</a>.</Note
		>
		<!-- The price and the terms a listing arrives with are decided on their
		     own screens, beside a preview of what each resource would get: this
		     send carries whatever has been approved for these resources, and a
		     send is not the place to choose it blind. A rule ticked for
		     Cross-list applies here without being approved row by row; nothing
		     else does. -->
		<Note icon="tag"
			>Set prices on <a href={PRICING_HREF}>Target prices</a> and licence and type on
			<a href={MAPPINGS_HREF}>Target terms</a>.</Note
		>

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
