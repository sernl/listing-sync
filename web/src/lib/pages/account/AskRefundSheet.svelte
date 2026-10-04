<script lang="ts">
	// Asking for a refund: pick a payment from the last year, read what the
	// refund policy says it is owed, and ask for that. A payment the policy
	// owes nothing says why, with no button to press.
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import Icon from '$lib/Icon.svelte';
	import { queryKeys } from '$lib/query';
	import Sheet, { type SheetHandle } from '$lib/Sheet.svelte';
	import { NOTE_MAX, askBody, askOffer, paymentRows } from '$lib/pages/account/refunds';

	let { onClose }: { onClose: () => void } = $props();

	const queryClient = useQueryClient();
	let sheet = $state<SheetHandle>();

	const refunds = createQuery(() => ({
		queryKey: queryKeys.billingRefunds,
		queryFn: () => api.billingRefunds()
	}));
	const rows = $derived(paymentRows(refunds.data?.payments ?? []));

	let picked = $state<string | null>(null);
	let note = $state('');
	let asked = $state(false);
	let refusal = $state<string | null>(null);

	const quote = createQuery(() => ({
		queryKey: queryKeys.billingRefundQuote(picked ?? ''),
		queryFn: () => api.billingRefundQuote(picked ?? ''),
		enabled: picked !== null,
		staleTime: 0,
		refetchOnWindowFocus: false
	}));
	const offer = $derived(quote.data === undefined ? null : askOffer(quote.data));

	const asking = createMutation(() => ({
		mutationFn: (charge: string) => api.askRefund(askBody(charge, note)),
		onSuccess: async () => {
			asked = true;
			await queryClient.invalidateQueries({ queryKey: queryKeys.billingRefunds });
		},
		onError: (failure: Error) => {
			refusal =
				failure instanceof ApiFailure ? failure.message : 'Your request did not reach us. Try again.';
		}
	}));

	function pick(charge: string) {
		picked = charge;
		refusal = null;
	}

	function keepOpenWhileSending(event: Event) {
		if (asking.isPending) event.preventDefault();
	}
</script>

<Sheet labelledby="ask-title" {onClose} oncancel={keepOpenWhileSending} bind:handle={sheet}>
	<header class="ask-head">
		<h2 id="ask-title">Ask for a refund</h2>
		<button
			type="button"
			class="ask-close"
			aria-label="Close"
			disabled={asking.isPending}
			onclick={() => sheet?.close()}
		>
			<Icon name="x" />
		</button>
	</header>

	<form
		class="ask-body"
		novalidate
		onsubmit={(event) => {
			event.preventDefault();
			if (picked !== null && offer?.kind === 'ask') asking.mutate(picked);
		}}
	>
		{#if asked}
			<Banner tone="ok" title="Asked">Thanks — we'll email you when it's decided.</Banner>
			<div class="ask-foot">
				<Button tier="primary" onclick={() => sheet?.close()}>Close</Button>
			</div>
		{:else if refunds.isPending}
			<p class="quiet">Loading your payments…</p>
		{:else if refunds.isError}
			<Banner tone="bad" title="Your payments did not load">
				Try again in a moment.
				{#snippet action()}
					<Button
						small
						icon="refresh-cw"
						disabled={refunds.isFetching}
						reason={refunds.isFetching ? 'Loading.' : undefined}
						onclick={() => void refunds.refetch()}
					>
						Try again
					</Button>
				{/snippet}
			</Banner>
		{:else if rows.length === 0}
			<p class="ask-empty">There are no payments from the last year to ask about.</p>
			<div class="ask-foot">
				<Button onclick={() => sheet?.close()}>Close</Button>
			</div>
		{:else}
			<fieldset class="ask-payments">
				<legend>Which payment?</legend>
				{#each rows as row (row.id)}
					<label class="ask-payment" class:on={picked === row.id}>
						<input
							type="radio"
							name="ask-payment"
							value={row.id}
							checked={picked === row.id}
							disabled={asking.isPending}
							onchange={() => pick(row.id)}
						/>
						<span class="ask-payment-text">
							<span class="ask-payment-top">
								<b>{row.amount}</b>
								<span>{row.what}</span>
							</span>
							<span class="quiet">
								{row.date}{#if row.refunded !== null}
									· {row.refunded}{/if}
							</span>
						</span>
					</label>
				{/each}
			</fieldset>

			{#if refusal !== null}
				<Banner tone="bad" title="Not asked">{refusal}</Banner>
			{/if}

			{#if picked !== null}
				{#if quote.isPending}
					<p class="quiet">Checking the refund policy…</p>
				{:else if quote.isError}
					<Banner tone="bad" title="The refund policy did not load">
						Try again in a moment.
						{#snippet action()}
							<Button
								small
								icon="refresh-cw"
								disabled={quote.isFetching}
								reason={quote.isFetching ? 'Checking.' : undefined}
								onclick={() => void quote.refetch()}
							>
								Try again
							</Button>
						{/snippet}
					</Banner>
				{:else if quote.data !== undefined && offer !== null}
					<section class="ask-quote" aria-live="polite">
						<p>{quote.data.explanation}</p>
						{#if offer.kind === 'nothing'}
							<p class="quiet">If something isn't right, reply to any email from us.</p>
						{/if}
					</section>

					{#if offer.kind === 'ask'}
						<Field label="Anything we should know?" id="ask-note">
							<textarea
								id="ask-note"
								rows="3"
								maxlength={NOTE_MAX}
								bind:value={note}
								disabled={asking.isPending}
							></textarea>
						</Field>
					{/if}
				{/if}
			{/if}

			<div class="ask-foot">
				<Button
					disabled={asking.isPending}
					reason={asking.isPending ? 'Sending.' : undefined}
					onclick={() => sheet?.close()}
				>
					Cancel
				</Button>
				{#if offer?.kind === 'ask'}
					<Button
						tier="primary"
						type="submit"
						disabled={asking.isPending || quote.isFetching}
						reason={asking.isPending
							? 'Sending.'
							: quote.isFetching
								? 'Checking the refund policy.'
								: undefined}
					>
						{asking.isPending ? 'Asking…' : offer.button}
					</Button>
				{/if}
			</div>
		{/if}
	</form>
</Sheet>

<style>
	.ask-head {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: var(--s-3);
		padding: var(--s-4) var(--s-4) var(--s-3);
		border-bottom: 1px solid var(--line);
	}

	.ask-head h2 {
		margin: 0;
		font-size: 16px;
		font-weight: 600;
	}

	.ask-close {
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

	.ask-close:hover {
		background: var(--hover);
		color: var(--ink);
	}

	.ask-body {
		flex: 1;
		overflow-y: auto;
		padding: var(--s-4) var(--s-4) calc(var(--s-4) + env(safe-area-inset-bottom));
		display: grid;
		align-content: start;
		gap: var(--s-4);
	}

	.ask-body .quiet,
	.ask-empty {
		margin: 0;
		font-size: 13px;
	}

	.ask-payments {
		display: grid;
		gap: var(--s-2);
		margin: 0;
		padding: 0;
		border: 0;
		min-width: 0;
	}

	.ask-payments legend {
		padding: 0;
		margin-bottom: var(--s-2);
		font-size: 13px;
		font-weight: 600;
	}

	.ask-payment {
		display: flex;
		align-items: flex-start;
		gap: var(--s-2);
		padding: var(--s-3);
		background: var(--card);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		cursor: pointer;
	}

	.ask-payment.on {
		border-color: var(--accent);
	}

	.ask-payment input {
		margin-top: 3px;
	}

	.ask-payment-text {
		display: grid;
		gap: 2px;
		min-width: 0;
		font-size: 13.5px;
	}

	.ask-payment-top {
		display: flex;
		flex-wrap: wrap;
		gap: var(--s-1) var(--s-2);
		overflow-wrap: anywhere;
	}

	.ask-quote {
		display: grid;
		gap: var(--s-2);
		padding: var(--s-3);
		background: var(--surface);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
	}

	.ask-quote p {
		margin: 0;
		font-size: 13.5px;
	}

	.ask-body textarea {
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

	.ask-foot {
		display: flex;
		justify-content: flex-end;
		flex-wrap: wrap;
		gap: var(--s-2);
		padding-top: var(--s-3);
		border-top: 1px solid var(--line);
	}
</style>
