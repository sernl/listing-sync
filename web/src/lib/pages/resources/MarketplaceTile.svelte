<script lang="ts">
	import Button from '$lib/Button.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { external } from '$lib/external';
	import { PILL_TONE } from './list';
	import type { MarketplaceTileView } from './marketplace-tile';

	let {
		tile,
		adding = false,
		addingHere = false,
		attachOpen = false,
		attachSending = false,
		oncrosslist,
		onattach
	}: {
		tile: MarketplaceTileView;
		/** Whether any marketplace is being added, which is what disables every
		 *  cross-list control at once. */
		adding?: boolean;
		/** Whether this one is the marketplace being added. */
		addingHere?: boolean;
		/** Whether this tile's attach form is the one open. */
		attachOpen?: boolean;
		attachSending?: boolean;
		oncrosslist?: () => void;
		onattach?: () => void;
	} = $props();
</script>

<!-- The label sits on the anchor where there is one and on the tile where
     there is not, so it is announced once either way. A tile with no
     destination is a group rather than a link: it says where the resource
     stands and offers nowhere to go, and an anchor that leads back to this
     same page would say otherwise. -->
<div
	class="tile"
	class:quiet={tile.control === 'crosslist'}
	class:held={tile.paused !== null}
	title={tile.name}
	role={tile.href === null ? 'group' : undefined}
	aria-label={tile.href === null ? tile.name : undefined}
>
	<div class="top">
		<img class="mark" src={tile.markSrc} alt="" width="28" height="28" />
	</div>

	{#if tile.href === null}
		<StatusPill tone={PILL_TONE[tile.tone]} label={tile.label} />
	{:else}
		<a
			class="go"
			href={tile.href}
			target={tile.external ? '_blank' : undefined}
			rel={tile.external ? 'noopener noreferrer' : undefined}
			aria-label={tile.name}
			use:external
		>
			<StatusPill tone={PILL_TONE[tile.tone]} label={tile.label} />
		</a>
	{/if}

	<p class="say">{tile.detail}</p>

	{#if tile.control !== null}
		<div class="foot">
			{#if tile.control === 'crosslist'}
				<Button
					disabled={adding}
					reason={adding ? 'A marketplace is being added.' : undefined}
					onclick={() => oncrosslist?.()}
				>
					{addingHere ? 'Adding…' : 'Cross-list'}
				</Button>
			{:else}
				<Button
					disabled={attachSending}
					reason={attachSending ? 'A listing is being attached.' : undefined}
					onclick={() => onattach?.()}
				>
					{attachOpen ? 'Cancel' : 'Mark listed'}
				</Button>
			{/if}
		</div>
	{/if}
</div>

<style>
	/* `position: relative` is what the tile's link stretches against, and the
	   `min-width` is what stops a long clause widening the grid column past its
	   share: a track sized `1fr` still takes its item's own minimum content
	   width unless the item says otherwise, which is how a sentence arriving
	   where a word was expected pushes a whole row off the panel. */
	.tile {
		position: relative;
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 7px;
		min-width: 0;
		background: var(--surface);
		border: 1px solid var(--line);
		border-radius: var(--r-panel);
		box-shadow: var(--sh-1);
		padding: 12px;
	}

	/* A marketplace this resource does not reach yet, drawn as the board draws
	   the same fact: present, so that cross-listing onto it is one tap away,
	   and quiet, so it does not read as somewhere the listing already is. */
	.quiet {
		border-style: dashed;
		box-shadow: none;
		background: none;
	}

	/* Paused sending keeps the marketplace's own state and says so separately,
	   as the board's chips do: the listing stands where it stood and only the
	   sending is held, so this is a mark on the tile rather than a ninth
	   status word. */
	.held {
		box-shadow: inset 0 -3px 0 0 var(--warn);
	}

	/* Pinned to the mark's own height, which is the whole of this row's
	   content. Measured rather than assumed: without it the row draws 53px
	   around a 28px mark and its only element child is that mark, so the
	   extra 25px belongs to no box the probe can name. That is 25px of dead
	   space above every tile's status, on the surface the founder asked to
	   shrink. */
	.top {
		display: flex;
		align-items: center;
		gap: 7px;
		height: 28px;
		min-width: 0;
	}

	/* The marketplace's own mark in place of its name, which is the founder's
	   "icons instead of typing out the names". No alternative text, because
	   the tile's accessible name already spells the platform out. */
	.mark {
		width: 28px;
		height: 28px;
		object-fit: contain;
		border-radius: 6px;
		flex: none;
	}

	/* The whole tile is one target rather than four, so a thumb landing
	   anywhere inside it goes where the status says. The overlay is on the
	   anchor and not on the tile, because the tile also carries a control that
	   must stay its own target, and that control is raised back above this by
	   `position`. */
	.go {
		text-decoration: none;
		color: inherit;
	}

	.go::after {
		content: '';
		position: absolute;
		inset: 0;
		border-radius: var(--r-panel);
	}

	/* The hover is painted by the overlay rather than by the tile beneath it,
	   so the whole target and the thing that reacts to it are one element. */
	.go:hover::after {
		background: color-mix(in srgb, var(--text) 4%, transparent);
	}

	.go:focus-visible::after {
		outline: 2px solid var(--accent);
		outline-offset: 2px;
	}

	.say {
		margin: 0;
		font-size: 11.5px;
		line-height: 1.35;
		color: var(--muted);
		min-width: 0;
		overflow-wrap: anywhere;
	}

	/* Above the tile's own link overlay, so the control is pressed rather than
	   the tile beneath it, and at the foot however tall the clause above it
	   ran. The control's own height is the shared token's, from the button. */
	.foot {
		position: relative;
		margin-top: auto;
		padding-top: 2px;
	}
</style>
