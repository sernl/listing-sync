<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from '$lib/Icon.svelte';
	import type { IconName } from '$lib/icons';

	// One quiet sentence beside a control, and the only channel for one. It
	// replaces the bare `<p class="foot-note">` the console used to write
	// inline, which had no component gate and grew into the place every
	// explain-why paragraph was parked. A component can carry the budget the
	// class could not: one short sentence that says what to do, with anything
	// longer belonging in a guide.

	let {
		icon,
		title,
		children
	}: {
		/** A glyph before the sentence, where one helps the eye find it. */
		icon?: IconName;
		/** The exact figure behind a rounded one, such as the UTC instant
		 *  under a relative date. Hover text, never the sentence itself. */
		title?: string;
		children: Snippet;
	} = $props();
</script>

<p class="note" {title}>
	{#if icon}<Icon name={icon} size={14} />{/if}
	<span>{@render children()}</span>
</p>
