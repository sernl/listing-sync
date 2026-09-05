<script lang="ts">
	import Icon from '$lib/Icon.svelte';
	import type { IconName } from '$lib/icons';

	let {
		icon,
		label,
		value,
		figure,
		sub,
		tag,
		counted = false
	}: {
		icon: IconName;
		label: string;
		/** The text to draw. The model decides it, em dash included. */
		value: string;
		/** Whether that text is a figure, which is what sets its weight. */
		figure: boolean;
		sub?: string;
		/** Where the figure came from, when that is not the marketplace this
		 *  tile's scope names. */
		tag?: string;
		/** Counted from the seller's own catalogue rather than reported by a
		 *  marketplace, which the tile says in its own colour as well as in
		 *  its words. */
		counted?: boolean;
	} = $props();
</script>

<div class="an-tile {counted ? 'an-counted' : ''}">
	{#if tag}<span class="an-tag">{tag}</span>{/if}
	<div class="an-ico"><Icon name={icon} size={16} /></div>
	<div class="an-n {figure ? '' : 'an-none'}">{value}</div>
	<div class="an-lbl">{label}</div>
	{#if sub}<div class="an-sub">{sub}</div>{/if}
</div>
