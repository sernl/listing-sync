<script lang="ts">
	// The refund panel: how much, why, a note for ourselves, whether the
	// customer hears about it, then one sentence to confirm. The request id is
	// minted once per opening, so a double press or a retry after a dropped
	// answer refunds once.
	import { createMutation } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import Icon from '$lib/Icon.svelte';
	import Sheet, { type SheetHandle } from '$lib/Sheet.svelte';
	import { toast } from '$lib/toast';
	import { money } from '$lib/pages/account/plans';
	import {
		REASONS,
		REASON_LABEL,
		confirmSentence,
		eventAmount,
		nzShortDate,
		refundBody,
		refundDraft,
		refundProblems,
		toCents,
		type PaymentEventView,
		type RefundDraft,
		type RefundField,
		type RefundView
	} from '$lib/pages/admin/payments';

	let {
		payment,
		left,
		autoMail,
		issuedBy,
		onClose,
		onRefunded
	}: {
		/** The `payment_succeeded` row being refunded. */
		payment: PaymentEventView;
		/** Cents still refundable on its charge. */
		left: number;
		autoMail: boolean;
		/** The operator's name, as the refund's log line carries it. */
		issuedBy: string;
		onClose: () => void;
		onRefunded: (refund: RefundView) => Promise<void>;
	} = $props();

	let sheet = $state<SheetHandle>();
	const requestId = crypto.randomUUID();
	const currency = $derived(payment.currency ?? 'usd');
	const charge = $derived(payment.charge_id ?? payment.provider_object_id);

	// svelte-ignore state_referenced_locally
	let draft = $state<RefundDraft>(refundDraft(left, autoMail));
	let confirming = $state(false);
	let tried = $state(false);
	let touched = $state<Partial<Record<RefundField, boolean>>>({});
	let refusal = $state<string | null>(null);

	const problems = $derived(refundProblems(draft, left, currency));
	const problemOf = (field: RefundField) =>
		tried || touched[field] === true ? (problems[field] ?? null) : null;

	const refunding = createMutation(() => ({
		mutationFn: () => api.refundCharge(charge, refundBody(draft, requestId, issuedBy)),
		onSuccess: async (refund: RefundView) => {
			await onRefunded(refund);
			toast('success', `Refunded ${money(refund.amount_cents, refund.currency)}.`);
			sheet?.close();
		},
		onError: (failure: Error) => {
			confirming = false;
			refusal = failure instanceof ApiFailure ? failure.message : 'The refund did not go through.';
		}
	}));

	function review() {
		tried = true;
		if (Object.keys(problems).length === 0) {
			refusal = null;
			confirming = true;
		}
	}

	function keepOpenWhileSending(event: Event) {
		if (refunding.isPending) event.preventDefault();
	}
</script>

{#snippet problem(field: RefundField, hint: string)}
	{@const says = problemOf(field)}
	{#if says === null}
		<span class="hint">{hint}</span>
	{:else}
		<span class="hint rf-error" role="alert">
			<Icon name="circle-alert" size={13} />
			{says}
		</span>
	{/if}
{/snippet}

<Sheet labelledby="refund-title" {onClose} oncancel={keepOpenWhileSending} bind:handle={sheet}>
	<header class="rf-head">
		<div class="rf-who">
			<h2 id="refund-title">Refund {payment.org_name ?? 'this payment'}</h2>
			<span class="rf-sub">
				{eventAmount(payment)} paid {nzShortDate(payment.occurred_at)} ·
				{money(left, currency)} left to refund
			</span>
		</div>
		<button
			type="button"
			class="rf-close"
			aria-label="Close"
			disabled={refunding.isPending}
			onclick={() => sheet?.close()}
		>
			<Icon name="x" />
		</button>
	</header>

	<form
		class="rf-body"
		novalidate
		onsubmit={(event) => {
			event.preventDefault();
			if (confirming) refunding.mutate();
			else review();
		}}
	>
		{#if refusal}
			<Banner tone="bad" title="The refund did not go through">{refusal}</Banner>
		{/if}

		{#if confirming}
			<p class="rf-confirm">
				{confirmSentence(toCents(draft.dollars) ?? 0, currency, payment.org_name)}
			</p>
			<p class="quiet">
				{REASON_LABEL[draft.reason === '' ? 'requested_by_customer' : draft.reason]}.
				{draft.sendEmail ? 'The customer gets an email.' : 'No email goes to the customer.'}
			</p>
			<div class="rf-foot">
				<Button
					disabled={refunding.isPending}
					reason={refunding.isPending ? 'Refunding.' : undefined}
					onclick={() => (confirming = false)}
				>
					Back
				</Button>
				<Button
					tier="primary"
					danger
					type="submit"
					icon="check"
					disabled={refunding.isPending}
					reason={refunding.isPending ? 'Refunding.' : undefined}
				>
					{refunding.isPending ? 'Refunding…' : 'Refund'}
				</Button>
			</div>
		{:else}
			<Field label="Amount ({currency.toUpperCase()})" id="refund-amount" required>
				<input
					id="refund-amount"
					type="number"
					min="0.01"
					step="0.01"
					max={left / 100}
					bind:value={draft.dollars}
					onblur={() => (touched = { ...touched, amount: true })}
					aria-invalid={problemOf('amount') !== null}
				/>
				{@render problem('amount', `Up to ${money(left, currency)}.`)}
			</Field>

			<Field label="Reason" id="refund-reason" required>
				<select
					id="refund-reason"
					bind:value={draft.reason}
					onblur={() => (touched = { ...touched, reason: true })}
					aria-invalid={problemOf('reason') !== null}
				>
					<option value="" disabled>Pick one</option>
					{#each REASONS as reason (reason)}
						<option value={reason}>{REASON_LABEL[reason]}</option>
					{/each}
				</select>
				{@render problem('reason', 'Stripe keeps this with the refund.')}
			</Field>

			<Field label="Note" id="refund-note" hint="For us only. The customer never sees it.">
				<textarea id="refund-note" rows="3" maxlength="500" bind:value={draft.note}></textarea>
			</Field>

			<label class="rf-check">
				<input type="checkbox" bind:checked={draft.sendEmail} disabled={payment.org_id === null} />
				<span>
					Email the customer
					{#if payment.org_id === null}
						<span class="hint">This payment isn't linked to an organisation.</span>
					{/if}
				</span>
			</label>

			<div class="rf-foot">
				<Button onclick={() => sheet?.close()}>Cancel</Button>
				<Button tier="primary" type="submit" icon="chevron-right">Review refund</Button>
			</div>
		{/if}
	</form>
</Sheet>

<style>
	.rf-head {
		display: grid;
		grid-template-columns: 1fr auto;
		gap: var(--s-3);
		align-items: start;
		padding: var(--s-4) var(--s-4) var(--s-3);
		border-bottom: 1px solid var(--line);
	}

	.rf-who {
		display: grid;
		gap: 2px;
		min-width: 0;
	}

	.rf-who h2 {
		margin: 0;
		font-size: 16px;
		font-weight: 600;
		overflow-wrap: anywhere;
	}

	.rf-sub {
		color: var(--muted);
		font-size: 13px;
	}

	.rf-close {
		display: inline-grid;
		place-items: center;
		width: 34px;
		height: 34px;
		border: 0;
		border-radius: var(--r-pill);
		background: transparent;
		color: var(--muted);
		cursor: pointer;
	}

	.rf-close:hover {
		background: var(--hover);
		color: var(--ink);
	}

	.rf-body {
		flex: 1;
		overflow-y: auto;
		padding: var(--s-4) var(--s-4) calc(var(--s-4) + env(safe-area-inset-bottom));
		display: grid;
		align-content: start;
		gap: var(--s-4);
	}

	.rf-body :global(input[type='number']),
	.rf-body select,
	.rf-body textarea {
		width: 100%;
		box-sizing: border-box;
		min-height: var(--control-h);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		background: var(--card);
		padding: 8px 11px;
		font: inherit;
		font-size: 13px;
		color: var(--ink);
	}

	.rf-body :global([aria-invalid='true']) {
		border-color: var(--bad);
	}

	.rf-body .hint {
		font-size: 12.5px;
		color: var(--muted);
	}

	.rf-body .hint.rf-error {
		display: inline-flex;
		align-items: center;
		gap: var(--s-1);
		font-weight: 500;
		color: var(--bad-ink);
	}

	.rf-check {
		display: flex;
		align-items: flex-start;
		gap: var(--s-2);
		font-size: 13.5px;
		font-weight: 500;
		cursor: pointer;
	}

	.rf-check > span {
		display: grid;
		gap: 2px;
	}

	.rf-confirm {
		margin: 0;
		font-size: 17px;
		font-weight: 600;
		line-height: 1.4;
	}

	.rf-body .quiet {
		margin: 0;
		font-size: 13px;
	}

	.rf-foot {
		display: flex;
		justify-content: flex-end;
		flex-wrap: wrap;
		gap: var(--s-2);
		padding-top: var(--s-3);
		border-top: 1px solid var(--line);
	}
</style>
