<script lang="ts">
	import { external } from '$lib/external';
	import TileMark from './TileMark.svelte';
	import type { Mark } from './catalogue';

	let {
		mark,
		name,
		home,
		about
	}: {
		mark: Mark;
		name: string;
		/** The owner's own front page. The mark opens it wherever it is drawn:
		 *  Shopify grants its brand assets on condition they link to its
		 *  homepage, and Mozilla its logo in a visual linking to the program. */
		home?: string;
		/** What it is, on hover. */
		about?: string;
	} = $props();
</script>

<!-- A compact mark with its name under it: the grid of marketplaces we plan to
     add and the "Coming soon" row. One link per entry, named by its label. -->
{#if home}
	<a
		class="mp-chip"
		href={home}
		target="_blank"
		rel="noopener noreferrer"
		title={about ?? name}
		use:external
	>
		<span class="mp-mark mp-small" aria-hidden="true"><TileMark {mark} /></span>
		<span class="mp-chip-name">{name}<span class="mp-away">(opens in a new tab)</span></span>
	</a>
{:else}
	<span class="mp-chip" title={about ?? name}>
		<span class="mp-mark mp-small" aria-hidden="true"><TileMark {mark} /></span>
		<span class="mp-chip-name">{name}</span>
	</span>
{/if}
