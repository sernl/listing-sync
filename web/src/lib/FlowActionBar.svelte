<script lang="ts">
	// A page's primary action where a thumb is on a phone: a bar pinned above
	// the tab bar, shown at 720px and below. On a wider screen the same action
	// sits at the top right of its step, so this renders nothing there.

	import type { Snippet } from 'svelte';
	import './flow.css';

	let { children }: { children: Snippet } = $props();
</script>

<div class="flow-bar" role="region" aria-label="Main action">
	{@render children()}
</div>

<style>
	.flow-bar {
		display: none;
	}

	@media (max-width: 720px) {
		.flow-bar {
			display: flex;
			gap: var(--s-2);
			position: fixed;
			left: 0;
			right: 0;
			bottom: 0;
			z-index: 15;
			padding: var(--s-3) var(--s-4);
			background: color-mix(in srgb, var(--surface) 94%, transparent);
			border-top: 1px solid var(--line);
			box-shadow: var(--sh-3);
			backdrop-filter: blur(8px);
		}

		.flow-bar > :global(*) {
			flex: 1;
			justify-content: center;
		}
	}

	/* Above the phone tab bar, which owns the bottom 80px below 620px. */
	@media (max-width: 620px) {
		.flow-bar {
			bottom: calc(80px + env(safe-area-inset-bottom, 0px));
		}
	}
</style>
