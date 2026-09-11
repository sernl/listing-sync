<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import { identity } from '$lib/auth-client';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { capabilityLines, grantNotice, migrationsLine, usageLines } from '$lib/entitlement';
	import { entitlementRead, readOf } from '$lib/entitlement-read';
	import { FOUNDING, IMPORT_LADDER, LADDER_ABOVE, PLANS, AI } from '$lib/generated/plans';
	import Icon from '$lib/Icon.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { PADDLE_CONFIG, openCheckout, planPriceKey, rungPriceKey } from '$lib/paddle';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import Toggle from '$lib/Toggle.svelte';
	import { paddleBadge } from '$lib/pages/account/tones';
	import {
		CHECKOUT_DORMANT,
		checkoutLabel,
		dollars,
		monthsFreeOnAnnual,
		periodLabel,
		priceOf,
		rungLabel,
		standingFor,
		type Cadence
	} from '$lib/pages/account/plans';
	import '$lib/pages/account/account.css';

	// The plan, under the key the shell already filled, so opening this page
	// asks nothing it has not asked.
	const entitlement = createQuery(() => entitlementRead);

	// The price table as the server holds it. The same figures are compiled
	// into `$lib/generated/plans` by `just web-typegen` and `just web-check`
	// fails if the two differ, so the generated copy stands while this read is
	// in flight rather than the page rendering a priceless grid for a moment.
	const served = createQuery(() => ({
		queryKey: queryKeys.plans,
		queryFn: () => api.plans(),
		staleTime: Infinity
	}));

	// Paddle's own record, which the plan panel still states: a grant says
	// what the seller is entitled to, and this says what the card is doing.
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

	const read = $derived(readOf(entitlement));
	const table = $derived(served.data?.plans ?? PLANS);
	const ladder = $derived(served.data?.import_ladder ?? IMPORT_LADDER);
	const above = $derived(served.data?.ladder_above ?? LADDER_ABOVE);
	const founding = $derived(served.data?.founding ?? FOUNDING);
	const ai = $derived(served.data?.ai ?? AI);
	const standing = $derived(standingFor(read, table));

	const held = $derived(entitlement.data);
	const caps = $derived(held?.capabilities);
	const usage = $derived(held?.usage);
	const notice = $derived(held === undefined ? null : grantNotice(held));

	const subscription = $derived(billing.data?.subscription ?? null);

	// The one recurring plan this deployment sells. `studio` ships priced and
	// unsold, so it is not a card here: a card for it would offer a plan no
	// checkout can buy.
	const recurring = $derived(table.find((plan) => plan.id === 'subscriber' && plan.sold));
	const monthsFree = $derived(recurring === undefined ? null : monthsFreeOnAnnual(recurring));

	// Null while the read has not answered, so the control cannot be rendered
	// over a standing that does not know what it would be changing.
	const label = $derived(checkoutLabel(read));

	let yearly = $state(false);
	const cadence = $derived<Cadence>(yearly ? 'annual' : 'monthly');

	// The controls render only where the build was given Paddle's client token
	// and price map. With neither, the sentence stands in their place and
	// Paddle's script is never loaded.
	const checkout = PADDLE_CONFIG;
	let opening = $state<string | null>(null);

	/** Why a purchase control cannot run, or null where it can.
	 *
	 *  The price key is checked here rather than at the click: a build whose
	 *  map is missing a rung must say so on the button, because a button that
	 *  looked buyable and then threw is the shape that gets reported as a
	 *  broken checkout. */
	function purchaseReason(priceKey: string): string | null {
		if (organisation.data === undefined) {
			return 'The organisation this would be billed to has not been read yet.';
		}
		if (checkout !== null && checkout.prices[priceKey] === undefined) {
			return 'This deployment has no price configured for that option.';
		}
		if (opening !== null) {
			return 'A checkout is opening.';
		}
		return null;
	}

	async function buy(priceKey: string) {
		const org = organisation.data?.id;
		if (checkout === null || org === undefined) {
			return;
		}
		opening = priceKey;
		try {
			await openCheckout(checkout, priceKey, org, profile.data?.email);
		} catch {
			toast('error', 'The checkout could not be opened.');
		} finally {
			opening = null;
		}
	}

	// "Talk to us" above the top rung. No support address reaches the console
	// — neither the session nor the organisation view carries one, and the
	// landing page's own is still null — so it lands on the guides rather than
	// on a `mailto:` that reaches nobody.
	const talkHref = '/guides';
</script>

<div class="page">
	<PageHead
		icon="credit-card"
		title="Subscription"
		description="What you are on, what it allows, and the two ways to buy more."
	/>

	<Panel title="Your plan" description="Read from the server, which is also what enforces it.">
		{#if entitlement.isPending}
			<p class="quiet">Loading…</p>
		{:else if entitlement.isError}
			<p class="quiet">We could not read your plan.</p>
		{:else}
			{#if notice !== null}
				<Banner tone="info" title="Set by Teachouse">
					{notice} If that is not what you expected, ask us before buying anything here.
				</Banner>
			{/if}

			<p>{standing.headline}</p>
			<p class="quiet">{standing.detail}</p>

			{#if caps !== undefined && usage !== undefined}
				<ul class="acct-usage">
					{#each usageLines(usage, caps) as row (row.limit)}
						<li class:acct-full={row.full}>
							<span class="acct-tick"><Icon name="circle-check" size={15} /></span>
							{row.line}
						</li>
					{/each}
					<li>
						<span class="acct-tick"><Icon name="arrow-right-left" size={15} /></span>
						{migrationsLine(usage, caps)}
					</li>
				</ul>
			{/if}

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

	{#if checkout === null}
		<Panel title="The two ways to buy">
			<p class="acct-checkout-note">{CHECKOUT_DORMANT}</p>
		</Panel>
	{:else}
		<Panel
			title="Subscribe"
			description="Everything, every month, with the migration allowance that keeps it going."
		>
			{#if recurring === undefined}
				<p class="quiet">No subscription is on sale in this deployment.</p>
			{:else}
				{@const price = priceOf(recurring, cadence)}
				{@const priceKey = planPriceKey(recurring.id, cadence)}
				{@const refusal = purchaseReason(priceKey)}
				<div class="acct-cadence">
					<Toggle bind:checked={yearly} label="Pay yearly" />
					{#if monthsFree !== null}
						<span class="saving">Paying yearly costs {monthsFree} months less.</span>
					{/if}
				</div>

				<div class="acct-plan">
					<div class="acct-plan-top">
						<span class="name">{recurring.name}</span>
						{#if standing.plan === recurring.id}
							<StatusPill tone="ok" label="current" />
						{/if}
					</div>
					<div class="price">
						<span class="n">{price.amount}</span>
						{#if price.per}<span class="per">{price.per}</span>{/if}
					</div>
					<div class="note">
						{#if recurring.trial_days > 0}
							{recurring.trial_days}-day free trial, card required
						{/if}
					</div>
					<ul>
						{#each capabilityLines(recurring.capabilities) as line (line)}
							<li>
								<span class="acct-tick"><Icon name="circle-check" size={15} /></span>
								{line}
							</li>
						{/each}
					</ul>
					{#if label !== null}
						<div class="actions">
							<Button
								tier="primary"
								disabled={refusal !== null}
								reason={refusal ?? undefined}
								onclick={() => buy(priceKey)}
							>
								{opening === priceKey ? 'Opening…' : label}
							</Button>
						</div>
					{:else}
						<p class="acct-checkout-note">{standing.detail}</p>
					{/if}
				</div>
			{/if}
		</Panel>

		<Panel
			title="Catalogue Import"
			description="A one-off purchase that brings a back catalogue in and publishes it once. No subscription."
		>
			<ul class="acct-ladder">
				{#each ladder as rung (rung.up_to)}
					{@const priceKey = rungPriceKey(rung.up_to)}
					{@const refusal = purchaseReason(priceKey)}
					<li>
						<span class="rung">{rungLabel(rung)}</span>
						<Button
							disabled={refusal !== null}
							reason={refusal ?? undefined}
							onclick={() => buy(priceKey)}
						>
							{opening === priceKey ? 'Opening…' : `Buy ${dollars(rung.price_cents)}`}
						</Button>
					</li>
				{/each}
				<li>
					<span class="rung">More than {ladder[ladder.length - 1]?.up_to} resources</span>
					<Button href={talkHref}>{above}</Button>
				</li>
			</ul>
			<p class="foot-note">
				The rung counts resources committed to your catalogue after duplicate merges, so 60
				listings that merge into 45 cost the 50 rung.
			</p>
		</Panel>

		<Panel title="Founding {founding.places}">
			<p>
				{founding.discount_year_one_pct}% off your first year, {founding.discount_ongoing_pct}% off
				for {founding.ongoing_years} years after that, and your first {founding.free_imports}
				resources imported free. {founding.places} places.
			</p>
			<p class="foot-note">
				Tell us when you subscribe and we apply it to your account. The ongoing discount ends
				after {founding.ongoing_years} years.
			</p>
		</Panel>

		{#if ai.status === 'coming_soon'}
			<Panel title="AI auto-fill">
				<p>
					Coming soon. Your subscription will include {ai.included_fills} fills a month, with {ai.add_on_fills}
					more for {dollars(ai.add_on_cents)}. Nothing is charged for it today.
				</p>
			</Panel>
		{/if}
	{/if}
</div>
