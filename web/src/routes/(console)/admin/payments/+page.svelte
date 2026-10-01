<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import { identity } from '$lib/auth-client';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import Icon from '$lib/Icon.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatCard from '$lib/StatCard.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import Toggle from '$lib/Toggle.svelte';
	import PaymentsRefundSheet from '$lib/pages/admin/PaymentsRefundSheet.svelte';
	import {
		GROUP_LABEL,
		KIND_GROUPS,
		KIND_LABEL,
		NO_FILTERS,
		REASON_LABEL,
		eventAmount,
		filterEvents,
		filtersActive,
		mailButton,
		monthStats,
		nzMonthLabel,
		nzShortDate,
		nzTime,
		refundOf,
		refundPill,
		rowPill,
		relatedEvents,
		relatedRefunds,
		remaining,
		statusOptions,
		statusWords,
		tallyAmount,
		type KindGroup,
		type PaymentEventView,
		type PaymentFilters,
		type PaymentsAdminView,
		type RefundReason,
		type RefundView
	} from '$lib/pages/admin/payments';
	import '$lib/flow.css';
	import '$lib/pages/admin/admin.css';
	import '$lib/styles/data.css';

	const queryClient = useQueryClient();

	const payments = createQuery(() => ({
		queryKey: queryKeys.adminPayments,
		queryFn: () => api.adminPayments()
	}));

	// The name a refund's log line carries, as the Mail page sends it.
	const who = createQuery(() => ({
		queryKey: queryKeys.identity,
		queryFn: () => identity()
	}));
	const issuedBy = $derived(who.data?.name.trim() || who.data?.email || 'an admin');

	const now = Date.now();
	const events = $derived(payments.data?.events ?? []);
	const refunds = $derived(payments.data?.refunds ?? []);
	const stripeless = $derived(payments.data?.stripe_configured === false);
	const stats = $derived(monthStats(events, refunds, now));
	const statuses = $derived(statusOptions(events));

	let filters = $state<PaymentFilters>({ ...NO_FILTERS });
	const shown = $derived(filterEvents(events, filters));

	const NO_STRIPE = 'This server has no Stripe key, so nothing can be refunded or synced here.';

	const left = (event: PaymentEventView) =>
		remaining(event.charge_id ?? event.provider_object_id, events, refunds);

	let open = $state<string | null>(null);
	let refunding = $state<PaymentEventView | null>(null);

	const invalidate = () => queryClient.invalidateQueries({ queryKey: queryKeys.adminPayments });

	const syncing = createMutation(() => ({
		mutationFn: () => api.syncPayments(),
		onSuccess: async (sync) => {
			await invalidate();
			toast(
				'success',
				`Read ${sync.charges} charges, ${sync.refunds} refunds, ${sync.disputes} chargebacks and ${sync.invoices} invoices from Stripe.`
			);
		},
		onError: (failure: Error) =>
			toast('error', failure instanceof ApiFailure ? failure.message : 'The sync did not finish.')
	}));

	const setting = createMutation(() => ({
		mutationFn: (on: boolean) => api.setPaymentSettings({ auto_refund_mail: on }),
		onSuccess: (saved) => {
			queryClient.setQueryData<PaymentsAdminView>(queryKeys.adminPayments, (held) =>
				held === undefined ? held : { ...held, auto_refund_mail: saved.auto_refund_mail }
			);
			toast(
				'success',
				saved.auto_refund_mail
					? 'Customers now get an email when you refund them.'
					: 'Refunds no longer email the customer unless you tick the box.'
			);
		},
		onError: async (failure: Error) => {
			await invalidate();
			toast('error', failure instanceof ApiFailure ? failure.message : 'It was not saved.');
		}
	}));

	let mailingId = $state<string | null>(null);
	const mailing = createMutation(() => ({
		mutationFn: (id: string) => api.mailRefund(id),
		onSuccess: async () => {
			mailingId = null;
			await invalidate();
			toast('success', 'The refund email is on its way.');
		},
		onError: (failure: Error) => {
			mailingId = null;
			toast('error', failure instanceof ApiFailure ? failure.message : 'The email was not queued.');
		}
	}));

	function mail(id: string) {
		mailingId = id;
		mailing.mutate(id);
	}

	async function copy(id: string) {
		try {
			await navigator.clipboard.writeText(id);
			toast('success', 'Copied');
		} catch {
			toast('error', 'Your browser would not copy it. Select the text instead.');
		}
	}

	const GROUPS = Object.keys(KIND_GROUPS) as KindGroup[];
</script>

{#snippet refundLine(refund: RefundView)}
	{@const button = mailButton(refund)}
	{@const pill = refundPill(refund)}
	<li class="pay-refund">
		<div class="pay-refund-main">
			<span class="pay-refund-top">
				<b>{eventAmount(refund)}</b>
				<StatusPill tone={pill.tone} label={pill.label} />
				<span class="quiet">{nzShortDate(refund.created_at)}</span>
			</span>
			<span class="quiet">
				{refund.reason === null
					? 'No reason given'
					: (REASON_LABEL[refund.reason as RefundReason] ?? statusWords(refund.reason))} ·
				{refund.issued_by === null ? "made in Stripe's dashboard" : `by ${refund.issued_by}`}
			</span>
			{#if refund.note}<span class="pay-note">“{refund.note}”</span>{/if}
			<span class="mono pay-id">{refund.provider_refund_id}</span>
		</div>
		<div class="pay-refund-mail">
			{#if button.failure !== null}
				<span class="pay-unsent" title={button.failure}>
					Not sent yet
					<Explain title="Why the email has not gone out" label="" tone="warn">
						<p>The last try said: {button.failure}</p>
						<p>Press Send email to try again.</p>
					</Explain>
				</span>
			{/if}
			<Button
				small
				icon="mail"
				disabled={button.disabled || mailing.isPending}
				reason={button.reason ?? (mailing.isPending ? 'Queueing one already.' : undefined)}
				onclick={() => mail(refund.id)}
			>
				{mailingId === refund.id ? 'Queueing…' : button.label}
			</Button>
		</div>
	</li>
{/snippet}

<div class="page flow-page">
	<PageHead
		icon="credit-card"
		title="Payments"
		description="Every Stripe payment, refund and chargeback, and refunds you can give from here."
	>
		{#snippet aside()}
			<Button
				icon="refresh-cw"
				disabled={stripeless || syncing.isPending || payments.isPending}
				reason={stripeless
					? NO_STRIPE
					: syncing.isPending
						? 'Syncing.'
						: payments.isPending
							? 'Loading.'
							: undefined}
				onclick={() => syncing.mutate()}
			>
				{syncing.isPending ? 'Syncing…' : 'Sync from Stripe'}
			</Button>
		{/snippet}
	</PageHead>

	{#if payments.isPending}
		<p class="quiet">Loading payments…</p>
	{:else if payments.isError}
		<Placeholder
			icon="circle-alert"
			headline="We could not load payments"
			body="Try reloading the page."
		/>
	{:else}
		<div class="pay-stack">
			{#if stripeless}
				<Banner tone="warn" title="No Stripe key">
					You can read payments here but not refund or sync them.
				</Banner>
			{/if}

			<section class="pay-section" aria-labelledby="pay-month">
				<div class="pay-head">
					<h2 id="pay-month">This month</h2>
					<span class="quiet">{nzMonthLabel(now)}, New Zealand time</span>
				</div>
				<div class="op-stats">
					<StatCard
						icon="credit-card"
						tone="ok"
						label="Paid"
						sub={`${stats.paid.count} ${stats.paid.count === 1 ? 'payment' : 'payments'}`}
					>
						{tallyAmount(stats.paid)}
					</StatCard>
					<StatCard
						icon="arrow-left"
						label="Refunded"
						sub={`${stats.refunded.count} ${stats.refunded.count === 1 ? 'refund' : 'refunds'}`}
					>
						{tallyAmount(stats.refunded)}
					</StatCard>
					<StatCard
						icon="triangle-alert"
						tone={stats.disputed.count > 0 ? 'bad' : ''}
						label="Disputed"
						sub={`${stats.disputed.count} ${stats.disputed.count === 1 ? 'chargeback' : 'chargebacks'}`}
					>
						{tallyAmount(stats.disputed)}
					</StatCard>
				</div>
			</section>

			<div class="pay-setting">
				<Toggle
					label="Email customers when I refund"
					checked={payments.data.auto_refund_mail}
					disabled={setting.isPending}
					onchange={(on) => setting.mutate(on)}
				/>
				<Explain title="Refund emails" label="">
					<p>
						With this on, the Email the customer box starts ticked on every refund. You can still
						untick it for one refund, or send the email later from the payment's details.
					</p>
					<p>Refunds made in Stripe's dashboard never send our email by themselves.</p>
				</Explain>
			</div>

			{#if events.length === 0}
				<Placeholder
					icon="credit-card"
					headline="No payments yet"
					body="Sync from Stripe to read the last 90 days."
				/>
			{:else}
				<section class="pay-section" aria-labelledby="pay-list">
					<h2 id="pay-list" class="pay-h2">Every payment</h2>
					<div class="pay-filters">
						<label class="pay-filter">
							<span>Kind</span>
							<select bind:value={filters.kind}>
								<option value="all">Every kind</option>
								<optgroup label="Groups">
									{#each GROUPS as group (group)}
										<option value={group}>{GROUP_LABEL[group]}</option>
									{/each}
								</optgroup>
								<optgroup label="Exactly">
									{#each GROUPS as group (group)}
										{#each KIND_GROUPS[group] as kind (kind)}
											<option value={kind}>{KIND_LABEL[kind]}</option>
										{/each}
									{/each}
								</optgroup>
							</select>
						</label>
						<label class="pay-filter">
							<span>Status</span>
							<select bind:value={filters.status}>
								<option value="all">Every status</option>
								{#each statuses as status (status)}
									<option value={status}>{statusWords(status)}</option>
								{/each}
							</select>
						</label>
						<label class="pay-filter pay-filter-org">
							<span>Organisation</span>
							<input type="search" placeholder="Name or id" bind:value={filters.org} />
						</label>
						<label class="pay-filter">
							<span>From</span>
							<input type="date" bind:value={filters.from} max={filters.to || undefined} />
						</label>
						<label class="pay-filter">
							<span>To</span>
							<input type="date" bind:value={filters.to} min={filters.from || undefined} />
						</label>
						{#if filtersActive(filters)}
							<Button small icon="x" onclick={() => (filters = { ...NO_FILTERS })}>Clear</Button>
						{/if}
					</div>

					{#if shown.length === 0}
						<p class="data-empty">No payments match these filters.</p>
					{:else}
						<div class="data-table-wrap">
							<table class="data-table stack pay-table">
								<thead>
									<tr>
										<th scope="col">Date</th>
										<th scope="col">Org</th>
										<th scope="col">Kind</th>
										<th scope="col" class="num">Amount</th>
										<th scope="col">Status</th>
										<th scope="col">Reference</th>
										<th scope="col" class="act">Actions</th>
									</tr>
								</thead>
								<tbody>
									{#each shown as event (event.id)}
										{@const isPayment = event.kind === 'payment_succeeded'}
										{@const leftCents = isPayment ? left(event) : null}
										{@const linked = refundOf(event, refunds)}
										{@const pill = rowPill(event, events, refunds)}
										{@const expanded = open === event.id}
										<tr class:pay-open={expanded}>
											<td class="nowrap" data-label="Date">
												<span>
													{nzShortDate(event.occurred_at)}
													<span class="sub">{nzTime(event.occurred_at)}</span>
												</span>
											</td>
											<td data-label="Org">
												{#if event.org_name !== null || event.org_id !== null}
													<span class="pay-org">{event.org_name ?? event.org_id}</span>
												{:else}
													<span class="quiet">No organisation</span>
												{/if}
											</td>
											<td data-label="Kind">{KIND_LABEL[event.kind]}</td>
											<td class="num" data-label="Amount">
												<span>
													{eventAmount(event)}
													{#if leftCents !== null && event.amount_cents !== null && leftCents > 0 && leftCents < event.amount_cents}
														<span class="sub"
															>{eventAmount({ amount_cents: leftCents, currency: event.currency })} left</span
														>
													{/if}
												</span>
											</td>
											<td data-label="Status">
												<span>
													<StatusPill tone={pill.tone} label={pill.label} />
													{#if linked !== null}
														<span class="sub">
															{linked.issued_by === null ? 'In Stripe' : `By ${linked.issued_by}`}
														</span>
													{/if}
												</span>
											</td>
											<td data-label="Reference">
												<span class="pay-ref">
													<span class="mono pay-id">{event.provider_object_id}</span>
													<button
														type="button"
														class="pay-icon"
														aria-label="Copy {event.provider_object_id}"
														title="Copy"
														onclick={() => copy(event.provider_object_id)}
													>
														<Icon name="copy" size={14} />
													</button>
												</span>
											</td>
											<td class="act" data-label="">
												<span class="op-acts">
													{#if isPayment && leftCents !== null && leftCents > 0}
														<Button
															small
															danger
															icon="arrow-left"
															disabled={stripeless}
															reason={stripeless ? NO_STRIPE : undefined}
															onclick={() => (refunding = event)}
														>
															Refund
														</Button>
													{/if}
													<button
														type="button"
														class="pay-expand"
														aria-expanded={expanded}
														aria-controls="pay-more-{event.id}"
														onclick={() => (open = expanded ? null : event.id)}
													>
														{expanded ? 'Hide' : 'Details'}
														<Icon name="chevron-down" size={14} />
													</button>
												</span>
											</td>
										</tr>
										{#if expanded}
											{@const related = relatedEvents(event, events)}
											{@const onCharge = relatedRefunds(event, refunds)}
											<tr class="pay-more" id="pay-more-{event.id}">
												<td colspan="7" data-label="">
													<div class="pay-more-grid">
														<section>
															<h3>About the same money</h3>
															{#if related.length === 0}
																<p class="quiet">
																	Nothing else in the ledger is about this payment.
																</p>
															{:else}
																<ul class="pay-related">
																	{#each related as other (other.id)}
																		{@const otherPill = rowPill(other, events, refunds)}
																		<li>
																			<span class="quiet">{nzShortDate(other.occurred_at)}</span>
																			<span>{KIND_LABEL[other.kind]}</span>
																			<b>{eventAmount(other)}</b>
																			<StatusPill tone={otherPill.tone} label={otherPill.label} />
																			<span class="mono pay-id">{other.provider_object_id}</span>
																		</li>
																	{/each}
																</ul>
															{/if}
															{#if event.reason}
																<p class="pay-reason">Stripe says: {event.reason}</p>
															{/if}
														</section>
														<section>
															<h3>Refunds on this charge</h3>
															{#if onCharge.length === 0}
																<p class="quiet">No refunds on this charge.</p>
															{:else}
																<ul class="pay-refunds">
																	{#each onCharge as refund (refund.id)}{@render refundLine(
																			refund
																		)}{/each}
																</ul>
															{/if}
														</section>
													</div>
												</td>
											</tr>
										{/if}
									{/each}
								</tbody>
							</table>
						</div>
					{/if}
				</section>
			{/if}
		</div>

		{#if refunding !== null}
			<PaymentsRefundSheet
				payment={refunding}
				left={left(refunding)}
				autoMail={payments.data.auto_refund_mail}
				{issuedBy}
				onClose={() => (refunding = null)}
				onRefunded={invalidate}
			/>
		{/if}
	{/if}
</div>

<style>
	.pay-stack {
		display: flex;
		flex-direction: column;
		gap: var(--s-5);
		min-width: 0;
	}

	/* Seven columns: the default padding left Actions a few pixels outside
	   the card at 1280px, as the Users table found. */
	.pay-table th,
	.pay-table td {
		padding-inline: var(--s-3);
	}

	/* Stacked rather than side by side: two buttons across is the widest
	   cell in the row, and at 1280px it pushed Actions out of the card. */
	.pay-table .op-acts {
		flex-direction: column;
		align-items: flex-end;
	}

	.pay-section {
		display: flex;
		flex-direction: column;
		gap: var(--s-3);
		min-width: 0;
	}

	.pay-head {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: var(--s-1) var(--s-3);
	}

	.pay-head h2,
	.pay-h2 {
		margin: 0;
		font-family: var(--display);
		font-size: 19px;
		font-weight: 600;
		letter-spacing: -0.01em;
	}

	.pay-head .quiet {
		font-size: 13px;
	}

	.pay-setting {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		padding: var(--s-3) var(--s-4);
		background: var(--card);
		border: 1px solid var(--line);
		border-radius: var(--r-panel);
	}

	.pay-filters {
		display: flex;
		flex-wrap: wrap;
		align-items: flex-end;
		gap: var(--s-2);
	}

	.pay-filter {
		display: flex;
		flex-direction: column;
		gap: 4px;
		min-width: 0;
		font-size: 12px;
		font-weight: 600;
		color: var(--muted);
	}

	.pay-filter-org {
		flex: 1 1 200px;
	}

	.pay-filter select,
	.pay-filter input {
		flex: none;
		box-sizing: border-box;
		width: 100%;
		min-height: var(--control-h);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		background: var(--card);
		padding: 6px 10px;
		font: inherit;
		font-size: 13px;
		font-weight: 400;
		color: var(--ink);
	}

	.pay-org {
		font-weight: 500;
		overflow-wrap: break-word;
	}

	.pay-ref {
		display: inline-flex;
		align-items: center;
		gap: var(--s-1);
		min-width: 0;
	}

	.pay-id {
		font-size: 12px;
		color: var(--muted);
		white-space: nowrap;
	}

	.pay-icon,
	.pay-expand {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		border: 1px solid transparent;
		border-radius: var(--r-field);
		background: transparent;
		color: var(--muted);
		font: inherit;
		font-size: 12.5px;
		font-weight: 600;
		cursor: pointer;
	}

	.pay-icon {
		flex: none;
		justify-content: center;
		width: 28px;
		height: 28px;
	}

	.pay-expand {
		min-height: 30px;
		padding: 0 var(--s-2);
		border-color: var(--line);
	}

	.pay-icon:hover,
	.pay-expand:hover {
		background: var(--hover);
		color: var(--ink);
	}

	.pay-icon:focus-visible,
	.pay-expand:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 1px;
	}

	.pay-expand[aria-expanded='true'] :global(svg) {
		transform: rotate(180deg);
	}

	.data-table tr.pay-open td {
		background: var(--hover);
	}

	.data-table tr.pay-more td {
		background: var(--surface);
		padding: var(--s-4);
	}

	/* Sized by the row it sits under rather than by its contents: a table
	   cell spanning every column would otherwise widen the whole table. */
	.pay-more-grid {
		width: 0;
		min-width: 100%;
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: var(--s-4);
		text-align: left;
	}

	@media (min-width: 900px) {
		.pay-more-grid {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}

	.pay-more-grid section {
		display: flex;
		flex-direction: column;
		gap: var(--s-2);
		min-width: 0;
	}

	.pay-more-grid h3 {
		margin: 0;
		font-size: 12px;
		font-weight: 600;
		letter-spacing: 0.04em;
		text-transform: uppercase;
		color: var(--muted);
	}

	.pay-more-grid p {
		margin: 0;
		font-size: 13px;
	}

	.pay-related,
	.pay-refunds {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: var(--s-2);
	}

	.pay-related li {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--s-1) var(--s-3);
	}

	.pay-refund {
		display: flex;
		flex-wrap: wrap;
		justify-content: space-between;
		gap: var(--s-2) var(--s-4);
		padding: var(--s-3);
		background: var(--card);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
	}

	.pay-refund-main {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}

	.pay-refund-top {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--s-2);
	}

	.pay-note {
		font-style: italic;
	}

	.pay-refund-mail {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--s-2);
	}

	.pay-unsent {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		font-size: 12.5px;
		font-weight: 600;
		color: var(--warn-ink);
	}

	.pay-reason {
		color: var(--muted);
	}

	@media (max-width: 620px) {
		.pay-filter {
			flex: 1 1 calc(50% - var(--s-2));
		}

		.pay-filter-org {
			flex-basis: 100%;
		}

		/* The details cell reads as a block under its card, not a label/value line. */
		.data-table tr.pay-more td {
			display: block;
			padding: var(--s-3) var(--s-4);
		}

		.pay-refund-mail {
			width: 100%;
		}

		.pay-id {
			white-space: normal;
			overflow-wrap: anywhere;
		}

		.pay-table .op-acts {
			flex-direction: row;
		}
	}
</style>
