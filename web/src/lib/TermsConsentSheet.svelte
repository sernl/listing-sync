<script lang="ts">
	// The agreement asked for again, when the terms have changed since this
	// account last agreed. Over every console page and not dismissable: a
	// modal <dialog> traps focus, Escape is refused, and nothing closes it but
	// the agreement landing. Held back while an operator is impersonating,
	// because an operator must never agree on a seller's behalf.

	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import ConsentBoxes from '$lib/ConsentBoxes.svelte';
	import { consentBody } from '$lib/legal';
	import { queryKeys } from '$lib/query';

	let {
		suppressed
	}: {
		/** Neither ask nor show: an operator is impersonating, or whether one
		 *  is has not been read yet. */
		suppressed: boolean;
	} = $props();

	const queryClient = useQueryClient();

	const status = createQuery(() => ({
		queryKey: queryKeys.termsConsent,
		queryFn: () => api.consentStatus(),
		enabled: !suppressed
	}));

	let element = $state<HTMLDialogElement | null>(null);
	let terms = $state(false);
	let age = $state(false);
	let saving = $state(false);
	let refusal = $state<string | null>(null);

	const required = $derived(!suppressed && status.data?.required === true);
	/** The agreement to the version the server says is in force, which is the
	 *  one the links open; null until both boxes are ticked. */
	const agreement = $derived(
		status.data === undefined ? null : consentBody({ terms, age }, status.data.current_version)
	);

	$effect(() => {
		if (required && element !== null && !element.open) {
			element.showModal();
		}
	});

	async function agree() {
		if (agreement === null || saving) {
			return;
		}
		saving = true;
		refusal = null;
		try {
			await api.recordConsent(agreement);
			await queryClient.invalidateQueries({ queryKey: queryKeys.termsConsent });
			terms = false;
			age = false;
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure
					? failure.message
					: 'Your agreement was not saved. Please try again.';
		} finally {
			saving = false;
		}
	}
</script>

{#if required}
	<!-- `oncancel` refuses Escape; a browser that closes anyway on a second
	     press is answered by `onclose` opening it again while the agreement is
	     still owed. -->
	<dialog
		bind:this={element}
		id="terms-consent-sheet"
		class="terms-consent"
		aria-labelledby="terms-consent-title"
		aria-describedby="terms-consent-lede"
		oncancel={(event) => event.preventDefault()}
		onclose={() => {
			if (required) element?.showModal();
		}}
	>
		<div class="dialog-body">
			<h2 id="terms-consent-title">Our terms have changed</h2>
			<p id="terms-consent-lede">Please read them and tick both boxes to keep using Teachouse.</p>

			<ConsentBoxes bind:terms bind:age disabled={saving} idPrefix="terms-consent" />

			{#if refusal !== null}
				<p class="refusal" role="alert">{refusal}</p>
			{/if}

			<div class="actions">
				<Button
					tier="primary"
					disabled={agreement === null || saving}
					reason={saving
						? 'Saving your agreement.'
						: agreement === null
							? 'Tick both boxes first.'
							: undefined}
					onclick={agree}
				>
					{saving ? 'Saving…' : 'I agree'}
				</Button>
			</div>
		</div>
	</dialog>
{/if}

<style>
	.terms-consent .dialog-body {
		display: grid;
		gap: var(--s-3);
	}

	.terms-consent .dialog-body > p {
		margin: 0;
		font-size: 13.5px;
		line-height: 1.5;
	}

	.terms-consent .refusal {
		margin: 0;
	}

	.terms-consent .actions {
		margin-top: var(--s-1);
	}
</style>
