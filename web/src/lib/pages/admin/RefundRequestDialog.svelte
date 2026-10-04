<script lang="ts">
	// Deciding one seller's refund request. Approve issues the quoted refund,
	// emails them and ends a yearly plan today; Decline needs a reason, which
	// they are emailed. Either way the host re-reads the page.

	import { ApiFailure, api } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { lightDismiss } from '$lib/dismiss';
	import Field from '$lib/Field.svelte';
	import Icon from '$lib/Icon.svelte';
	import {
		approveSentence,
		declineBody,
		declineProblem,
		type RefundRequestView
	} from '$lib/pages/admin/payments';

	let {
		request,
		kind,
		decidedBy,
		onClose,
		onDecided
	}: {
		request: RefundRequestView;
		kind: 'approve' | 'decline';
		/** The operator's name, as the refund panel sends it. */
		decidedBy: string;
		onClose: () => void;
		onDecided: (decided: RefundRequestView) => Promise<void>;
	} = $props();

	let element = $state<HTMLDialogElement | null>(null);
	let reason = $state('');
	let tried = $state(false);
	let working = $state(false);
	let refusal = $state<string | null>(null);

	const problem = $derived(tried ? declineProblem(reason) : null);
	const approving = $derived(kind === 'approve');

	$effect(() => {
		if (element !== null && !element.open) element.showModal();
	});

	async function decide() {
		if (working) return;
		tried = true;
		if (kind === 'decline' && declineProblem(reason) !== null) return;
		working = true;
		refusal = null;
		try {
			const decided =
				kind === 'approve'
					? await api.approveRefundRequest(request.id, { decided_by_label: decidedBy })
					: await api.declineRefundRequest(request.id, declineBody(reason, decidedBy));
			await onDecided(decided);
			element?.close();
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure
					? failure.message
					: kind === 'approve'
						? 'The refund did not go through.'
						: 'The request was not declined.';
		} finally {
			working = false;
		}
	}
</script>

<dialog
	use:lightDismiss
	bind:this={element}
	aria-labelledby="request-title"
	onclose={onClose}
	oncancel={(event) => {
		if (working) event.preventDefault();
	}}
>
	<div class="dialog-body">
		{#if kind === 'approve'}
			<h2 id="request-title">Approve this refund?</h2>
			<p>{approveSentence(request)}</p>
		{:else}
			<h2 id="request-title">Decline {request.org_name ?? 'this request'}?</h2>
			<Field label="Why?" id="decline-reason" required>
				<textarea
					id="decline-reason"
					class="rq-reason"
					rows="3"
					maxlength="1000"
					bind:value={reason}
					disabled={working}
					aria-invalid={problem !== null}
				></textarea>
				{#if problem === null}
					<span class="hint">We email this to them.</span>
				{:else}
					<span class="hint rq-error" role="alert">
						<Icon name="circle-alert" size={13} />
						{problem}
					</span>
				{/if}
			</Field>
		{/if}

		{#if refusal !== null}
			<Banner tone="bad" title={kind === 'approve' ? 'Not refunded' : 'Not declined'}>
				{refusal}
			</Banner>
		{/if}

		<div class="actions">
			<Button
				disabled={working}
				reason={working ? 'Sending.' : undefined}
				onclick={() => element?.close()}
			>
				Cancel
			</Button>
			<Button
				tier="primary"
				danger={approving}
				icon={approving ? 'check' : 'x'}
				disabled={working}
				reason={working ? 'Sending.' : undefined}
				onclick={decide}
			>
				{#if kind === 'approve'}
					{working ? 'Refunding…' : 'Approve'}
				{:else}
					{working ? 'Declining…' : 'Decline'}
				{/if}
			</Button>
		</div>
	</div>
</dialog>

<style>
	.rq-reason {
		width: 100%;
		box-sizing: border-box;
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		background: var(--card);
		padding: 8px 11px;
		font: inherit;
		font-size: 13px;
		color: var(--ink);
	}

	.rq-reason[aria-invalid='true'] {
		border-color: var(--bad);
	}

	.hint {
		font-size: 12.5px;
		color: var(--muted);
	}

	.rq-error {
		display: inline-flex;
		align-items: center;
		gap: var(--s-1);
		font-weight: 500;
		color: var(--bad-ink);
	}
</style>
