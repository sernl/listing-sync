<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { ApiFailure, api, type BillingView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { FOUNDING, PLANS, SERVICES } from '$lib/generated/plans';
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
		foundingClosesLabel,
		foundingOpen,
		invoiceStatus,
		money,
		moves,
		packsBySize,
		perMonth,
		planBullets,
		planMeaning,
		syncPlan,
		termLine
	} from '$lib/pages/account/plans';
	import CancelPlanDialog from '$lib/pages/account/CancelPlanDialog.svelte';
	import { readIntent } from '$lib/pages/account/intent';
	import '$lib/pages/account/account.css';

	// Everything priced on this page comes from the generated table, which is
	// `tam-limits`' own and the same figures the checkout charges. Nothing
	// here writes a price as a literal: a number typed into markup is a price
	// that drifts from the one Stripe takes.
	const look = PLANS.find((plan) => plan.id === 'free') ?? null;
	const sync = syncPlan();
	const packs = packsBySize();
	const best = bestValuePack();
	const service = SERVICES[0];
	const founding = FOUNDING;

	// The card lines are the landing page's pricing cards, read off the same
	// capabilities, so a seller reads one promise on both.
	const lookBullets = look === null ? [] : planBullets(look.capabilities);
	const syncBullets = sync === null ? [] : planBullets(sync.capabilities);
	const editDays = (sync ?? look)?.capabilities.pack_edit_days ?? null;

	// Read once at load rather than in a `$derived`: whether the offer is open
	// is a fact about today, and re-evaluating it per render would make the
	// card flicker off mid-session at midnight on the closing day.
	const foundingIsOpen = foundingOpen(founding);

	const queryClient = useQueryClient();

	const billing = createQuery(() => ({
		queryKey: queryKeys.billing,
		queryFn: () => api.billing()
	}));

	const held = $derived(billing.data);
	const balance = $derived(held?.moves);
	const subscribed = $derived(held?.plan === 'subscriber' || held?.plan === 'studio');
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
			const { url } = await api.billingCheckout(priceKey);
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

	<div class="page-grid" id="plans">
		<div class="plan-column">
			{#if look !== null}
				<div class="acct-plan">
					<div class="acct-plan-top">
						<span class="name">
							<Icon name="eye" size={16} />
							{look.name}
							<span class="kind">(Trial)</span>
						</span>
						{#if held?.plan === 'free'}
							<StatusPill tone="ok" label="current" />
						{/if}
					</div>
					<div class="price">
						<span class="n">{dollars(0)}</span>
						<span class="per">to start</span>
					</div>
					<ul class="bullets">
						{#each lookBullets as line (line.text)}
							<li>{line.text}</li>
						{/each}
					</ul>
				</div>
			{/if}

			{#if sync !== null && sync.yearly_cents !== null && sync.monthly_cents !== null}
				<div class="acct-plan">
					<div class="acct-plan-top">
						<span class="name">
							<Icon name="credit-card" size={16} />
							{sync.name}
							<span class="kind">(Subscription)</span>
						</span>
						{#if subscribed}
							<StatusPill tone="ok" label="current" />
						{/if}
					</div>
					<div class="price">
						<span class="n">{perMonth(sync.yearly_cents)}</span>
						<span class="per">a month, billed yearly</span>
					</div>
					<p class="quiet">
						{dollars(sync.yearly_cents)} a year, or {dollars(sync.monthly_cents)} a month.
					</p>
					<ul class="bullets">
						{#each syncBullets as line (line.text)}
							<li class:soon={line.soon}>{line.text}</li>
						{/each}
					</ul>
					{#if subscribed}
						{#if renews !== null}
							<p class="quiet">{renews}</p>
						{/if}
						<div class="actions">
							<Button
								tier="primary"
								disabled={portalOpening || held?.portal_available === false}
								reason={portalOpening
									? 'Billing is opening.'
									: held?.portal_available === false
										? 'Billing opens after your first payment.'
										: undefined}
								onclick={manage}
							>
								{portalOpening ? 'Opening…' : 'Manage billing'}
							</Button>
						</div>
					{:else}
						<div class="actions">
							<Button
								tier="primary"
								disabled={busy !== null}
								reason={busy ?? undefined}
								onclick={() => buy('sync_yearly')}
							>
								{working === 'sync_yearly' ? 'Opening…' : 'Choose yearly'}
							</Button>
							<Button
								disabled={busy !== null}
								reason={busy ?? undefined}
								onclick={() => buy('sync_monthly')}
							>
								{working === 'sync_monthly' ? 'Opening…' : 'Choose monthly'}
							</Button>
						</div>
					{/if}
				</div>
			{/if}
		</div>

		<div class="plan-column">
			{#if foundingIsOpen}
				<div class="acct-plan">
					<div class="acct-plan-top">
						<span class="name">
							<Icon name="gift" size={16} />
							Founding {founding.places}
						</span>
					</div>
					<div class="price">
						<span class="n">{dollars(founding.year_one_cents)}</span>
						<span class="per">first year</span>
					</div>
					<p>
						Then {dollars(founding.ongoing_cents)} a year for years 2–{founding.ongoing_years},
						with {founding.extra_moves} extra moves.
					</p>
					<p class="quiet">{foundingClosesLabel(founding)}</p>
					<div class="actions">
						<Button
							tier="primary"
							disabled={busy !== null}
							reason={busy ?? undefined}
							onclick={() => buy('founding_yearly')}
						>
							{working === 'founding_yearly' ? 'Opening…' : 'Take a founding place'}
						</Button>
					</div>
				</div>
			{/if}

			{#if service !== undefined}
				<div class="acct-plan">
					<div class="acct-plan-top">
						<span class="name">
							<Icon name="sparkles" size={16} />
							{service.name}
						</span>
					</div>
					<div class="price">
						<span class="n">{dollars(service.price_cents)}</span>
						<span class="per">one-off</span>
					</div>
					<p>We move your back catalogue for you.</p>
					<div class="actions">
						<Button
							disabled={busy !== null}
							reason={busy ?? undefined}
							onclick={() => buy(service.key)}
						>
							{working === service.key ? 'Opening…' : 'Book a move'}
						</Button>
					</div>
				</div>
			{/if}
		</div>
	</div>

	<Panel title="Move Packs (One-Off)">
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

	.pack-note {
		margin: 0 0 12px;
		font-size: 13px;
		color: var(--muted);
	}
</style>
