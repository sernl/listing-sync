<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from '$lib/Icon.svelte';
	import type { IconName } from '$lib/icons';

	let {
		icon,
		title,
		description,
		back,
		aside
	}: {
		icon: IconName;
		title: string;
		description: string;
		/** Where the back control goes on a drill-down view. Set only there:
		 *  Vendoo's own pattern replaces the icon with a back arrow and drops
		 *  the help control, because a drill-down is reached from one page and
		 *  returns to it. */
		back?: { href: string; label: string };
		aside?: Snippet;
	} = $props();
</script>

<header class="page-head">
	{#if back}
		<a class="page-ico" href={back.href} aria-label={back.label}>
			<Icon name="arrow-left" size={18} />
		</a>
	{:else}
		<div class="page-ico"><Icon name={icon} size={18} /></div>
	{/if}
	<div class="head-titles">
		<h1>{title}</h1>
		<p>{description}</p>
	</div>
	{#if aside}
		<div class="head-aside">{@render aside()}</div>
	{/if}
	<!-- Rendered here rather than by each page, so every header band carries
	     exactly one and no page has to remember. A drill-down has none: it is
	     reached from one screen and returns to it, which is Vendoo's own
	     pattern and the specification's. -->
	{#if !back}
		<a class="page-help" href="/guides" aria-label="Help with this page">
			<Icon name="circle-question-mark" size={20} />
		</a>
	{/if}
</header>
