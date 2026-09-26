<script lang="ts">
	import { useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import type { Marketplace } from '$lib/generated/vocab';
	import { queryKeys } from '$lib/query';
	import { CONSENT_NOTICE_VERSION, consentCopy } from './consent';

	let {
		open,
		marketplace,
		onAgreed,
		onClose
	}: {
		open: boolean;
		marketplace: Marketplace;
		/** The grant landed and the cache is invalidated. */
		onAgreed: () => void;
		/** Cancel, Escape or the backdrop: nothing was recorded. */
		onClose: () => void;
	} = $props();

	const queryClient = useQueryClient();

	let element = $state<HTMLDialogElement | null>(null);
	let ticked = $state(false);
	let saving = $state(false);
	let refusal = $state<string | null>(null);

	const copy = $derived(consentCopy(marketplace));

	// Guarded on the element's own state: `showModal` on a dialog that is
	// already modal throws, and this effect re-runs whenever the element is
	// bound as well as when `open` moves. The box starts unticked on every
	// opening, because the agreement is an act and not a remembered setting.
	$effect(() => {
		if (open && element !== null && !element.open) {
			ticked = false;
			refusal = null;
			element.showModal();
		} else if (!open) {
			element?.close();
		}
	});

	async function agree() {
		if (!ticked || saving) {
			return;
		}
		saving = true;
		refusal = null;
		try {
			await api.grantConsent(marketplace, CONSENT_NOTICE_VERSION);
			await queryClient.invalidateQueries({ queryKey: queryKeys.consents });
			onAgreed();
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure
					? failure.message
					: 'Your permission was not saved, so nothing was connected. Try again.';
		} finally {
			saving = false;
		}
	}
</script>

<dialog bind:this={element} aria-labelledby="consent-title" onclose={onClose}>
	<div class="dialog-body">
		<h2 id="consent-title">{copy.title}</h2>
		<p>{copy.intro}</p>
		<p>By connecting, you allow Teachouse to:</p>
		<ul>
			{#each copy.grants as grant (grant)}
				<li>{grant}</li>
			{/each}
		</ul>
		<p class="foot-note">{copy.boundary}</p>

		<label class="choice">
			<input type="checkbox" bind:checked={ticked} disabled={saving} />
			<span class="t">{copy.checkbox}</span>
		</label>

		{#if refusal !== null}
			<p class="refusal">{refusal}</p>
		{/if}

		<div class="actions">
			<button class="btn" type="button" onclick={onClose} disabled={saving}>Cancel</button>
			<button class="cta" type="button" onclick={agree} disabled={!ticked || saving}>
				{saving ? 'Saving…' : 'I agree'}
			</button>
		</div>
	</div>
</dialog>
