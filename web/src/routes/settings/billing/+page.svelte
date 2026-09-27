<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { ApiFailure, api, type BillingView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { PLANS } from '$lib/generated/plans';
	import { type PriceKey } from '$lib/generated/vocab';
	import Icon from '$lib/Icon.svelte';
	import Note from '$lib/Note.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { capture } from '$lib/posthog';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import {
		bestValuePack,
		cardLabel,
		checkoutOutcome,
		dayLabel,
		dollars,
		expiryLine,
		invoiceStatus,
		money,
		moves,
		packsBySize,
		paidPlans,
		planBullets,
		planMeaning,
		termLine,
		tierPrice,
		type Cadence
	} from '$lib/pages/account/plans';
	import CancelPlanDialog from '$lib/pages/account/CancelPlanDialog.svelte';
	import { readIntent } from '$lib/pages/account/intent';
	import { afterPercentOff, saleLine, salePrice } from '$lib/sale';
	import '$lib/flow.css';
	import '$lib/pages/account/account.css';

	// Everything priced on this page comes from the generated table, which is
	// `tam-limits`' own and the same figures the checkout charges. Nothing
	// here writes a price as a literal: a number typed into markup is a price
	// that drifts from the one Stripe takes.
	const look = PLANS.find((plan) => plan.id === 'free') ?? null;
	const tiers = paidPlans();
	const packs = packsBySize();
	const best = bestValuePack();

	// The card lines are the landing page's pricing cards, read off the same
	// capabilities, so a seller reads one promise on both.
	const lookBullets = look === null ? [] : planBullets(look.capabilities);
	const editDays = look?.capabilities.pack_edit_days ?? null;

	// Which of each tier's two prices the cards show and buy. Yearly first,
	// because it is the price every card's headline is compared by.
	let cadence = $state<Cadence>('yearly');

	const queryClient = useQueryClient();

	const billing = createQuery(() => ({
		queryKey: queryKeys.billing,
		queryFn: () => api.billing()
	}));

	// The sale open now, if any: every plan checkout applies it by itself, so
	// the cards strike the list price and show what Stripe will charge.
	const plans = createQuery(() => ({
		queryKey: queryKeys.plans,
		queryFn: () => api.plans()
	}));
	const sale = $derived(plans.data?.sale ?? null);

	// A code the seller typed, sent with the next checkout. The server says
	// whether it reaches the price chosen, in its own sentence.
	let code = $state('');

	const held = $derived(billing.data);
	const balance = $derived(held?.moves);
	// A seller already paying changes tier in Stripe's portal rather than
	// through a second checkout, which would bill them twice.
	const subscribed = $derived(tiers.some((plan) => plan.id === held?.plan));
	const expiry = $derived(balance === undefined ? null : expiryLine(balance));
	const renews = $derived(held === undefined ? null : termLine(held));

	// The plan the seller holds, as the current-plan card names it. Read off
	// the generated table so the name and the one-line meaning move when the
	// plan does.
	const heldPlan = $derived(PLANS.find((plan) => plan.id === held?.plan) ?? null);

	// Payment and invoices exist once Stripe knows a customer, which is the
	// same fact that opens the portal.
	const hasCustomer = $derived(held?.portal_available === true);
	const paymentMethod = createQuery(() => ({
		queryKey: queryKeys.billingPaymentMethod,
		queryFn: () => api.billingPaymentMethod(),
		enabled: hasCustomer
	}));
	const invoices = createQuery(() => ({
		queryKey: queryKeys.billingInvoices,
		queryFn: () => api.billingInvoices(),
		enabled: hasCustomer
	}));

	// A subscription that is still running can be cancelled; a cancelled one
	// that has not yet ended can be kept. Neither applies to a plan with no
	// subscription behind it, nor to one Stripe has already ended.
	const canCancel = $derived(held?.renews_at !== undefined && !held.cancel_at_period_end);
	const canKeep = $derived(held?.cancel_at_period_end === true);
	let cancelOpen = $state(false);
	let resuming = $state(false);
	let cardOpening = $state(false);

	// What Stripe handed back, and which gate sent the seller here. The gate
	// is a query parameter so the paywall that linked here is what the
	// checkout event is attributed to, rather than every purchase looking as
	// though it started on this page.
	const outcome = $derived(checkoutOutcome(page.url.searchParams.get('checkout')));
	const gate = $derived(page.url.searchParams.get('gate') ?? 'plans_page');

	// Which checkout was opened, across the redirect to Stripe and back. The
	// browser leaves the page entirely, so the fact cannot live in a
	// component: `sessionStorage` is the narrowest place that survives the
	// round trip and dies with the tab.
	const OPENED_KEY = 'teachouse.checkout_opened';

	// What the landing page's CTA asked to buy, carried across the signup the
	// seller had to do first. `$lib/pages/account/intent` holds the record and
	// the rules about it; this is its only reader, and reading spends it — an
	// intent that survived a refused checkout would reopen Stripe on every
	// visit.

	/** Why the checkout the landing page promised did not open, or null. Said
	 *  on the page rather than in a toast: nothing here was clicked, so the
	 *  sentence has to stand beside the options it is asking the seller to
	 *  pick from again. */
	let intentRefused = $state<string | null>(null);

	let working = $state<PriceKey | null>(null);
	let portalOpening = $state(false);

	// Set while this page is the one navigating away, so the `pagehide` the
	// redirect itself fires is not reported as an abandonment.
	let leaving = false;

	function opened(): { price_key: string; origin_gate: string } | null {
		const raw = sessionStorage.getItem(OPENED_KEY);
		if (raw === null) {
			return null;
		}
		try {
			return JSON.parse(raw) as { price_key: string; origin_gate: string };
		} catch {
			return null;
		}
	}

	onMount(() => {
		if (outcome === 'success') {
			// The purchase landed, so the checkout that opened is spent and
			// the balance on this page is stale: Stripe's webhook writes the
			// moves, and this asks the server again rather than adding them
			// in the browser.
			sessionStorage.removeItem(OPENED_KEY);
			void queryClient.invalidateQueries({ queryKey: queryKeys.billing });
		}

		// Straight on to the checkout the landing page promised, but never on
		// a return from Stripe: a seller who cancelled would be sent back
		// into the same checkout they just left.
		if (outcome === null) {
			const wanted = readIntent()?.price ?? null;
			if (wanted !== null) {
				void buy(wanted, 'landing');
			}
		}

		function abandoned() {
			if (leaving || outcome === 'success') {
				return;
			}
			const record = opened();
			if (record === null) {
				return;
			}
			sessionStorage.removeItem(OPENED_KEY);
			capture('checkout_abandoned', record);
		}

		window.addEventListener('pagehide', abandoned);
		return () => window.removeEventListener('pagehide', abandoned);
	});

	async function buy(priceKey: PriceKey, origin: string = gate) {
		working = priceKey;
		try {
			const { url } = await api.billingCheckout(priceKey, code);
			sessionStorage.setItem(
				OPENED_KEY,
				JSON.stringify({ price_key: priceKey, origin_gate: origin })
			);
			capture('checkout_opened', { price_key: priceKey, origin_gate: origin });
			leaving = true;
			window.location.assign(url);
		} catch (failure) {
			working = null;
			const why =
				failure instanceof ApiFailure ? failure.message : 'The checkout did not open.';
			if (origin === 'landing') {
				// The API's own sentence where it gave one; otherwise the one
				// thing left to say, which is what to do next.
				intentRefused =
					failure instanceof ApiFailure ? why : 'Pick an option below to try again.';
				return;
			}
			toast('error', why);
		}
	}

	async function manage() {
		portalOpening = true;
		try {
			const { url } = await api.billingPortal();
			leaving = true;
			window.location.assign(url);
		} catch (failure) {
			portalOpening = false;
			toast(
				'error',
				failure instanceof ApiFailure ? failure.message : 'Billing did not open.'
			);
		}
	}

	function settled(view: BillingView) {
		queryClient.setQueryData(queryKeys.billing, view);
	}

	async function keepPlan() {
		resuming = true;
		try {
			settled(await api.billingResume());
			toast('info', 'Your plan will renew as before.');
		} catch (failure) {
			toast(
				'error',
				failure instanceof ApiFailure ? failure.message : 'Your plan was not changed.'
			);
		} finally {
			resuming = false;
		}
	}

	async function updateCard() {
		cardOpening = true;
		try {
			const { url } = await api.billingUpdatePaymentMethod();
			leaving = true;
			window.location.assign(url);
		} catch (failure) {
			cardOpening = false;
			toast(
				'error',
				failure instanceof ApiFailure ? failure.message : 'The card page did not open.'
			);
		}
	}

	/** Why a buy control cannot run, or null where it can. */
	const busy = $derived(working === null ? null : 'A checkout is opening.');
</script>

<div class="page">
	<PageHead icon="credit-card" title="Billing" guide="plans" />

	{#if outcome === 'success'}
		<Banner tone="ok" title="Payment taken">Your moves are on the card below.</Banner>
	{:else if outcome === 'cancel'}
		<Banner tone="info" title="Checkout closed">Pick an option below to try again.</Banner>
	{:else if intentRefused !== null}
		<Banner tone="bad" title="Checkout did not open">{intentRefused}</Banner>
	{/if}

	<section class="current-plan" aria-labelledby="current-plan-name">
		<span class="current-mark" aria-hidden="true"><Icon name="credit-card" size={22} /></span>
		<div class="current-text">
			{#if billing.isPending}
				<p class="quiet">Loading…</p>
			{:else if billing.isError || held === undefined}
				<p class="quiet">Your plan did not load. Refresh the page to try again.</p>
			{:else}
				<h2 id="current-plan-name">{heldPlan?.name ?? held.plan} plan</h2>
				{#if heldPlan !== null}
					<p>{planMeaning(heldPlan.capabilities)}</p>
				{/if}
				{#if renews !== null}
					<p class="term" class:ending={held.ends_at !== undefined}>{renews}</p>
				{/if}
			{/if}
		</div>
		<Button href="#plans">Adjust plan</Button>
	</section>

	{#if hasCustomer}
		<Panel title="Payment">
			<div class="bill-row">
				<span class="bill-card">
					<Icon name="credit-card" size={16} />
					{#if paymentMethod.isPending}
						<span class="quiet">Loading…</span>
					{:else if paymentMethod.isError}
						<span class="quiet">Your card did not load.</span>
					{:else if paymentMethod.data?.card === undefined}
						<span class="quiet">No card on file.</span>
					{:else}
						{cardLabel(paymentMethod.data.card)}
					{/if}
				</span>
				<Button
					small
					disabled={cardOpening}
					reason={cardOpening ? 'The card page is opening.' : undefined}
					onclick={updateCard}
				>
					{cardOpening ? 'Opening…' : 'Update'}
				</Button>
			</div>
		</Panel>

		<Panel title="Invoices">
			{#if invoices.isPending}
				<p class="quiet">Loading…</p>
			{:else if invoices.isError}
				<p class="quiet">Your invoices did not load. Refresh the page to try again.</p>
			{:else if (invoices.data?.invoices.length ?? 0) === 0}
				<p class="quiet">No invoices yet.</p>
			{:else}
				<table class="bill-invoices">
					<thead>
						<tr>
							<th scope="col">Date</th>
							<th scope="col">Total</th>
							<th scope="col">Status</th>
							<th scope="col"><span class="sr-only">Invoice</span></th>
						</tr>
					</thead>
					<tbody>
						{#each invoices.data?.invoices ?? [] as invoice (invoice.id)}
							{@const status = invoiceStatus(invoice.status)}
							<tr>
								<td>{dayLabel(invoice.created_at)}</td>
								<td class="num">{money(invoice.total, invoice.currency)}</td>
								<td><StatusPill tone={status.tone} label={status.label} /></td>
								<td class="act">
									{#if invoice.hosted_url !== undefined}
										<a href={invoice.hosted_url} target="_blank" rel="noopener noreferrer">View</a>
									{/if}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}
		</Panel>

		{#if canCancel || canKeep}
			<Panel title="Cancellation">
				<div class="bill-row">
					{#if canKeep}
						<span>{renews ?? 'Your plan will not renew.'}</span>
						<Button
							tier="additive"
							small
							disabled={resuming}
							reason={resuming ? 'Your plan is being kept.' : undefined}
							onclick={keepPlan}
						>
							{resuming ? 'Keeping…' : 'Keep my plan'}
						</Button>
					{:else}
						<span>Stop renewing. You keep your plan until the end of the period you paid for.</span>
						<Button small danger onclick={() => (cancelOpen = true)}>Cancel plan</Button>
					{/if}
				</div>
			</Panel>
		{/if}
	{/if}

	<Panel title="Your moves">
		{#if billing.isPending}
			<p class="quiet">Loading…</p>
		{:else if billing.isError || balance === undefined}
			<p class="quiet">Your balance did not load. Refresh the page to try again.</p>
		{:else}
			<p class="moves-count"><span class="n">{balance.available}</span> available</p>
			{#if expiry !== null}
				<p class="quiet">{expiry}</p>
			{/if}
			<Note icon="info">
				A move is publishing one imported resource onto one marketplace. Publishing a resource to
				Tes and TPT is 2 moves; to Tes alone is 1 move.
				<a href="/guides/plans">Read how moves work.</a>
			</Note>
		{/if}
	</Panel>

	{#if sale !== null && !subscribed}
		<Banner tone="ok" title={sale.banner}>
			{saleLine(sale)}. The sale price is taken off at checkout.
		</Banner>
	{/if}

	<section class="tiers" id="plans" aria-labelledby="plans-title">
		<div class="tiers-head">
			<h2 id="plans-title">Plans</h2>
			<div class="flow-choice cadence" role="radiogroup" aria-label="Billing period">
				<button
					type="button"
					role="radio"
					aria-checked={cadence === 'monthly'}
					onclick={() => (cadence = 'monthly')}>Monthly</button
				>
				<button
					type="button"
					role="radio"
					aria-checked={cadence === 'yearly'}
					onclick={() => (cadence = 'yearly')}>Yearly</button
				>
			</div>
		</div>
		<div class="tier-grid">
			{#if look !== null}
				<div class="acct-plan" class:held={held?.plan === 'free'}>
					<div class="acct-plan-top">
						<span class="name">
							{look.name}
							<span class="kind">(trial)</span>
						</span>
						{#if held?.plan === 'free'}
							<StatusPill tone="ok" label="current" />
						{/if}
					</div>
					<div class="price">
						<span class="n">Free</span>
						<span class="per">for as long as you like</span>
					</div>
					<ul class="bullets">
						{#each lookBullets as line (line.text)}
							<li>{line.text}</li>
						{/each}
					</ul>
				</div>
			{/if}

			{#each tiers as plan (plan.id)}
				{@const price = tierPrice(plan, cadence)}
				{@const current = held?.plan === plan.id}
				{@const onSale = subscribed
					? null
					: cadence === 'yearly'
						? salePrice(sale, plan.yearly_cents, 12)
						: salePrice(sale, plan.monthly_cents)}
				{#if price !== null}
					<div class="acct-plan" class:held={current}>
						<div class="acct-plan-top">
							<span class="name">
								{plan.name}
								<span class="kind">(subscription)</span>
							</span>
							{#if current}
								<StatusPill tone="ok" label="current" />
							{/if}
						</div>
						<div class="price">
							{#if onSale !== null}
								<s class="was">{dollars(onSale.listCents)}</s>
								<span class="n">{dollars(onSale.saleCents)}</span>
							{:else}
								<span class="n">{price.headline}</span>
							{/if}
							<span class="per">{price.per}</span>
						</div>
						{#if onSale !== null && sale !== null}
							<p class="sale-line">{saleLine(sale)}</p>
						{/if}
						{#if onSale !== null && cadence === 'yearly' && plan.yearly_cents !== null}
							<p class="quiet">
								{dollars(afterPercentOff(plan.yearly_cents, sale?.percent_off ?? 0))} for the year (usually
								{dollars(plan.yearly_cents)}).
							</p>
						{:else}
							<p class="quiet">{price.note}</p>
						{/if}
						<ul class="bullets">
							{#each planBullets(plan.capabilities) as line (line.text)}
								<li class:soon={line.soon}>{line.text}</li>
							{/each}
						</ul>
						<div class="actions">
							{#if subscribed}
								{#if !current}
									<Button
										disabled={portalOpening || held?.portal_available === false}
										reason={portalOpening
											? 'Billing is opening.'
											: held?.portal_available === false
												? 'Billing opens after your first payment.'
												: undefined}
										onclick={manage}
									>
										{portalOpening ? 'Opening…' : `Switch to ${plan.name}`}
									</Button>
								{/if}
							{:else}
								<Button
									tier="primary"
									disabled={busy !== null}
									reason={busy ?? undefined}
									onclick={() => buy(price.key)}
								>
									{working === price.key ? 'Opening…' : `Choose ${plan.name}`}
								</Button>
							{/if}
						</div>
					</div>
				{/if}
			{/each}
		</div>
		{#if !subscribed}
			<div class="code-row">
				<label for="discount-code">Have a code?</label>
				<input
					id="discount-code"
					type="text"
					autocomplete="off"
					spellcheck="false"
					placeholder="Discount code"
					bind:value={code}
				/>
				<span class="quiet">It is checked when you choose a plan or a pack.</span>
			</div>
		{/if}
		{#if subscribed}
			<p class="quiet tiers-note">
				Switching opens Stripe, which shows the new price before anything changes.
			</p>
		{/if}
	</section>

	<Panel title="Move Packs (one-off)">
		{#if editDays !== null}
			<p class="pack-note">
				Edit a moved listing once within {editDays} days without spending another move.
			</p>
		{/if}
		<div class="pack-grid">
			{#each packs as pack (pack.key)}
				<div class="acct-plan">
					<div class="acct-plan-top">
						<span class="name">
							<Icon name="shopping-bag" size={16} />
							{moves(pack.moves)}
						</span>
						{#if best !== null && best.key === pack.key}
							<StatusPill tone="ok" label="best value" />
						{/if}
					</div>
					<div class="price">
						<span class="n">{dollars(pack.price_cents)}</span>
						<span class="per">{dollars(pack.per_move_cents)} a move</span>
					</div>
					<p class="quiet">Valid 12 months.</p>
					<div class="actions">
						<Button
							disabled={busy !== null}
							reason={busy ?? undefined}
							onclick={() => buy(pack.key)}
						>
							{working === pack.key ? 'Opening…' : `Buy ${dollars(pack.price_cents)}`}
						</Button>
					</div>
				</div>
			{/each}
		</div>
	</Panel>
</div>

<CancelPlanDialog
	open={cancelOpen}
	planName={heldPlan?.name ?? 'current'}
	periodEnd={held?.renews_at}
	onClose={() => (cancelOpen = false)}
	onCancelled={(view) => {
		settled(view);
		cancelOpen = false;
		toast('info', 'Your plan is cancelled. It keeps working until the end date.');
	}}
/>

<style>
	.price .was {
		font-size: 18px;
		color: var(--muted);
		text-decoration-thickness: 2px;
	}

	.sale-line {
		font-size: 12.5px;
		font-weight: 600;
		color: var(--ok-ink);
	}

	.code-row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 6px 10px;
		margin-top: 12px;
		font-size: 13px;
	}

	.code-row label {
		font-weight: 500;
	}

	.code-row input {
		width: 14em;
		text-transform: uppercase;
	}

	/* The plan the seller holds, set apart from the plan cards below by its
	   ground: a card of the same colour as the ones on sale reads as one more
	   offer rather than as what is already theirs. */
	.current-plan {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--s-3) var(--s-4);
		padding: 20px 18px;
		border-radius: var(--r-card);
		background: var(--additive-soft);
		border: 1px solid color-mix(in srgb, var(--additive) 25%, transparent);
	}

	.current-mark {
		display: grid;
		place-items: center;
		width: 44px;
		height: 44px;
		border-radius: var(--r-panel);
		background: var(--surface);
		color: var(--additive);
		flex: none;
	}

	.current-text {
		flex: 1 1 220px;
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	.current-text h2 {
		margin: 0;
		font-family: var(--display);
		font-size: 18px;
		font-weight: 600;
	}

	.current-text p {
		margin: 0;
		font-size: 13px;
		line-height: 1.4;
	}

	.current-text .term {
		color: var(--muted);
	}

	.current-text .term.ending {
		color: var(--warn-ink);
		font-weight: 500;
	}

	.bill-row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: var(--s-3);
		font-size: 13px;
	}

	.bill-row > span {
		flex: 1 1 200px;
		min-width: 0;
	}

	.bill-card {
		display: inline-flex;
		align-items: center;
		gap: 8px;
	}

	.bill-invoices {
		width: 100%;
		border-collapse: collapse;
		font-size: 13px;
	}

	.bill-invoices th {
		text-align: left;
		font-size: 12px;
		font-weight: 500;
		color: var(--muted);
		padding: 0 8px 8px 0;
	}

	.bill-invoices td {
		padding: 8px 8px 8px 0;
		border-top: 1px solid var(--line);
		white-space: nowrap;
	}

	.bill-invoices .num {
		font-variant-numeric: tabular-nums;
	}

	.bill-invoices .act {
		text-align: right;
		padding-right: 0;
	}

	.kind {
		font-family: var(--sans);
		font-size: 13px;
		font-weight: 500;
		color: var(--muted);
	}

	.bullets {
		margin: 0;
		padding-left: 18px;
		display: flex;
		flex-direction: column;
		gap: 4px;
		font-size: 13px;
		line-height: 1.4;
	}

	.bullets .soon {
		color: var(--muted);
	}

	.tiers {
		display: flex;
		flex-direction: column;
		gap: var(--s-3);
	}

	.tiers-head {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: var(--s-3);
	}

	/* Two buttons side by side: the flow-choice grid's auto-fit would fall
	   to one column inside the flex head. */
	.tiers-head .cadence {
		grid-template-columns: repeat(2, minmax(0, auto));
		flex: 0 0 auto;
	}

	.tiers-head h2 {
		margin: 0;
		font-family: var(--display);
		font-size: 18px;
		font-weight: 600;
	}

	/* Four cards on a laptop, two on a tablet, one on a phone: the same
	   break the landing page's pricing deck takes. */
	.tier-grid {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 12px;
	}

	@media (min-width: 701px) {
		.tier-grid {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}

	@media (min-width: 1180px) {
		.tier-grid {
			grid-template-columns: repeat(4, minmax(0, 1fr));
		}
	}

	/* The tier the seller holds, ringed in the same accent as the current
	   plan card at the top of the page. */
	.tier-grid .acct-plan.held {
		border-color: var(--additive);
		box-shadow: inset 0 0 0 1px var(--additive);
	}

	.tiers-note {
		margin: 0;
		font-size: 13px;
	}

	.pack-note {
		margin: 0 0 12px;
		font-size: 13px;
		color: var(--muted);
	}
</style>
