<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import { identity } from '$lib/auth-client';
	import Button from '$lib/Button.svelte';
	import Icon from '$lib/Icon.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { PADDLE_CONFIG, openCheckout } from '$lib/paddle';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import Toggle from '$lib/Toggle.svelte';
	import { paddleBadge } from '$lib/pages/account/tones';
	import {
		CHECKOUT_DORMANT,
		PLANS,
		checkoutLabel,
		periodLabel,
		priceOf,
		sharedMonthsFree,
		standingFor,
		type BillingRead,
		type Cadence
	} from '$lib/pages/account/plans';
	import '$lib/pages/account/account.css';

	const billing = createQuery(() => ({
		queryKey: queryKeys.billing,
		queryFn: () => api.billing()
	}));

	// The checkout needs the organisation the webhook will be told about, and
	// the address Paddle should prefill. Both share the keys the rest of the
	// console reads them under, so neither costs a second request.
	const organisation = createQuery(() => ({
		queryKey: queryKeys.org,
		queryFn: () => api.org()
	}));

	const profile = createQuery(() => ({
		queryKey: queryKeys.identity,
		queryFn: () => identity()
	}));

	// The read's three states, not two. `billing.data?.subscription ?? null` is
	// null while pending and null on failure as well as when there genuinely is
	// no subscription, and collapsing those showed a paying seller the free
	// tier in green and offered them a second one.
	const read = $derived<BillingRead>(
		billing.isSuccess && billing.data !== undefined
			? { state: 'read', subscription: billing.data.subscription ?? null }
			: { state: 'unread' }
	);
	const subscription = $derived(read.state === 'read' ? read.subscription : null);
	const standing = $derived(standingFor(read));

	// Null while the read has not answered, so the control cannot be rendered
	// over a standing that does not know what it would be changing.
	const label = $derived(checkoutLabel(read));
	const monthsFree = $derived(sharedMonthsFree());

	let yearly = $state(false);
	const cadence = $derived<Cadence>(yearly ? 'annual' : 'monthly');

	// The control renders only where the build was given Paddle's client token
	// and price. With neither, the sentence stands in its place and Paddle's
	// script is never loaded.
	const checkout = PADDLE_CONFIG;
	let opening = $state(false);

	async function subscribe() {
		const org = organisation.data?.id;
		if (checkout === null || org === undefined) {
			return;
		}
		opening = true;
		try {
			await openCheckout(checkout, org, profile.data?.email);
		} catch {
			toast('error', 'The checkout could not be opened.');
		} finally {
			opening = false;
		}
	}
</script>

<div class="page">
	<PageHead
		icon="credit-card"
		title="Subscription"
		description="What you are on, what it costs, and when it renews."
	/>

	<Panel title="Your plan" description="Read from the billing service, not from this page.">
		{#if billing.isPending}
			<p class="quiet">Loading…</p>
		{:else if billing.isError}
			<p class="quiet">We could not read your billing.</p>
		{:else}
			<p>{standing.headline}</p>
			<p class="quiet">{standing.detail}</p>
			{#if subscription !== null}
				<dl class="acct-detail">
					<dt>Status</dt>
					<dd>
						<StatusPill tone={paddleBadge(subscription.status)} label={subscription.status} />
					</dd>
					<dt>Current period ends</dt>
					<dd>{periodLabel(subscription)}</dd>
					<dt>Last recorded</dt>
					<dd>{new Date(subscription.occurred_at).toLocaleString()}</dd>
				</dl>
				<p class="foot-note">
					Subscription <span class="mono">{subscription.paddle_subscription_id}</span> · customer
					<span class="mono">{subscription.paddle_customer_id}</span>. The status is Paddle's own
					word for it, passed through rather than translated.
				</p>
			{/if}
		{/if}
	</Panel>

	<Panel title="Plans" description="What each plan lets you do. Every figure is per account.">
		<div class="acct-cadence">
			<Toggle bind:checked={yearly} label="Pay yearly" />
			{#if monthsFree !== null}
				<span class="saving">
					Paying yearly costs {monthsFree} months less on every paid plan.
				</span>
			{/if}
		</div>

		<div class="acct-plan-grid">
			{#each PLANS as plan (plan.id)}
				{@const price = priceOf(plan, cadence)}
				<div class="acct-plan" class:acct-current={standing.plan === plan.id}>
					<div class="acct-plan-top">
						<span class="name">{plan.name}</span>
						{#if standing.plan === plan.id}
							<StatusPill tone="ok" label="current" />
						{/if}
					</div>
					<p class="blurb">{plan.blurb}</p>
					<div class="price">
						<span class="n">{price.amount}</span>
						{#if price.per}<span class="per">{price.per}</span>{/if}
					</div>
					<!-- The trial takes precedence over the saving: the saving is
					     already stated above the grid for all three paid tiers, and
					     showing it here hid the one fact no other card or line
					     carries. -->
					<div class="note">
						{#if plan.trialDays !== undefined}
							{plan.trialDays}-day free trial
						{:else if cadence === 'annual' && monthsFree !== null}
							{monthsFree} months free against paying monthly
						{/if}
					</div>
					<ul>
						{#each plan.allowances as allowance (allowance)}
							<li>
								<span class="acct-tick"><Icon name="circle-check" size={15} /></span>
								{allowance}
							</li>
						{/each}
					</ul>
				</div>
			{/each}
		</div>

		<!-- One control for the one price this deployment sells. A button per
		     tier waits on a price per tier: four buttons over a single price id
		     would each charge the same amount for a different promise. -->
		{#if checkout === null}
			<p class="acct-checkout-note">{CHECKOUT_DORMANT}</p>
		{:else if label === null}
			<p class="acct-checkout-note">{standing.detail}</p>
		{:else}
			<div class="actions">
				<Button
					tier="primary"
					disabled={opening || organisation.data === undefined}
					reason={organisation.data === undefined
						? 'The organisation this would be billed to has not been read yet.'
						: opening
							? 'The checkout is opening.'
							: undefined}
					onclick={subscribe}
				>
					{opening ? 'Opening…' : label}
				</Button>
			</div>
			<p class="foot-note">
				We sell one price today, so this button opens checkout for that price.
			</p>
		{/if}

		<p class="foot-note">
			Nothing is limited by these figures yet — they are what each plan will allow when billing
			opens.
		</p>
	</Panel>
</div>
