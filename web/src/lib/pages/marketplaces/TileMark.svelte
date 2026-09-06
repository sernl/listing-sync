<script lang="ts">
	import Icon from '$lib/Icon.svelte';
	import { type Mark, wordmarkSize } from './catalogue';

	let { mark }: { mark: Mark } = $props();

	/** How large a neutral glyph is drawn, against the square tile's 38px of
	 *  usable room: a line drawing filling the tile edge to edge reads as
	 *  straining against the border in a way a filled logo does not. */
	const GLYPH_PX = 26;
</script>

<!-- What goes inside a `.mp-mark` box, and nothing about the box itself. The box
     is an anchor on a tile whose mark opens the owner's own site and a plain
     span everywhere else, which is the caller's decision; how a `Mark` draws is
     this file's, and it is one file so that a mark landing in a new form draws
     the same on every tile that shows one. -->
{#if mark.kind === 'wordmark'}
	<span class="word" style="font-size: {wordmarkSize(mark.text)}px">{mark.text}</span>
{:else if mark.kind === 'glyph'}
	<Icon name={mark.name} size={GLYPH_PX} />
{:else}
	<img src={mark.src} alt="" loading="lazy" />
{/if}
