<script lang="ts">
	// The fine print, behind a trigger the seller opens on demand.
	//
	// A page's main flow says what to do in one sentence; the reasons, the
	// marketplace policy quotes and the licence cautions live here, one press
	// away. A native <dialog> rather than a hover popover: a thumb cannot
	// hover, and the dialog brings focus trapping and Escape with it.

	import type { Snippet } from 'svelte';
	import Icon from '$lib/Icon.svelte';
	import type { IconName } from '$lib/icons';

	let {
		title,
		label = 'Why?',
		icon = 'info',
		tone = 'info',
		children
	}: {
		/** The dialog's heading, and the trigger's accessible name. */
		title: string;
		/** The trigger's visible word. Empty for an icon-only trigger. */
		label?: string;
		icon?: IconName;
		/** `warn` draws the trigger in the warning ink, for a caution. */
		tone?: 'info' | 'warn';
		children: Snippet;
	} = $props();

	const id = $props.id();
	let element = $state<HTMLDialogElement | null>(null);
</script>

<button
	type="button"
	class="explain-trigger"
	class:warn={tone === 'warn'}
	class:bare={label.length === 0}
	aria-haspopup="dialog"
	aria-label={label.length === 0 ? title : undefined}
	title={title}
	onclick={() => element?.showModal()}
>
	<Icon name={icon} size={14} />
	{#if label.length > 0}<span>{label}</span>{/if}
</button>

<dialog bind:this={element} aria-labelledby="{id}-title" class="explain">
	<div class="dialog-body">
		<h2 id="{id}-title">{title}</h2>
		<div class="explain-body">
			{@render children()}
		</div>
		<div class="actions">
			<button class="btn" type="button" onclick={() => element?.close()}>Close</button>
		</div>
	</div>
</dialog>

<style>
	.explain-trigger {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		min-height: 26px;
		padding: 2px 10px;
		border: 1px solid var(--line);
		border-radius: var(--r-pill);
		background: var(--surface);
		color: var(--primary);
		font: inherit;
		font-size: 12px;
		font-weight: 600;
		line-height: 1;
		cursor: pointer;
		white-space: nowrap;
		vertical-align: middle;
	}

	.explain-trigger:hover {
		background: var(--additive-soft);
	}

	.explain-trigger:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 2px;
	}

	.explain-trigger.bare {
		padding: 2px;
		width: 26px;
		justify-content: center;
	}

	.explain-trigger.warn {
		color: var(--warn-ink);
		border-color: color-mix(in srgb, var(--warn) 40%, var(--line));
		background: var(--warn-soft);
	}

	dialog.explain {
		width: min(560px, calc(100vw - 32px));
	}

	.explain-body {
		display: flex;
		flex-direction: column;
		gap: var(--s-3);
		margin-top: var(--s-3);
		color: var(--text);
		font-size: 13.5px;
		line-height: 1.55;
	}

	.explain-body :global(p) {
		margin: 0;
	}

	.explain-body :global(a) {
		color: var(--primary);
		overflow-wrap: anywhere;
	}
</style>
