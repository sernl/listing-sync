<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		icon,
		title,
		description,
		aside
	}: {
		icon: string;
		title: string;
		description: string;
		aside?: Snippet;
	} = $props();
</script>

<header class="page-head">
	<div class="page-ico" aria-hidden="true">{icon}</div>
	<div>
		<h1>{title}</h1>
		<p>{description}</p>
	</div>
	{#if aside}
		<div class="head-aside">{@render aside()}</div>
	{/if}
</header>
