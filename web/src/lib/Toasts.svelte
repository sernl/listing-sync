<script lang="ts">
	import { tick } from 'svelte';
	import Icon from '$lib/Icon.svelte';
	import { afterToastDismissed, captureSlots, focusRegion } from '$lib/focus-return';
	import type { IconName } from '$lib/icons';
	import { dismiss, sweep, toastStore, type Toast } from '$lib/toast';

	// The stack every layout draws its toasts in: the console's, and the
	// signed-out screens', which report a refused sign-in the same way.

	// The console's notice glyph for each tone, as `Banner` holds its own.
	// Here rather than in `toast.ts`, which is a pure store with no view in
	// it, and read through the record rather than written as a ternary in the
	// markup: `icons.test.ts` sweeps the quoted arms of an `Icon name={...}`
	// expression, so a tone compared inline reads to it as an icon name.
	const TOAST_GLYPH: Record<Toast['tone'], IconName> = {
		info: 'circle-check',
		error: 'circle-alert'
	};

	// The toast stack's clock lives here rather than in the store, because what
	// pauses it is a pointer and a focus ring, and only the element knows about
	// those. A sweep that removed the toast under the pointer would steal the
	// click that was about to close it, so the stack holds every standing toast
	// while it is hovered or focused within, and lets them go on the way out.
	//
	// The interval is armed only while something is standing, and `sweep` is
	// silent when it drops nothing, so an idle console redraws nothing.
	let held = $state(false);

	$effect(() => {
		if (held || $toastStore.length === 0) {
			return;
		}
		const timer = setInterval(() => sweep(Date.now()), 250);
		return () => clearInterval(timer);
	});

	// Who was working when each toast arrived, keyed by that toast's id.
	// Captured as the toast appears rather than as it is closed, because by then
	// the close control itself holds focus and the answer is gone. Per toast
	// rather than one slot for the stack: two overlapping toasts interrupted two
	// different controls, and a single slot sent the second one's closer back to
	// whatever the first had interrupted.
	//
	// Held here rather than on the toast record, so `toast.ts` stays a pure
	// store with no element references in it. Not `$state`: nothing renders from
	// it, and a reactive read here would re-run the effect that writes it.
	const interrupted = new Map<number, HTMLElement | null>();

	$effect(() => {
		const { add, drop } = captureSlots(
			$toastStore.map((entry) => entry.id),
			[...interrupted.keys()]
		);
		for (const id of drop) {
			interrupted.delete(id);
		}
		if (add.length === 0) {
			return;
		}
		const active = document.activeElement;
		const opener =
			active instanceof HTMLElement && active.closest('.toasts') === null ? active : null;
		for (const id of add) {
			interrupted.set(id, opener);
		}
	});

	function keyedToast(event: KeyboardEvent, id: number) {
		if (event.key !== 'Escape') {
			return;
		}
		event.stopPropagation();
		void closeToast(id);
	}

	async function closeToast(id: number) {
		const previous = interrupted.get(id) ?? null;
		dismiss(id);
		await tick();
		const region = document.querySelector('main');
		switch (
			afterToastDismissed({
				previous: previous?.isConnected === true,
				region: region !== null
			})
		) {
			case 'previous':
				previous?.focus();
				break;
			case 'region':
				if (region !== null) {
					focusRegion(region);
				}
				break;
			case 'none':
				break;
		}
	}
</script>

<div
	class="toasts"
	role="status"
	aria-live="polite"
	onmouseenter={() => (held = true)}
	onmouseleave={() => (held = false)}
	onfocusin={() => (held = true)}
	onfocusout={() => (held = false)}
>
	{#each $toastStore as entry (entry.id)}
		<!-- Escape closes the toast focus is inside, and the handler sits on
		     that toast rather than on the window so a dialog open over the
		     stack keeps the key. The close button is the only thing in a toast
		     that takes focus, so this reaches the same seller by another
		     press; the element is not a control in its own right. -->
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			class="toast {entry.tone === 'error' ? 'error' : ''}"
			onkeydown={(event) => keyedToast(event, entry.id)}
		>
			<span class="toast-mark">
				<Icon name={TOAST_GLYPH[entry.tone]} size={18} />
			</span>
			<span class="toast-say">{entry.message}</span>
			<button
				type="button"
				class="toast-close"
				aria-label="Close: {entry.message}"
				onclick={() => closeToast(entry.id)}
			>
				<Icon name="x" size={16} />
			</button>
		</div>
	{/each}
</div>
