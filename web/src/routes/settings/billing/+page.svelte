<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { ApiFailure, api, type BillingView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import { PLANS } from '$lib/generated/plans';
	import { type PriceKey } from '$lib/generated/vocab';
	import Icon from '$lib/Icon.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import { capture } from '$lib/posthog';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import {
		bestValuePack,
		cardExpiry,
		cardLabel,
		cardMark,
		checkoutOutcome,
		dollars,
		expiryLine,
		heldPrice,
		invoiceRows,
		moves,
		packsBySize,
		paidPlans,
		planCards,
		planMeaning,
		planTagline,
		termLine,
		type Cadence
	} from '$lib/pages/account/plans';
	import CancelPlanDialog from '$lib/pages/account/CancelPlanDialog.svelte';
	import { readIntent } from '$lib/pages/account/intent';
	import { saleLine } from '$lib/sale';
	import '$lib/pages/account/account.css';
	import '$lib/styles/data.css';

	// Everything priced on this page comes from the generated table, which is
	// `tam-limits`' own and the same figures the checkout charges. Nothing
	// here writes a price as a literal: a number typed into markup is a price
	// that drifts from the one Stripe takes.
	const look = PLANS.find((plan) => plan.id === 'free') ?? null;
	const tiers = paidPlans();
	const packs = packsBySize();
	const best = bestValuePack();

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
	const heroPrice = $derived(heldPlan === null ? null : heldPrice(heldPlan, held?.cadence));

	// The Plans grid, read off the generated table: the card lines are the
	// landing page's pricing cards, so a seller reads one promise on both.
	const cards = $derived(planCards({ held: held?.plan, cadence, sale }));

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
	const invoiceList = $derived(invoiceRows(invoices.data?.invoices ?? []));

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

<div class="page bill">
	<PageHead icon="credit-card" title="Billing" guide="plans" />

	{#if outcome === 'success'}
		<Banner tone="ok" title="Payment taken">Your moves are on the card below.</Banner>
	{:else if outcome === 'cancel'}
		<Banner tone="info" title="Checkout closed">Pick an option below to try again.</Banner>
	{:else if intentRefused !== null}
		<Banner tone="bad" title="Checkout did not open">{intentRefused}</Banner>
	{/if}

	<section class="bill-hero" aria-labelledby="current-plan-name">
		{#if billing.isPending}
			<p class="quiet">Loading…</p>
		{:else if billing.isError || held === undefined}
			<p class="quiet">Your plan did not load. Refresh the page to try again.</p>
		{:else}
			<div class="bill-hero-main">
				<span class="bill-eyebrow">Your plan</span>
				<h2 id="current-plan-name">{heldPlan?.name ?? held.plan}</h2>
				{#if heldPlan !== null}
					<p class="bill-tagline">{planTagline(heldPlan)}</p>
					{#if planTagline(heldPlan) !== planMeaning(heldPlan.capabilities)}
						<p class="bill-meaning">{planMeaning(heldPlan.capabilities)}</p>
					{/if}
				{/if}
				{#if renews !== null}
					<p class="bill-term" class:ending={held.ends_at !== undefined}>
						<Icon name="calendar-clock" size={14} />
						{renews}
					</p>
				{/if}
			</div>
			<div class="bill-hero-side">
				{#if heroPrice !== null}
					<p class="bill-hero-price">
						<span class="n">{heroPrice.headline}</span>
						<span class="per">{heroPrice.per}</span>
					</p>
				{/if}
				<div class="bill-hero-acts">
					{#if hasCustomer && canCancel}
						<Button tier="quiet" small onclick={() => (cancelOpen = true)}>Cancel plan</Button>
					{:else if hasCustomer && canKeep}
						<Button
							tier="additive"
							small
							disabled={resuming}
							reason={resuming ? 'Your plan is being kept.' : undefined}
							onclick={keepPlan}
						>
							{resuming ? 'Keeping…' : 'Keep my plan'}
						</Button>
					{/if}
					<Button tier="primary" href="#plans">Adjust plan</Button>
				</div>
			</div>
		{/if}
	</section>

	<div class="bill-pair">
		<section class="bill-card" aria-labelledby="payment-title">
			<div class="bill-card-head">
				<h2 id="payment-title">Payment</h2>
				{#if hasCustomer}
					<Button
						small
						disabled={cardOpening}
						reason={cardOpening ? 'The card page is opening.' : undefined}
						onclick={updateCard}
					>
						{cardOpening ? 'Opening…' : 'Update'}
					</Button>
				{/if}
			</div>
			{#if !hasCustomer}
				<p class="quiet">No card yet. You add one when you choose a plan or a pack.</p>
			{:else if paymentMethod.isPending}
				<p class="quiet">Loading…</p>
			{:else if paymentMethod.isError}
				<p class="quiet">Your card did not load.</p>
			{:else if paymentMethod.data?.card === undefined}
				<p class="quiet">No card on file.</p>
			{:else}
				{@const card = paymentMethod.data.card}
				{@const expires = cardExpiry(card)}
				<div class="bill-method">
					<span class="bill-brand" aria-hidden="true">{cardMark(card)}</span>
					<div>
						<p class="bill-method-name">{cardLabel(card)}</p>
						{#if expires !== null}<p class="quiet">{expires}</p>{/if}
					</div>
				</div>
			{/if}
		</section>

		<section class="bill-card" aria-labelledby="moves-title">
			<div class="bill-card-head">
				<h2 id="moves-title">Your moves</h2>
				<Explain title="How moves work" label="How it works">
					<p>
						A move is publishing one imported resource onto one marketplace. Publishing a resource
						to Tes and TPT is 2 moves; to Tes alone is 1 move.
					</p>
					<p><a href="/guides/plans">Read the guide to plans and moves.</a></p>
				</Explain>
			</div>
			{#if billing.isPending}
				<p class="quiet">Loading…</p>
			{:else if billing.isError || balance === undefined}
				<p class="quiet">Your balance did not load. Refresh the page to try again.</p>
			{:else}
				<p class="bill-moves"><span class="n">{balance.available}</span> available</p>
				<p class="quiet">{expiry ?? 'None of your moves are about to expire.'}</p>
			{/if}
		</section>
	</div>

	{#if hasCustomer}
		<section class="bill-section" aria-labelledby="invoices-title">
			<h2 id="invoices-title" class="bill-h2">Invoices</h2>
			{#if invoices.isPending}
				<p class="data-empty">Loading…</p>
			{:else if invoices.isError}
				<p class="data-empty">Your invoices did not load. Refresh the page to try again.</p>
			{:else if invoiceList.length === 0}
				<p class="data-empty">No invoices yet. Each payment shows up here with its receipt.</p>
			{:else}
				<div class="data-table-wrap">
					<table class="data-table stack">
						<thead>
							<tr>
								<th scope="col">Date</th>
								<th scope="col">Description</th>
								<th scope="col" class="num">Amount</th>
								<th scope="col">Status</th>
								<th scope="col" class="act">Receipt</th>
							</tr>
						</thead>
						<tbody>
							{#each invoiceList as row (row.id)}
								<tr>
									<td class="nowrap" data-label="Date">{row.date}</td>
									<td data-label="Description">{row.description}</td>
									<td class="num" data-label="Amount">{row.amount}</td>
									<td data-label="Status"><StatusPill tone={row.status.tone} label={row.status.label} /></td>
									<td class="act" data-label="Receipt">
										{#if row.receipt !== null}
											<a
												class="bill-receipt"
												href={row.receipt}
												target="_blank"
												rel="noopener noreferrer"
											>
												View <Icon name="external-link" size={12} />
											</a>
										{:else}
											<span class="quiet">—</span>
										{/if}
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</section>
	{/if}

	<section class="bill-section" id="plans" aria-labelledby="plans-title">
		<div class="bill-plans-head">
			<h2 id="plans-title" class="bill-h2">Plans</h2>
			<div class="segmented" role="radiogroup" aria-label="Billing period">
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

		{#if sale !== null && !subscribed}
			<Banner tone="ok" title={sale.banner}>
				{saleLine(sale)}. The sale price is taken off at checkout.
			</Banner>
		{/if}

		<div class="bill-plan-grid">
			{#each cards as card (card.id)}
				<article
					class="bill-plan"
					class:held={card.current}
					class:recommended={card.recommended}
					aria-labelledby="plan-{card.id}"
				>
					{#if card.recommended}
						<span class="bill-plan-badge">Recommended</span>
					{/if}
					<div class="bill-plan-top">
						<h3 id="plan-{card.id}">{card.name}</h3>
						{#if card.current}
							<StatusPill tone="ok" label="Your plan" />
						{:else}
							<span class="kind">{card.kind}</span>
						{/if}
					</div>
					<p class="bill-plan-tagline">{card.tagline}</p>
					<div class="bill-plan-price">
						{#if card.was !== null}<s class="was">{card.was}</s>{/if}
						<span class="n">{card.headline}</span>
						<span class="per">{card.per}</span>
					</div>
					{#if card.saleNote !== null}<p class="sale-line">{card.saleNote}</p>{/if}
					<p class="bill-plan-note">{card.note ?? ''}</p>
					<ul class="bill-bullets">
						{#each card.bullets as line (line.text)}
							<li class:soon={line.soon}>
								<Icon name="check" size={14} />
								<span>{line.text}</span>
							</li>
						{/each}
					</ul>
					<div class="bill-plan-cta">
						{#if card.cta?.kind === 'buy'}
							{@const key = card.cta.key}
							<Button
								tier={card.recommended ? 'primary' : 'outline'}
								disabled={busy !== null}
								reason={busy ?? undefined}
								onclick={() => buy(key)}
							>
								{working === key ? 'Opening…' : card.cta.label}
							</Button>
						{:else if card.cta?.kind === 'switch'}
							<Button
								tier={card.recommended ? 'primary' : 'outline'}
								disabled={portalOpening || held?.portal_available === false}
								reason={portalOpening
									? 'Billing is opening.'
									: held?.portal_available === false
										? 'Billing opens after your first payment.'
										: undefined}
								onclick={manage}
							>
								{portalOpening ? 'Opening…' : card.cta.label}
							</Button>
						{:else if card.current}
							<span class="bill-plan-here">You are on this plan</span>
						{:else}
							<span class="bill-plan-here">Where every account starts</span>
						{/if}
					</div>
				</article>
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
		{:else}
			<p class="quiet">Switching opens Stripe, which shows the new price before anything changes.</p>
		{/if}
	</section>

	<section class="bill-section" aria-labelledby="packs-title">
		<div>
			<h2 id="packs-title" class="bill-h2">Move Packs</h2>
			<p class="quiet bill-sub">
				One-off, valid 12 months.
				{#if editDays !== null}
					Edit a moved listing once within {editDays} days without spending another move.
				{/if}
			</p>
		</div>
		<div class="bill-pack-grid">
			{#each packs as pack (pack.key)}
				<div class="bill-card bill-pack">
					<div class="bill-card-head">
						<span class="bill-pack-name">
							<Icon name="shopping-bag" size={16} />
							{moves(pack.moves)}
						</span>
						{#if best !== null && best.key === pack.key}
							<StatusPill tone="ok" label="Best value" />
						{/if}
					</div>
					<div class="bill-plan-price">
						<span class="n">{dollars(pack.price_cents)}</span>
						<span class="per">{dollars(pack.per_move_cents)} a move</span>
					</div>
					<div class="bill-plan-cta">
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
	</section>
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
	/* One column of sections at one rhythm: --s-6 between sections, --s-4
	   inside one. 1040 wide at most, so four plan cards sit on a laptop
	   without the page reading as a spreadsheet. */
	.bill {
		max-width: 1040px;
		display: flex;
		flex-direction: column;
		gap: var(--s-6);
	}

	.bill :global(.page-head) {
		margin-bottom: 0;
	}

	.bill-h2 {
		margin: 0;
		font-family: var(--display);
		font-size: 19px;
		font-weight: 600;
		letter-spacing: -0.01em;
	}

	.bill-sub {
		margin: var(--s-1) 0 0;
	}

	.bill-section {
		display: flex;
		flex-direction: column;
		gap: var(--s-4);
		min-width: 0;
		scroll-margin-top: var(--s-5);
	}

	/* --- the current plan ------------------------------------------------ */

	.bill-hero {
		display: flex;
		flex-wrap: wrap;
		align-items: flex-end;
		justify-content: space-between;
		gap: var(--s-5);
		padding: var(--s-5) var(--s-6);
		border-radius: var(--r-card);
		background: var(--additive-soft);
		border: 1px solid color-mix(in srgb, var(--additive) 25%, transparent);
	}

	.bill-hero-main {
		flex: 1 1 320px;
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: var(--s-1);
	}

	.bill-eyebrow {
		font-size: 12px;
		font-weight: 600;
		letter-spacing: 0.04em;
		text-transform: uppercase;
		color: var(--additive);
	}

	.bill-hero h2 {
		margin: 0;
		font-family: var(--display);
		font-size: 28px;
		font-weight: 600;
		letter-spacing: -0.01em;
		line-height: 1.2;
	}

	.bill-hero p {
		margin: 0;
		font-size: 14px;
		line-height: 1.45;
	}

	.bill-hero .bill-meaning {
		font-size: 13px;
		color: var(--muted);
	}

	.bill-hero .bill-term {
		display: inline-flex;
		align-items: center;
		gap: var(--s-2);
		margin-top: var(--s-2);
		color: var(--muted);
		font-size: 13px;
	}

	.bill-hero .bill-term.ending {
		color: var(--warn-ink);
		font-weight: 500;
	}

	.bill-hero-side {
		display: flex;
		flex-direction: column;
		align-items: flex-end;
		gap: var(--s-3);
	}

	.bill-hero-price {
		display: flex;
		align-items: baseline;
		gap: var(--s-2);
	}

	.bill-hero-price .n {
		font-family: var(--display);
		font-size: 32px;
		font-weight: 600;
		font-variant-numeric: tabular-nums;
	}

	.bill-hero-price .per {
		color: var(--muted);
		font-size: 13px;
	}

	.bill-hero-acts {
		display: flex;
		align-items: center;
		gap: var(--s-2);
	}

	/* --- payment and moves ----------------------------------------------- */

	.bill-pair {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: var(--s-4);
	}

	@media (min-width: 761px) {
		.bill-pair {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}

	.bill-card {
		display: flex;
		flex-direction: column;
		gap: var(--s-3);
		background: var(--card);
		border: 1px solid var(--line);
		border-radius: var(--r-card);
		padding: var(--s-5);
		min-width: 0;
	}

	.bill-card p {
		margin: 0;
	}

	.bill-card-head {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: var(--s-3);
		min-height: var(--control-h-sm);
	}

	.bill-card-head h2 {
		margin: 0;
		font-family: var(--display);
		font-size: 16px;
		font-weight: 600;
	}

	.bill-method {
		display: flex;
		align-items: center;
		gap: var(--s-3);
	}

	.bill-brand {
		display: grid;
		place-items: center;
		min-width: 52px;
		height: 34px;
		padding: 0 var(--s-2);
		border-radius: var(--r-field);
		background: var(--primary);
		color: var(--on-fill);
		font-size: 11px;
		font-weight: 700;
		letter-spacing: 0.06em;
	}

	.bill-method-name {
		font-size: 15px;
		font-weight: 600;
		font-variant-numeric: tabular-nums;
	}

	.bill-moves {
		display: flex;
		align-items: baseline;
		gap: var(--s-2);
		color: var(--muted);
		font-size: 13px;
	}

	.bill-moves .n {
		font-family: var(--display);
		font-size: 40px;
		font-weight: 600;
		line-height: 1;
		color: var(--ink);
		font-variant-numeric: tabular-nums;
	}

	/* --- invoices -------------------------------------------------------- */

	.bill-receipt {
		display: inline-flex;
		align-items: center;
		gap: var(--s-1);
		font-weight: 500;
	}

	/* --- plans ----------------------------------------------------------- */

	.bill-plans-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--s-3);
	}

	/* Four cards on a laptop, two on a tablet, one on a phone. Rows stretch,
	   so every card in a row is the height of the tallest and the buttons
	   line up along one foot. */
	.bill-plan-grid {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: var(--s-4);
		align-items: stretch;
		padding-top: var(--s-3);
	}

	@media (min-width: 701px) {
		.bill-plan-grid {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}

	@media (min-width: 1100px) {
		.bill-plan-grid {
			grid-template-columns: repeat(4, minmax(0, 1fr));
		}
	}

	.bill-plan {
		position: relative;
		display: flex;
		flex-direction: column;
		gap: var(--s-2);
		background: var(--card);
		border: 1px solid var(--line);
		border-radius: var(--r-card);
		padding: var(--s-5) var(--s-4);
		min-width: 0;
	}

	.bill-plan.recommended {
		border-color: var(--accent);
		box-shadow: var(--sh-3);
		transform: translateY(calc(-1 * var(--s-2)));
	}

	.bill-plan.held {
		border-color: var(--additive);
		box-shadow: inset 0 0 0 1px var(--additive);
	}

	.bill-plan.held.recommended {
		box-shadow:
			inset 0 0 0 1px var(--additive),
			var(--sh-3);
	}

	.bill-plan-badge {
		position: absolute;
		top: 0;
		left: 50%;
		transform: translate(-50%, -50%);
		padding: 2px var(--s-3);
		border-radius: var(--r-pill);
		background: var(--accent);
		color: var(--on-fill);
		font-size: 11px;
		font-weight: 700;
		letter-spacing: 0.04em;
		text-transform: uppercase;
		white-space: nowrap;
	}

	.bill-plan-top {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--s-2);
	}

	.bill-plan-top h3 {
		margin: 0;
		font-family: var(--display);
		font-size: 19px;
		font-weight: 600;
	}

	.kind {
		font-size: 12px;
		font-weight: 500;
		color: var(--muted);
		text-transform: capitalize;
	}

	.bill-plan p {
		margin: 0;
		font-size: 13px;
		line-height: 1.4;
	}

	.bill-plan-tagline {
		color: var(--muted);
		min-height: 2.8em;
	}

	.bill-plan-price {
		display: flex;
		align-items: baseline;
		flex-wrap: wrap;
		gap: var(--s-1) var(--s-2);
		margin-top: var(--s-1);
	}

	.bill-plan-price .n {
		font-family: var(--display);
		font-size: 30px;
		font-weight: 600;
		letter-spacing: -0.02em;
		font-variant-numeric: tabular-nums;
	}

	.bill-plan-price .per {
		flex-basis: 100%;
		font-size: 12.5px;
		color: var(--muted);
	}

	.bill-plan-price .was {
		font-size: 18px;
		color: var(--muted);
		text-decoration-thickness: 2px;
	}

	.bill-plan .bill-plan-note {
		min-height: 1.4em;
		font-size: 12.5px;
		color: var(--muted);
	}

	.sale-line {
		font-weight: 600;
		color: var(--ok-ink);
	}

	.bill-bullets {
		list-style: none;
		margin: var(--s-2) 0 0;
		padding: var(--s-3) 0 0;
		border-top: 1px solid var(--line);
		display: flex;
		flex-direction: column;
		gap: var(--s-2);
		font-size: 13px;
		line-height: 1.4;
	}

	.bill-bullets li {
		display: flex;
		align-items: flex-start;
		gap: var(--s-2);
	}

	.bill-bullets li :global(svg) {
		flex: none;
		margin-top: 2px;
		color: var(--ok-ink);
	}

	.bill-bullets .soon {
		color: var(--muted);
	}

	.bill-bullets .soon :global(svg) {
		color: var(--muted);
	}

	.bill-plan-cta {
		display: flex;
		flex-direction: column;
		align-items: stretch;
		margin-top: auto;
		padding-top: var(--s-4);
	}

	.bill-plan-cta :global(.cta),
	.bill-plan-cta :global(.btn) {
		justify-content: center;
		width: 100%;
	}

	.bill-plan-here {
		display: grid;
		place-items: center;
		min-height: var(--control-h);
		font-size: 13px;
		color: var(--muted);
	}

	.code-row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--s-2) var(--s-3);
		font-size: 13px;
	}

	.code-row label {
		font-weight: 500;
	}

	.code-row input {
		width: 14em;
		text-transform: uppercase;
	}

	/* --- packs ----------------------------------------------------------- */

	.bill-pack-grid {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
		gap: var(--s-4);
	}

	.bill-pack-name {
		display: inline-flex;
		align-items: center;
		gap: var(--s-2);
		font-family: var(--display);
		font-size: 16px;
		font-weight: 600;
	}

	.bill-pack .bill-plan-cta {
		padding-top: var(--s-2);
	}

	@media (max-width: 620px) {
		.bill {
			gap: var(--s-5);
		}

		.bill-hero {
			padding: var(--s-5) var(--s-4);
		}

		.bill-hero-side {
			flex: 1 1 100%;
			align-items: stretch;
		}

		.bill-hero-acts {
			flex-direction: row-reverse;
			justify-content: flex-end;
		}

		.bill-plan.recommended {
			transform: none;
		}
	}
</style>
