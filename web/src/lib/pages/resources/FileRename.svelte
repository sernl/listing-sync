<script lang="ts">
	// One file's name, typed in place under its row. Shared by the Files and
	// Preview bands so a rename reads and behaves the same in both.
	import { untrack } from 'svelte';
	import Button from '$lib/Button.svelte';

	let {
		name,
		what,
		busy = false,
		onSave,
		onCancel
	}: {
		/** The name the row has now, as the field's starting text. */
		name: string;
		/** How the row is spoken of, for the field's accessible name. */
		what: string;
		busy?: boolean;
		onSave: (name: string) => void;
		onCancel: () => void;
	} = $props();

	// The prefill, not a binding: a refetch while the seller types must not
	// wipe what they typed.
	let typed = $state(untrack(() => name));
	const blank = $derived(typed.trim() === '');
</script>

<form
	class="res-file-rename"
	onsubmit={(event) => {
		event.preventDefault();
		if (!blank && !busy) {
			onSave(typed.trim());
		}
	}}
>
	<!-- svelte-ignore a11y_autofocus -->
	<input
		class="res-file-name"
		bind:value={typed}
		maxlength="255"
		autofocus
		aria-label="New name for {what}"
		onkeydown={(event) => {
			if (event.key === 'Escape') {
				event.preventDefault();
				onCancel();
			}
		}}
	/>
	<Button
		small
		tier="primary"
		type="submit"
		disabled={busy || blank}
		reason={busy ? 'Wait for the current change to finish.' : blank ? 'Type a name first.' : undefined}
		>Save name</Button
	>
	<Button small onclick={onCancel}>Cancel</Button>
</form>
