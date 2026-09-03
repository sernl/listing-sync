<script lang="ts">
	import type { MarketplaceChip } from '$lib/inventory';
	import { SHORT_NAME, platformTitle } from '$lib/platforms';

	let { chips }: { chips: MarketplaceChip[] } = $props();

	/** The whole sentence a screen reader and a hover both get, so the state is
	 *  never carried by colour alone. */
	function described(chip: MarketplaceChip): string {
		const paused = chip.paused === null ? '' : ` Sending is paused here: ${chip.paused}`;
		const opens = chip.action?.external === true ? ' Opens the listing on the marketplace.' : '';
		return `${platformTitle(chip.inventory)}: ${chip.label}. ${chip.detail}${paused}${opens}`;
	}
</script>

<span class="strip">
	{#each chips as chip (chip.inventory)}
		{#if chip.action?.external === true}
			<a
				class="mk {chip.tone}"
				class:paused={chip.paused !== null}
				href={chip.action.href}
				target="_blank"
				rel="noopener noreferrer"
				title={described(chip)}
				aria-label={described(chip)}
			>
				<b>{SHORT_NAME[chip.inventory]}</b>
				<i>{chip.label}</i>
			</a>
		{:else}
			<span
				class="mk {chip.tone}"
				class:ghost={chip.state === 'not_listed'}
				class:paused={chip.paused !== null}
				title={described(chip)}
				aria-label={described(chip)}
			>
				<b>{SHORT_NAME[chip.inventory]}</b>
				{#if chip.state !== 'not_listed'}<i>{chip.label}</i>{/if}
			</span>
		{/if}
	{/each}
</span>

<style>
	.strip {
		display: inline-flex;
		gap: 4px;
		flex-wrap: nowrap;
	}

	.mk {
		display: inline-flex;
		align-items: baseline;
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
		color: var(--ok);
	}

	.mk.run {
		border-color: color-mix(in srgb, var(--warn) 30%, var(--line));
		background: var(--warn-soft);
		color: var(--warn);
	}

	.mk.bad {
		border-color: color-mix(in srgb, var(--bad) 32%, var(--line));
		background: var(--bad-soft);
		color: var(--bad);
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
