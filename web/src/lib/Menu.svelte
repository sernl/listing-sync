<script lang="ts">
	import type { Snippet } from 'svelte';
	import { closes } from '$lib/menu-dismissal';

	let {
		open = $bindable(false),
		label,
		align = 'end',
		trigger,
		children
	}: {
		open?: boolean;
		/** Names the menu for a screen reader, since the trigger is often a
		 *  glyph with no text of its own. */
		label: string;
		/** Which edge of the trigger the panel hangs from.
		 *
		 *  `end` is right-anchored and is what every menu near the right of a
		 *  page needs to stay on screen; a trigger near the left edge of its
		 *  column needs `start`, or the panel opens leftward out of the column
		 *  and its first word is cut off. CSS cannot measure which case it is
		 *  in, so the caller names it. */
		align?: 'start' | 'end';
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
		<div class="menu" class:menu-start={align === 'start'} role="menu" aria-label={label}>
			{@render children()}
		</div>
	{/if}
</span>
