<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		icon,
		tone = '',
		tag,
		label,
		sub,
		children
	}: {
		icon: string;
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
	<div class="ico {tone}" aria-hidden="true">{icon}</div>
	<div class="n">{@render children()}</div>
	<div class="lbl">{label}</div>
	{#if sub}<div class="sub">{sub}</div>{/if}
</div>
