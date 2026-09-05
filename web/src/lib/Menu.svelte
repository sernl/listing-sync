<script lang="ts">
	import type { Snippet } from 'svelte';
	import { closes } from '$lib/menu-dismissal';

	let {
		open = $bindable(false),
		label,
		trigger,
		children
	}: {
		open?: boolean;
		/** Names the menu for a screen reader, since the trigger is often a
		 *  glyph with no text of its own. */
		label: string;
		trigger: Snippet;
		children: Snippet;
	} = $props();

	let anchor = $state<HTMLElement | null>(null);

	// Which events close a menu is `$lib/menu-dismissal`'s to say, so the rule is
	// stated once and tested; this component only reports what happened. The
	// third reason, a choice, reaches the menu through `open` from whoever
	// renders the items.
	function pressed(event: MouseEvent) {
		const inside = anchor !== null && anchor.contains(event.target as Node);
		if (open && closes({ kind: 'press', inside })) {
			open = false;
		}
	}

	function keyed(event: KeyboardEvent) {
		if (open && event.key === 'Escape' && closes({ kind: 'escape' })) {
			open = false;
		}
	}
</script>

<svelte:window onpointerdown={pressed} onkeydown={keyed} />

<span class="menu-anchor" bind:this={anchor}>
	{@render trigger()}
	{#if open}
		<div class="menu" role="menu" aria-label={label}>{@render children()}</div>
	{/if}
</span>
