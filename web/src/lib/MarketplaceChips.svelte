<script lang="ts">
	import { external } from '$lib/external';
	import type { MarketplaceChip } from '$lib/inventory';
	import { MARK_SRC, REGION_TAG, platformTitle } from '$lib/platforms';
	import { MARKETPLACE_OF } from '$lib/listings-view';

	let { chips }: { chips: MarketplaceChip[] } = $props();

	/** How many chips a phone shows before the marker. Two rather than three,
	 *  and the mark did not buy back the third: a chip is as wide as its state
	 *  word, and at 390 the widest pair measured 66 and 94 pixels, which with
	 *  the marker fills the strip. Four ghost chips are 28 each and fit easily,
	 *  but a cap that holds only for the narrow states is not a cap. The strip
	 *  still wraps, so a third chip takes a second row inside the card rather
	 *  than leaving it. */
	const PHONE_CAP = 2;

	// Expanding is one way on purpose: the seller asked to see the rest, and
	// taking them away again on a second tap serves nobody.
	let expanded = $state(false);
	const withheld = $derived(Math.max(0, chips.length - PHONE_CAP));

	/** The whole sentence a screen reader and a hover both get, so the state is
	 *  never carried by colour alone. */
	function described(chip: MarketplaceChip): string {
		const paused = chip.paused === null ? '' : ` Sending is paused here: ${chip.paused}`;
		const opens = chip.action?.external === true ? ' Opens the listing on the marketplace.' : '';
		return `${platformTitle(chip.inventory)}: ${chip.label}. ${chip.detail}${paused}${opens}`;
	}
</script>

<span class="strip" class:open={expanded}>
	{#each chips as chip, index (chip.inventory)}
		{#if chip.action?.external === true}
			<a
				class="mk {chip.tone}"
				class:withheld={index >= PHONE_CAP}
				class:paused={chip.paused !== null}
				href={chip.action.href}
				target="_blank"
				rel="noopener noreferrer"
				title={described(chip)}
				aria-label={described(chip)}
				use:external
			>
				<img
					class="mark"
					src={MARK_SRC[MARKETPLACE_OF[chip.inventory]]}
					alt=""
					width="14"
					height="14"
				/>{#if REGION_TAG[chip.inventory] !== null}<b>{REGION_TAG[chip.inventory]}</b>{/if}
				<i>{chip.label}</i>
			</a>
		{:else}
			<span
				class="mk {chip.tone}"
				class:withheld={index >= PHONE_CAP}
				class:ghost={chip.state === 'not_listed'}
				class:paused={chip.paused !== null}
				title={described(chip)}
				aria-label={described(chip)}
			>
				<img
					class="mark"
					src={MARK_SRC[MARKETPLACE_OF[chip.inventory]]}
					alt=""
					width="14"
					height="14"
				/>{#if REGION_TAG[chip.inventory] !== null}<b>{REGION_TAG[chip.inventory]}</b>{/if}
				{#if chip.state !== 'not_listed'}<i>{chip.label}</i>{/if}
			</span>
		{/if}
	{/each}

	<!-- Drawn only below the phone breakpoint, by the sheet rather than by a
	     width read here: above it the row has the space for every chip, and a
	     count is a worse answer than the thing counted. -->
	{#if withheld > 0 && !expanded}
		<button
			class="more"
			type="button"
			onclick={() => (expanded = true)}
			aria-label={`Show ${withheld} more ${withheld === 1 ? 'marketplace' : 'marketplaces'}`}
		>
			+{withheld}
		</button>
	{/if}
</span>

<style>
	/* A wrapping flex row rather than an unbreakable inline one. The caller
	   wraps this in a `.strip` of its own that already wraps, and an inline-flex
	   child is a single flex item with the default `min-width: auto`, so the
	   outer wrap had one unbreakable thing to wrap and every chip stayed on one
	   line past the card's edge. Five inventories reach that at phone width. */
	.strip {
		display: flex;
		gap: 4px;
		flex-wrap: wrap;
		min-width: 0;
	}

	/* The marker is a phone control and nothing else: above the breakpoint the
	   row has the width for every chip, and the count would stand where the
	   thing counted could. */
	.more {
		display: none;
	}

	/* On a phone the row shows the first few marketplaces and says how many it
	   is not showing. Which chips are withheld is the component's answer, from
	   the same constant the count comes from, so the two cannot disagree; this
	   sheet only decides the width at which the answer is acted on, and the
	   marker restores them in place. */
	@media (max-width: 620px) {
		.strip:not(.open) .withheld {
			display: none;
		}

		.more {
			display: inline-flex;
			align-items: center;
			font: 600 10.5px var(--sans);
			border-radius: 5px;
			padding: 2px 6px;
			border: 1px solid var(--line);
			background: var(--ground);
			color: var(--muted);
			cursor: pointer;
			white-space: nowrap;
			position: relative;
		}

		/* The marker paints at chip size and is pressed at thumb size. It draws
		   about 26 by 19, which is under WCAG 2.2 SC 2.5.8's 24px on one axis,
		   and the spacing exemption does not reach it because a listed chip
		   beside it is a real target 4px away. Painting it larger instead would
		   push the strip out of the row card's text column, so the hit area is
		   what grows. */
		.more::after {
			content: '';
			position: absolute;
			inset: -12px -9px;
		}
	}

	.mk {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		font-size: 10.5px;
		font-weight: 600;
		border-radius: 5px;
		padding: 2px 6px;
		border: 1px solid var(--line);
		background: var(--ground);
		color: var(--muted);
		white-space: nowrap;
	}

	/* The listed chip is the one that leaves the console, and it keeps the
	   chip's own shape rather than taking the link colour. */
	a.mk {
		text-decoration: none;
		cursor: pointer;
	}

	a.mk:hover {
		border-color: color-mix(in srgb, currentcolor 45%, var(--line));
	}

	/* The marketplace's own mark in place of the acronym, which is the
	   founder's "icons instead of typing out the names". It carries no
	   alternative text because the chip's accessible name already spells the
	   platform out in words, and a second reading of the same fact is noise to
	   anyone listening rather than looking. */
	.mk .mark {
		width: 14px;
		height: 14px;
		object-fit: contain;
		border-radius: 3px;
		flex: none;
	}

	/* The region, and only where three sites of one marketplace share a mark. */
	.mk b {
		font-weight: 700;
		letter-spacing: 0.02em;
	}

	.mk i {
		font-style: normal;
		font-weight: 500;
		opacity: 0.85;
	}

	.mk.ok {
		border-color: color-mix(in srgb, var(--ok) 30%, var(--line));
		background: var(--ok-soft);
		color: var(--ok-ink);
	}

	.mk.run {
		border-color: color-mix(in srgb, var(--warn) 30%, var(--line));
		background: var(--warn-soft);
		color: var(--warn-ink);
	}

	.mk.bad {
		border-color: color-mix(in srgb, var(--bad) 32%, var(--line));
		background: var(--bad-soft);
		color: var(--bad-ink);
	}

	.ghost {
		opacity: 0.45;
		border-style: dashed;
	}

	/* A paused marketplace keeps its own state and says so separately: the
	   listing stands where it stood, and only the sending is held. */
	.paused {
		box-shadow: inset 0 -2px 0 0 var(--warn);
	}
</style>
