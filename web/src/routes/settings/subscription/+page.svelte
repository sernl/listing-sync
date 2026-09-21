<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { ApiFailure, api } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { movesLimit } from '$lib/entitlement';
	import { FOUNDING, PACKS, SERVICES } from '$lib/generated/plans';
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
		checkoutOutcome,
		dollars,
		expiryLine,
		foundingClosesLabel,
		foundingOpen,
		moves,
		packsBySize,
		perMonth,
		renewsLine,
		syncPlan
	} from '$lib/pages/account/plans';
	import { readIntent } from '$lib/pages/account/intent';
	import '$lib/pages/account/account.css';

	// Everything priced on this page comes from the generated table, which is
	// `tam-limits`' own and the same figures the checkout charges. Nothing
	// here writes a price as a literal: a number typed into markup is a price
	// that drifts from the one Stripe takes.
	const sync = syncPlan();
	const packs = packsBySize();
	const best = bestValuePack();
	const service = SERVICES[0];
	const founding = FOUNDING;

	// The allowance sentence is `entitlement.ts`' own, because the plan card
	// and the gate that refuses a move must promise the same thing in the
	// same words.
	const allowance = sync === null ? null : movesLimit(sync.capabilities);

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
	const renews = $derived(renewsLine(held?.renews_at));

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

	/** Why a buy control cannot run, or null where it can. */
	const busy = $derived(working === null ? null : 'A checkout is opening.');
</script>

<div class="page">
	<PageHead icon="credit-card" title="Plan and moves" guide="plans" />

	{#if outcome === 'success'}
		<Banner tone="ok" title="Payment taken">Your moves are on the card above.</Banner>
	{:else if outcome === 'cancel'}
		<Banner tone="info" title="Checkout closed">Pick an option below to try again.</Banner>
	{:else if intentRefused !== null}
		<Banner tone="bad" title="Checkout did not open">{intentRefused}</Banner>
	{/if}

	<Panel title="Your moves">
		{#if billing.isPending}
			<p class="quiet">Loading…</p>
		{:else if billing.isError || balance === undefined}
			<p class="quiet">We could not read your balance.</p>
		{:else}
			<p class="moves-count"><span class="n">{balance.available}</span> available</p>
			{#if expiry !== null}
				<p class="quiet">{expiry}</p>
			{/if}
			<Note icon="info">
				<a href="/guides/plans">Read how moves are spent.</a>
			</Note>
		{/if}
	</Panel>

	<div class="page-grid">
		<div class="plan-column">
			{#if sync !== null && sync.yearly_cents !== null && sync.monthly_cents !== null}
				<div class="acct-plan">
					<div class="acct-plan-top">
						<span class="name">
							<Icon name="credit-card" size={16} />
							{sync.name}
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
					{#if allowance !== null}
						<p>{allowance}.</p>
					{/if}
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
										? 'Billing opens once a payment has been taken.'
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
						<span class="per">one off</span>
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

	<Panel title="Move packs">
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
