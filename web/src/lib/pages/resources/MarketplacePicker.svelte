<script lang="ts">
	// Where this listing goes, and the licence a chosen marketplace gates.
	// Extracted from `ResourceForm` when the edit path started rendering the
	// same control: the two differ only in what a tick means, so the difference
	// is a prop rather than a second copy of the markup.
	import Field from '$lib/Field.svelte';
	import { AUTHORABLE_PLATFORMS, platformTitle } from '$lib/platforms';
	import type { VocabularyView } from '$lib/api';
	import type { InventoryId } from '$lib/generated/vocab';

	let {
		chosen,
		known,
		refusalOf,
		heldOf,
		licensing,
		licences,
		licence,
		free,
		note = 'Where this listing goes. Choose none to keep it here as a draft and decide later; you can add a marketplace from the resource itself at any time.',
		onToggle,
		onLicence
	}: {
		chosen: readonly InventoryId[];
		known: ReadonlyMap<InventoryId, VocabularyView>;
		/** Why this marketplace cannot carry this listing at all, or `null`. */
		refusalOf: (inventory: InventoryId) => string | null;
		/** Why this marketplace is ticked and cannot be unticked, or `null`.
		 *
		 *  Edit mode is add-only: no route unmaps a marketplace, so one already
		 *  mapped renders ticked and disabled with the reason said, rather than
		 *  offering a control that would silently do nothing. */
		heldOf: (inventory: InventoryId) => string | null;
		licensing: readonly InventoryId[];
		licences: readonly { id: string; label: string }[];
		licence: string | null;
		free: boolean;
		note?: string;
		onToggle: (inventory: InventoryId, on: boolean) => void;
		onLicence: (value: string | null) => void;
	} = $props();
</script>

<div class="res-group res-rail">
	<span class="res-group-label" id="rail-label">Marketplaces</span>
	<span class="res-note">{note}</span>
	<div class="res-picks" role="group" aria-labelledby="rail-label">
		{#each AUTHORABLE_PLATFORMS as inventory (inventory)}
			{@const held = heldOf(inventory)}
			{@const refusal = held === null ? refusalOf(inventory) : null}
			{@const off = held !== null || refusal !== null}
			<label class="res-pick" class:off={refusal !== null}>
				<input
					type="checkbox"
					checked={held !== null || chosen.includes(inventory)}
					disabled={off}
					onchange={(event) => onToggle(inventory, event.currentTarget.checked)}
				/>
				<span class="res-pick-t">{platformTitle(inventory)}</span>
				{#if held !== null}
					<span class="res-pick-why">{held}</span>
				{:else if refusal !== null}
					<span class="res-pick-why bad">{refusal}</span>
				{:else if known.get(inventory)?.authoring.payload_files === 'exactly_one'}
					<span class="res-pick-why">Takes one file.</span>
				{:else}
					<span class="res-pick-why">Takes every file you upload.</span>
				{/if}
			</label>
		{/each}
	</div>

	<!-- Only where a chosen marketplace gates one, because a licence is a grant
	     the seller issues and asking for one nothing will carry invites an
	     answer with no meaning. Nothing is pre-selected: the grant is theirs,
	     which is the same reason the copyright attestation arrives blank. -->
	{#if licensing.length > 0}
		<Field
			label="Licence"
			id="draft-licence"
			required
			hint="{licensing.map(platformTitle).join(' and ')} will not list this without one. It is never chosen for you: the grant is yours to make."
		>
			<select
				id="draft-licence"
				value={licence ?? ''}
				onchange={(event) =>
					onLicence(event.currentTarget.value === '' ? null : event.currentTarget.value)}
			>
				<option value="">Choose a licence</option>
				{#each licences as option (option.id)}
					<option value={option.id}>{option.label}</option>
				{/each}
			</select>
		</Field>
		<p class="res-foot">
			{free
				? 'These are the licences a free listing may carry.'
				: 'These are the licences a paid listing may carry. Ticking Free Resource offers a different set.'}
		</p>
	{/if}
</div>
