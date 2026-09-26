<script lang="ts" module>
	import type { InventoryId } from '$lib/generated/vocab';
	import type { IconName } from '$lib/icons';

	/** One end of the arrow: a marketplace, or something of the seller's own
	 *  ("Your resources") drawn with a glyph. */
	export type FlowEnd = { inventory: InventoryId } | { icon: IconName; label: string };

	/** One real resource put through the rule, so the seller sees what it
	 *  does before previewing everything. */
	export interface FlowExample {
		name: string;
		before: string;
		after: string;
	}

	/** Source terms on the left, the target terms they land under on the
	 *  right. */
	export interface FlowPair {
		from: readonly string[];
		to: readonly string[];
	}
</script>

<script lang="ts">
	// The mapping as a picture: where it comes from, an arrow carrying the
	// rule, and where it lands. Teachers read the arrow faster than a
	// sentence about it.

	import Icon from '$lib/Icon.svelte';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import { SHORT_NAME } from '$lib/platforms';
	import './flow.css';

	let {
		from,
		to,
		rule = null,
		empty = 'No rule yet',
		pending = 'Not chosen',
		example = null,
		exampleLabel = 'For example',
		pairs = [],
		label
	}: {
		from: FlowEnd;
		to: readonly FlowEnd[];
		/** The rule, written on the arrow. */
		rule?: string | null;
		/** What the arrow says while there is no rule. */
		empty?: string;
		/** What the target end says while `to` is empty. */
		pending?: string;
		example?: FlowExample | null;
		exampleLabel?: string;
		pairs?: readonly FlowPair[];
		/** The whole diagram in words, for a screen reader. */
		label: string;
	} = $props();
</script>

{#snippet end(one: FlowEnd, side: 'from' | 'to')}
	<span class="fd-end {side}">
		<span class="fd-tile">
			{#if 'inventory' in one}
				<MarketplaceMark inventory={one.inventory} size={30} />
			{:else}
				<Icon name={one.icon} size={24} />
			{/if}
		</span>
		<span class="fd-name">{'inventory' in one ? SHORT_NAME[one.inventory] : one.label}</span>
	</span>
{/snippet}

<figure class="flow-diagram" aria-label={label}>
	<div class="fd-row">
		{@render end(from, 'from')}
		<span class="fd-arrow" class:empty={rule === null} aria-hidden="true">
			<span class="fd-rule">{rule ?? empty}</span>
		</span>
		<span class="fd-targets">
			{#each to as one, index (index)}
				{@render end(one, 'to')}
			{:else}
				<span class="fd-end to pending">
					<span class="fd-tile">?</span>
					<span class="fd-name">{pending}</span>
				</span>
			{/each}
		</span>
	</div>

	{#if example !== null}
		<div class="fd-example">
			<span class="fd-example-label">{exampleLabel}</span>
			<span class="res-name">{example.name}</span>
			<span class="fd-prices">
				<span class="price before">{example.before}</span>
				<span class="fd-mini-arrow" aria-label="becomes">→</span>
				<span class="price after">{example.after}</span>
			</span>
		</div>
	{/if}

	{#if pairs.length > 0}
		<ul class="fd-pairs">
			{#each pairs as pair, index (index)}
				<li>
					<span class="fd-chips">
						{#each pair.from as word, at (at)}<span class="term-chip from">{word}</span>{/each}
					</span>
					<span class="fd-mini-arrow" aria-label="lands under">→</span>
					<span class="fd-chips">
						{#each pair.to as word, at (at)}<span class="term-chip to">{word}</span>{/each}
					</span>
				</li>
			{/each}
		</ul>
	{/if}
</figure>

<style>
	.flow-diagram {
		margin: 0;
		padding: var(--s-5);
		border-radius: var(--r-panel);
		background: color-mix(in srgb, var(--additive-soft) 55%, var(--surface));
		display: flex;
		flex-direction: column;
		gap: var(--s-4);
		min-width: 0;
	}

	.fd-row {
		display: flex;
		align-items: center;
		gap: var(--s-3);
		min-width: 0;
	}

	.fd-end {
		flex: none;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 6px;
		min-width: 64px;
	}

	.fd-tile {
		display: grid;
		place-items: center;
		width: 56px;
		height: 56px;
		border-radius: 14px;
		background: var(--mark-ground);
		border: 2px solid var(--lavender);
		color: var(--additive);
		box-shadow: var(--sh-1);
	}

	.fd-end.to .fd-tile {
		border-color: var(--accent);
		color: var(--ok-ink);
	}

	.fd-end.pending .fd-tile {
		border-style: dashed;
		border-color: var(--muted);
		color: var(--muted);
		font-size: 20px;
		font-weight: 600;
		box-shadow: none;
	}

	.fd-name {
		font-size: 12.5px;
		font-weight: 600;
		color: var(--text);
		white-space: nowrap;
	}

	.fd-targets {
		display: flex;
		flex-wrap: wrap;
		gap: var(--s-3);
	}

	/* A line with a head, and the rule sitting on it. */
	.fd-arrow {
		position: relative;
		flex: 1 1 auto;
		min-width: 72px;
		display: flex;
		justify-content: center;
		align-items: center;
		align-self: flex-start;
		height: 56px;
	}

	.fd-arrow::before {
		content: '';
		position: absolute;
		left: 0;
		right: 8px;
		top: 50%;
		height: 2px;
		margin-top: -1px;
		background: var(--accent);
		border-radius: 1px;
	}

	.fd-arrow::after {
		content: '';
		position: absolute;
		right: 0;
		top: 50%;
		margin-top: -7px;
		border: 7px solid transparent;
		border-right: 0;
		border-left: 10px solid var(--accent);
	}

	.fd-arrow.empty::before {
		background: repeating-linear-gradient(
			90deg,
			var(--muted) 0 6px,
			transparent 6px 11px
		);
	}

	.fd-arrow.empty::after {
		border-left-color: var(--muted);
	}

	.fd-rule {
		position: relative;
		z-index: 1;
		max-width: calc(100% - 16px);
		padding: 5px 12px;
		border-radius: var(--r-pill);
		background: var(--surface);
		border: 1.5px solid var(--accent);
		color: var(--primary);
		font-size: 13px;
		font-weight: 600;
		text-align: center;
		line-height: 1.3;
		box-shadow: var(--sh-1);
	}

	.empty .fd-rule {
		border-style: dashed;
		border-color: var(--muted);
		color: var(--muted);
		font-weight: 500;
	}

	.fd-example {
		display: flex;
		align-items: baseline;
		flex-wrap: wrap;
		gap: var(--s-2) var(--s-3);
		padding: var(--s-3) var(--s-4);
		border-radius: var(--r-field);
		background: var(--surface);
		border: 1px solid var(--line);
	}

	.fd-example-label {
		font-size: 11px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--muted);
	}

	.fd-example .res-name {
		flex: 1 1 12rem;
		min-width: 0;
	}

	.fd-prices {
		display: inline-flex;
		align-items: baseline;
		gap: var(--s-2);
		white-space: nowrap;
		font-size: 15px;
	}

	.price.before {
		color: var(--muted);
		font-weight: 500;
	}

	.fd-mini-arrow {
		color: var(--muted);
		font-weight: 600;
	}

	.fd-pairs {
		display: flex;
		flex-direction: column;
		gap: var(--s-2);
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.fd-pairs li {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: var(--s-2);
		padding: var(--s-2) var(--s-3);
		border-radius: var(--r-field);
		background: var(--surface);
		border: 1px solid var(--line);
	}

	.fd-chips {
		display: inline-flex;
		flex-wrap: wrap;
		gap: 6px;
	}

	@media (max-width: 720px) {
		.flow-diagram {
			padding: var(--s-4);
		}

		.fd-row {
			gap: var(--s-2);
		}

		.fd-end {
			min-width: 56px;
		}

		.fd-tile {
			width: 48px;
			height: 48px;
		}

		.fd-arrow {
			min-width: 56px;
			height: 48px;
		}

		.fd-rule {
			font-size: 12px;
			padding: 4px 8px;
		}
	}
</style>
