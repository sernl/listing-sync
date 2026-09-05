<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from '$lib/Icon.svelte';
	import type { IconName } from '$lib/icons';

	let {
		icon,
		tone = '',
		tag,
		label,
		sub,
		children
	}: {
		icon: IconName;
		/** `ok`, `warn` or `bad`; the accent tint when omitted. */
		tone?: string;
		tag?: string;
		label: string;
		sub?: string;
		children: Snippet;
	} = $props();
</script>

<div class="stat">
	{#if tag}<span class="tag">{tag}</span>{/if}
	<div class="ico {tone}"><Icon name={icon} size={16} /></div>
	<div class="n">{@render children()}</div>
	<div class="lbl">{label}</div>
	{#if sub}<div class="sub">{sub}</div>{/if}
</div>
