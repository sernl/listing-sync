<script lang="ts">
	// Where this listing goes: one tile per marketplace, ticked or not.
	//
	// Tiles rather than a list of named rows, and one Tes rather than three,
	// because that is the founder's rule of 2026-09-11: the mark is what a
	// teacher recognises the place by, the full name is on hover and read out,
	// and which Tes catalogue a listing reaches is the Tes panel's question.
	import { MARKETPLACE_TILES, MARK_SRC } from '$lib/platforms';
	import type { Marketplace } from '$lib/generated/vocab';

	let {
		chosen,
		refusalOf,
		heldOf,
		onToggle
	}: {
		chosen: readonly Marketplace[];
		/** Why this marketplace cannot carry this listing, or `null`. */
		refusalOf: (marketplace: Marketplace) => string | null;
		/** Why this marketplace is ticked and cannot be unticked, or `null`.
		 *
		 *  Edit mode is add-only: no route unmaps a marketplace, so one already
		 *  mapped renders ticked and disabled with the reason said, rather than
		 *  offering a control that would silently do nothing. */
		heldOf: (marketplace: Marketplace) => string | null;
		onToggle: (marketplace: Marketplace, on: boolean) => void;
	} = $props();

	const base = $props.id();
</script>

<div class="res-tiles" role="group" aria-label="Marketplaces">
	{#each MARKETPLACE_TILES as tile (tile.marketplace)}
		{@const held = heldOf(tile.marketplace)}
		{@const refusal = held === null ? refusalOf(tile.marketplace) : null}
		{@const why = held ?? refusal}
		{@const id = `${base}-${tile.marketplace}`}
		<!-- The reason is the tooltip rather than a line under the tile: the
		     grid is evenly spaced and a sentence under one tile would make that
		     column taller than the rest. -->
		<label class="res-tile" class:off={why !== null} for={id} title={why ?? tile.name}>
			<input
				{id}
				type="checkbox"
				checked={held !== null || chosen.includes(tile.marketplace)}
				disabled={why !== null}
				onchange={(event) => onToggle(tile.marketplace, event.currentTarget.checked)}
			/>
			<img class="res-tile-mark" src={MARK_SRC[tile.marketplace]} alt="" />
			<span class="sr-only">{tile.name}{why === null ? '' : ` — ${why}`}</span>
		</label>
	{/each}
</div>
