<script lang="ts">
	import {
		ApiFailure,
		api,
		type ConnectionView,
		type InventoryStatus,
		type MappingHead,
		type PublishIntent,
		type VocabularyView
	} from '$lib/api';
	import { connectionFor, readinessOf } from '$lib/publish-readiness';
	import type { InventoryId } from '$lib/generated/vocab';
	import Note from '$lib/Note.svelte';

	let {
		open,
		mappings,
		connections,
		statuses,
		vocabularies,
		payloadFiles,
		hasRights,
		onClose,
		onPublished
	}: {
		open: boolean;
		mappings: MappingHead[];
		connections: ConnectionView[];
		statuses: InventoryStatus[];
		vocabularies: ReadonlyMap<InventoryId, VocabularyView>;
		payloadFiles: number;
		hasRights: boolean;
		onClose: () => void;
		onPublished: (runs: { inventory: InventoryId; job: string }[]) => void;
	} = $props();

	let element = $state<HTMLDialogElement | null>(null);
	let intent = $state<PublishIntent>('draft');
	let ticked = $state<Set<string>>(new Set());
	let sending = $state(false);
	let refusal = $state<string | null>(null);

	// One key per platform per attempt, minted once: a retry after a failure
	// reuses it, so the retry and the double-click are the same job on the
	// server rather than two writes against one listing.
	let keys: Record<string, string> = {};

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

	const rows = $derived(
		mappings.map((mapping) => ({
			mapping,
			readiness: readinessOf({
				inventory: mapping.inventory,
				intent,
				mapping,
				connection: connectionFor(mapping.inventory, connections),
				status: statuses.find((status) => status.inventory === mapping.inventory),
				vocabulary: vocabularies.get(mapping.inventory),
				payloadFiles,
				hasRights
			})
		}))
	);

	function toggle(id: string, on: boolean) {
		const next = new Set(ticked);
		if (on) {
			next.add(id);
		} else {
			next.delete(id);
		}
		ticked = next;
	}

	async function publish() {
		const chosen = mappings.filter((mapping) => ticked.has(mapping.id));
		if (chosen.length === 0) {
			return;
		}
		sending = true;
		refusal = null;
		const runs: { inventory: InventoryId; job: string }[] = [];
		try {
			// One job per platform, because a job carries exactly one inventory.
			// Sequential rather than concurrent so a refusal names the platform it
			// belongs to and the ones before it stay enqueued.
			for (const mapping of chosen) {
				const key = (keys[`${mapping.id}:${intent}`] ??= crypto.randomUUID());
				const created = await api.createJob(mapping.inventory, [mapping.id], key, intent);
				runs.push({ inventory: mapping.inventory, job: created.job });
			}
			keys = {};
			ticked = new Set();
			onPublished(runs);
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure
					? failure.message
					: 'Sending did not start. Try again; nothing will be sent twice.';
			if (runs.length > 0) {
				onPublished(runs);
			}
		} finally {
			sending = false;
		}
	}
</script>

<dialog bind:this={element} aria-labelledby="publish-title" onclose={onClose}>
	<div class="dialog-body">
		<h2 id="publish-title">Send to marketplaces</h2>
		<p>
			Pick where to send this listing.
		</p>

		<div class="inline-choices">
			<label>
				<input
					type="radio"
					name="publish-intent"
					checked={intent === 'draft'}
					onchange={() => (intent = 'draft')}
				/>
				Leave as a draft
			</label>
			<label>
				<input
					type="radio"
					name="publish-intent"
					checked={intent === 'live'}
					onchange={() => (intent = 'live')}
				/>
				Publish live
			</label>
		</div>
		<Note icon="triangle-alert">You can't undo a live publish to Tes from here.</Note>

		{#each rows as row (row.mapping.id)}
			<label class="choice">
				<input
					type="checkbox"
					checked={ticked.has(row.mapping.id)}
					disabled={sending}
					onchange={(event) => toggle(row.mapping.id, event.currentTarget.checked)}
				/>
				<span class="t">{row.readiness.title}</span>
				<span class="why {row.readiness.tone}">{row.readiness.line}</span>
			</label>
		{:else}
			<p class="quiet">This listing isn't set up for any marketplace, so there is nothing to send.</p>
		{/each}

		<Note>You can still pick a marketplace that isn't ready. We check it again before sending.</Note>

		{#if refusal !== null}
			<p class="refusal">{refusal}</p>
		{/if}

		<div class="actions">
			<button class="btn" type="button" onclick={onClose} disabled={sending}>Cancel</button>
			<button class="cta" type="button" onclick={publish} disabled={sending || ticked.size === 0}>
				{sending ? 'Starting…' : ticked.size === 0 ? 'Send' : `Send to ${ticked.size}`}
			</button>
		</div>
	</div>
</dialog>
