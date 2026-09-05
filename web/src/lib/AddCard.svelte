<script lang="ts">
	import Icon from '$lib/Icon.svelte';

	let {
		title,
		why,
		href,
		onclick
	}: {
		title: string;
		why: string;
		href?: string;
		onclick?: () => void;
	} = $props();
</script>

<svelte:element
	this={href === undefined ? 'button' : 'a'}
	class="add-card"
	{href}
	type={href === undefined ? 'button' : undefined}
	{onclick}
	role={href === undefined ? undefined : 'link'}
>
	<span class="ring"><Icon name="plus" size={18} /></span>
	<span class="t">{title}</span>
	<span class="why">{why}</span>
</svelte:element>
