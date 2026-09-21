<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from '$lib/Icon.svelte';
	import type { IconName } from '$lib/icons';

	let {
		icon,
		headline,
		body,
		actions
	}: {
		icon: IconName;
		headline: string;
		/** The one sentence that says what to do about the emptiness. Omit it
		 *  where the headline already says it. */
		body?: string;
		/** The way forward, where there is one. Two actions of different
		 *  weights say which is recommended; one says there is only the one. */
		actions?: Snippet;
	} = $props();
</script>

<div class="placeholder">
	<span class="big"><Icon name={icon} size={30} /></span>
	<b>{headline}</b>
	{#if body}<p>{body}</p>{/if}
	{#if actions}<div class="actions">{@render actions()}</div>{/if}
</div>
