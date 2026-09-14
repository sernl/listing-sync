<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		title,
		description,
		id,
		more,
		children
	}: {
		title?: string;
		description?: string;
		/** An anchor, for a link that lands on this panel. */
		id?: string;
		more?: Snippet;
		children: Snippet;
	} = $props();
</script>

<section class="panel" {id}>
	{#if title || more}
		<div class="head-row">
			<div>
				{#if title}<h2>{title}</h2>{/if}
				{#if description}<div class="desc">{description}</div>{/if}
			</div>
			{#if more}{@render more()}{/if}
		</div>
	{/if}
	{@render children()}
</section>
