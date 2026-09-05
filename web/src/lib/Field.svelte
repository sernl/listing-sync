<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		label,
		id,
		hint,
		required = false,
		children
	}: {
		label: string;
		/** The control's own id, so the label points at it rather than wrapping
		 *  it: several of this console's controls are composites. */
		id: string;
		hint?: string;
		required?: boolean;
		children: Snippet;
	} = $props();
</script>

<div class="field">
	<label for={id}>
		{label}{#if required}<span class="req" aria-hidden="true">*</span>{/if}
	</label>
	{@render children()}
	{#if hint}<span class="hint">{hint}</span>{/if}
</div>
