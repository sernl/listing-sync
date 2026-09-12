<script lang="ts">
	// The board's own add-to-collection verb: pick a collection the seller
	// already has, or make one, and put the selection in it.
	//
	// The write is `PUT /v1/collections/{id}/members`, which takes the whole
	// ordered set, so the existing members are read first and the selection is
	// appended to them. Sending the selection alone would empty the collection
	// of everything else in it, which is the one mistake this dialog must not
	// make; `added` is where that arithmetic lives and is tested.

	import { ApiFailure, collectionsApi, type CollectionHead } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import type { InventoryRow } from '$lib/inventory';
	import {
		DESCRIPTION_MAX_CHARS,
		NAME_MAX_CHARS,
		added,
		checkName,
		countLine
	} from '$lib/pages/collections/collections';
	import '$lib/pages/collections/collections.css';

	let {
		open,
		rows,
		onClose,
		onAdded
	}: {
		open: boolean;
		/** The selected rows, in the order the board shows them: that order is
		 *  the order they join the collection in. */
		rows: InventoryRow[];
		onClose: () => void;
		/** The collection they went into and how many were added, so the board
		 *  says which set grew rather than "done". */
		onAdded: (into: string, count: number) => void;
	} = $props();

	let element = $state<HTMLDialogElement | null>(null);
	let collections = $state<CollectionHead[]>([]);
	let unread = $state(false);
	let reading = $state(false);
	/** The collection chosen, or `new` while the seller is making one. Null
	 *  before either: a dialog that preselected the first collection would
	 *  hand a seller a set they did not pick. */
	let chosen = $state<string | 'new' | null>(null);
	let name = $state('');
	let description = $state('');
	let sending = $state(false);
	let refusal = $state<string | null>(null);

	$effect(() => {
		if (open && element !== null && !element.open) {
			element.showModal();
		} else if (!open) {
			element?.close();
		}
	});

	// Read on opening rather than at mount: the board holds this dialog for
	// every visit, and a collection made on the Collections page between two
	// opens has to be offered here.
	$effect(() => {
		if (!open) {
			return;
		}
		reading = true;
		refusal = null;
		void collectionsApi
			.list()
			.then((view) => {
				collections = view.collections;
				unread = false;
				chosen = view.collections.length === 0 ? 'new' : chosen;
			})
			.catch(() => {
				collections = [];
				unread = true;
			})
			.finally(() => {
				reading = false;
			});
	});

	const taken = $derived(collections.map((collection) => collection.name));
	const verdict = $derived(checkName(name, taken));
	const products = $derived(rows.map((row) => row.product.id));

	/** Why the confirm cannot run, or undefined where it can. Stated on the
	 *  control, because a disabled control with no reason reads as a fault. */
	const blocked = $derived.by(() => {
		if (sending) {
			return 'The collection is being written.';
		}
		if (products.length === 0) {
			return 'Select at least one resource that is in view.';
		}
		if (chosen === null) {
			return 'Choose a collection, or make one.';
		}
		if (chosen === 'new' && !verdict.accepted) {
			return verdict.message;
		}
		return undefined;
	});

	async function confirm() {
		if (chosen === null || blocked !== undefined) {
			return;
		}
		sending = true;
		refusal = null;
		try {
			// A new collection is created first and then filled, because the
			// create route takes a name and a description and nothing else: the
			// membership is one write in both arms, so the fill cannot be
			// skipped for the new one.
			const into =
				chosen === 'new'
					? await collectionsApi.create({
							name: verdict.accepted ? verdict.name : name.trim(),
							description: description.trim().length === 0 ? null : description.trim()
						})
					: collections.find((collection) => collection.id === chosen);
			if (into === undefined) {
				refusal = 'That collection is no longer one of yours. Close this and open it again.';
				return;
			}
			// The stored order, read now rather than remembered from the list:
			// the list carries a count and not the members, and appending to a
			// set this dialog guessed at would drop whatever it guessed wrong.
			const held = chosen === 'new' ? [] : (await collectionsApi.get(into.id)).members;
			const before = held.length;
			const next = added(
				held.map((member) => member.product),
				products
			);
			await collectionsApi.setMembers(into.id, next);
			name = '';
			description = '';
			onAdded(into.name, next.length - before);
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure
					? failure.message
					: 'The collection was not written. Nothing has been added.';
		} finally {
			sending = false;
		}
	}
</script>

<dialog
	class="coll-dialog"
	bind:this={element}
	aria-labelledby="add-to-collection-title"
	onclose={onClose}
>
	<div class="dialog-body">
		<h2 id="add-to-collection-title">
			Add {rows.length} {rows.length === 1 ? 'resource' : 'resources'} to a collection
		</h2>
		<p>
			A collection is an ordered set of your resources. These join the end of the one you
			pick, in the order the board has them; a resource the collection already holds keeps its
			place.
		</p>

		{#if reading}
			<p class="foot-note">Reading your collections…</p>
		{:else if unread}
			<p class="refusal">
				Your collections could not be read, so there is none to pick. Making one below still
				works.
			</p>
		{/if}

		<div class="mk-tiles" role="radiogroup" aria-label="Which collection">
			{#each collections as collection (collection.id)}
				<button
					type="button"
					class="mk-tile"
					class:on={chosen === collection.id}
					role="radio"
					aria-checked={chosen === collection.id}
					disabled={sending}
					onclick={() => (chosen = collection.id)}
				>
					<span aria-hidden="true">{chosen === collection.id ? '●' : '○'}</span>
					<span>
						{collection.name}
						<span class="mk-tile-why">
							{countLine(collection.count)}{collection.description === null
								? ''
								: ` · ${collection.description}`}
						</span>
					</span>
				</button>
			{/each}
			<button
				type="button"
				class="mk-tile"
				class:on={chosen === 'new'}
				role="radio"
				aria-checked={chosen === 'new'}
				disabled={sending}
				onclick={() => (chosen = 'new')}
			>
				<span aria-hidden="true">{chosen === 'new' ? '●' : '○'}</span>
				<span>
					A new collection
					<span class="mk-tile-why">Named below, holding these resources.</span>
				</span>
			</button>
		</div>

		{#if chosen === 'new'}
			<Field label="Name" id="add-collection-name" required>
				<input
					id="add-collection-name"
					type="text"
					maxlength={NAME_MAX_CHARS}
					placeholder="Autumn term"
					disabled={sending}
					bind:value={name}
				/>
			</Field>
			<Field
				label="Description"
				id="add-collection-description"
				hint="Yours alone; it reaches no marketplace."
			>
				<textarea
					id="add-collection-description"
					rows="2"
					maxlength={DESCRIPTION_MAX_CHARS}
					disabled={sending}
					bind:value={description}
				></textarea>
			</Field>
			{#if name.trim().length > 0 && !verdict.accepted}
				<p class="refusal">{verdict.message}</p>
			{/if}
		{/if}

		{#if refusal !== null}
			<p class="refusal">{refusal}</p>
		{/if}

		<div class="actions">
			<Button onclick={onClose} disabled={sending} reason={sending ? 'Writing.' : undefined}>
				Cancel
			</Button>
			<Button
				tier="additive"
				disabled={blocked !== undefined}
				reason={blocked}
				onclick={() => void confirm()}
			>
				{sending ? 'Adding…' : `Add ${rows.length}`}
			</Button>
		</div>
	</div>
</dialog>
