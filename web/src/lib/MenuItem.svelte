<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from '$lib/Icon.svelte';
	import type { IconName } from '$lib/icons';

	// One row inside `Menu`. The menu itself takes a snippet rather than a list
	// of items, because half its call sites are filter checkboxes rather than
	// verbs; this is the verb shape, so a glyph, a disabled reason and the
	// 44px phone target are written once instead of at every kebab.

	let {
		icon,
		danger = false,
		disabled = false,
		/** Why the item cannot run. Required when `disabled`, for the reason
		 *  `Button` states it: a dead row with no reason reads as a fault. */
		reason,
		onclick,
		children
	}: {
		icon?: IconName;
		danger?: boolean;
		disabled?: boolean;
		reason?: string;
		onclick?: () => void;
		children: Snippet;
	} = $props();
</script>

<button
	type="button"
	role="menuitem"
	class:danger
	{disabled}
	title={reason}
	{onclick}
>
	{#if icon}<Icon name={icon} size={14} />{/if}
	<span>{@render children()}</span>
</button>
