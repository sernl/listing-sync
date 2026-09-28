<script lang="ts" module>
	/** What a sheet's owner can do to it from outside. */
	export interface SheetHandle {
		/** Close it the way the close button does, running `onClose`. */
		close: () => void;
	}
</script>

<script lang="ts">
	// A panel that slides over the page: from the right on a desktop, up from
	// the bottom on a phone. A modal <dialog> underneath, so focus is trapped
	// and Escape closes it natively; a press on the dimmed page behind it
	// closes it too (`lightDismiss`). `oncancel` can refuse both while
	// something is saving.

	import type { Snippet } from 'svelte';
	import { lightDismiss } from '$lib/dismiss';

	let {
		labelledby,
		onClose,
		oncancel,
		handle = $bindable(),
		children
	}: {
		/** The id of the heading that names the sheet. */
		labelledby: string;
		/** The sheet has closed, by any route. */
		onClose: () => void;
		/** Escape or an outside press; `preventDefault()` keeps it open. */
		oncancel?: (event: Event) => void;
		handle?: SheetHandle;
		children: Snippet;
	} = $props();

	let element = $state<HTMLDialogElement | null>(null);

	handle = { close: () => element?.close() };

	$effect(() => {
		if (element !== null && !element.open) {
			element.showModal();
		}
	});
</script>

<dialog
	bind:this={element}
	class="sheet"
	aria-labelledby={labelledby}
	onclose={onClose}
	{oncancel}
	use:lightDismiss
>
	<div class="sheet-panel">
		{@render children()}
	</div>
</dialog>

<style>
	dialog.sheet[open] {
		display: flex;
	}

	.sheet-panel {
		display: flex;
		flex-direction: column;
		min-height: 0;
		width: 100%;
	}

	@media (min-width: 621px) {
		dialog.sheet {
			margin: 0 0 0 auto;
			width: min(460px, 100vw);
			max-width: 100vw;
			height: 100dvh;
			max-height: 100dvh;
			border-radius: var(--r-region) 0 0 var(--r-region);
			border-right: 0;
		}
	}
</style>
