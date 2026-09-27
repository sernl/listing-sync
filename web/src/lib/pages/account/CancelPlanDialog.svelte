<script lang="ts">
	import { ApiFailure, api, type BillingView } from '$lib/api';
	import { dayLabel } from '$lib/pages/account/plans';

	let {
		open,
		planName,
		periodEnd,
		onClose,
		onCancelled
	}: {
		open: boolean;
		planName: string;
		/** When the paid period closes, which is when the plan stops. */
		periodEnd: number | undefined;
		onClose: () => void;
		onCancelled: (view: BillingView) => void;
	} = $props();

	let element = $state<HTMLDialogElement | null>(null);
	let sending = $state(false);
	let refusal = $state<string | null>(null);

	// Guarded on the element's own state: `showModal` on a dialog that is
	// already modal throws.
	$effect(() => {
		if (open && element !== null && !element.open) {
			refusal = null;
			element.showModal();
		} else if (!open) {
			element?.close();
		}
	});

	async function confirm() {
		if (sending) return;
		sending = true;
		refusal = null;
		try {
			onCancelled(await api.billingCancel());
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure
					? failure.message
					: 'Your plan was not cancelled. Try again in a moment.';
		} finally {
			sending = false;
		}
	}
</script>

<dialog
	bind:this={element}
	aria-labelledby="cancel-plan-title"
	onclose={onClose}
	oncancel={(event) => {
		if (sending) event.preventDefault();
	}}
>
	<div class="dialog-body">
		<h2 id="cancel-plan-title">Cancel your {planName} plan?</h2>
		<p>
			{#if periodEnd === undefined}
				You keep your plan until the end of the period you have paid for. It will not renew.
			{:else}
				You keep your plan until {dayLabel(periodEnd)}. It will not renew after that.
			{/if}
		</p>
		<p>
			Your moves stay yours until they expire. You can keep your plan any time before that date.
		</p>

		{#if refusal !== null}
			<p class="refusal">{refusal}</p>
		{/if}

		<div class="actions">
			<button class="btn" type="button" onclick={onClose} disabled={sending}>Keep my plan</button>
			<button class="btn danger" type="button" onclick={confirm} disabled={sending}>
				{sending ? 'Cancelling…' : 'Cancel plan'}
			</button>
		</div>
	</div>
</dialog>
