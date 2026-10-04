<script lang="ts">
	// Sellers asking for the policy's refund, open ones first. An open one is
	// approved at its quote or declined with a reason; a decided one says who
	// decided it and when.

	import Button from '$lib/Button.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { money } from '$lib/pages/account/plans';
	import RefundRequestDialog from '$lib/pages/admin/RefundRequestDialog.svelte';
	import {
		basisLabel,
		decidedLine,
		nzShortDate,
		requestPill,
		requestsInOrder,
		type RefundRequestView
	} from '$lib/pages/admin/payments';

	let {
		requests,
		decidedBy,
		blocked,
		onDecided
	}: {
		requests: readonly RefundRequestView[];
		/** The operator's name, as the refund panel sends it. */
		decidedBy: string;
		/** Why nothing can be refunded here, where nothing can. */
		blocked: string | null;
		onDecided: (decided: RefundRequestView) => Promise<void>;
	} = $props();

	const ordered = $derived(requestsInOrder(requests));
	let deciding = $state<{ request: RefundRequestView; kind: 'approve' | 'decline' } | null>(null);
</script>

<section class="rq-section" aria-labelledby="rq-title">
	<h2 id="rq-title" class="rq-h2">Refund requests</h2>
	<ul class="rq-list">
		{#each ordered as request (request.id)}
			{@const pill = requestPill(request)}
			{@const decided = decidedLine(request)}
			<li class="rq-item">
				<div class="rq-main">
					<span class="rq-top">
						<b class="rq-org">{request.org_name ?? request.org_id}</b>
						<b>{money(request.quoted_cents, request.currency)}</b>
						<StatusPill tone={pill.tone} label={pill.label} />
					</span>
					<span class="quiet">
						Asked {nzShortDate(request.created_at)} · {basisLabel(request.policy_basis)}
					</span>
					{#if request.note}<span class="rq-note">“{request.note}”</span>{/if}
					{#if decided !== null}<span class="quiet">{decided}</span>{/if}
					{#if request.decline_reason}
						<span class="rq-declined">Why: {request.decline_reason}</span>
					{/if}
				</div>
				{#if request.status === 'requested'}
					<div class="rq-acts">
						<Button
							small
							onclick={() => (deciding = { request, kind: 'decline' })}
							disabled={blocked !== null}
							reason={blocked ?? undefined}
						>
							Decline
						</Button>
						<Button
							small
							tier="primary"
							icon="check"
							onclick={() => (deciding = { request, kind: 'approve' })}
							disabled={blocked !== null}
							reason={blocked ?? undefined}
						>
							Approve
						</Button>
					</div>
				{/if}
			</li>
		{/each}
	</ul>
</section>

{#if deciding !== null}
	<RefundRequestDialog
		request={deciding.request}
		kind={deciding.kind}
		{decidedBy}
		onClose={() => (deciding = null)}
		{onDecided}
	/>
{/if}

<style>
	.rq-section {
		display: flex;
		flex-direction: column;
		gap: var(--s-3);
		min-width: 0;
	}

	.rq-h2 {
		margin: 0;
		font-family: var(--display);
		font-size: 19px;
		font-weight: 600;
		letter-spacing: -0.01em;
	}

	.rq-list {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: var(--s-2);
	}

	.rq-item {
		display: flex;
		flex-wrap: wrap;
		justify-content: space-between;
		align-items: flex-start;
		gap: var(--s-2) var(--s-4);
		padding: var(--s-3) var(--s-4);
		background: var(--card);
		border: 1px solid var(--line);
		border-radius: var(--r-panel);
	}

	.rq-main {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
		font-size: 13px;
	}

	.rq-top {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--s-1) var(--s-2);
		font-size: 14px;
	}

	.rq-org {
		overflow-wrap: anywhere;
	}

	.rq-note {
		font-style: italic;
		overflow-wrap: anywhere;
	}

	.rq-declined {
		color: var(--muted);
		overflow-wrap: anywhere;
	}

	.rq-acts {
		display: flex;
		flex-wrap: wrap;
		gap: var(--s-2);
	}

	@media (max-width: 620px) {
		.rq-acts {
			width: 100%;
			justify-content: flex-end;
		}
	}
</style>
