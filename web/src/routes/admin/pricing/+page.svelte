<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import Banner from '$lib/Banner.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill, { type Tone } from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import {
		STATE_LABEL,
		codeRefusal,
		discountFigure,
		durationLabel,
		reachLabel,
		termsBody,
		termsRefusal,
		type DiscountKind,
		type DiscountState,
		type DiscountView,
		type TermsDraft,
		type Theme
	} from '$lib/pages/admin/pricing';
	import '$lib/flow.css';
	import '$lib/pages/admin/admin.css';

	const queryClient = useQueryClient();

	const pricing = createQuery(() => ({
		queryKey: queryKeys.adminPricing,
		queryFn: () => api.adminPricing()
	}));

	const rows = $derived(pricing.data?.discounts ?? []);
	const sales = $derived(rows.filter((row) => row.kind === 'sale'));
	const oneOffs = $derived(rows.filter((row) => row.kind === 'one_off'));
	const codes = $derived(rows.filter((row) => row.kind === 'code'));
	const priceKeys = $derived(pricing.data?.price_keys ?? []);

	// UTC, because the server's windows are UTC days.
	const today = new Date().toISOString().slice(0, 10);

	const TONE: Record<DiscountState, Tone> = {
		scheduled: 'run',
		open: 'ok',
		over: 'soon',
		ended: 'soon'
	};

	// ------------------------------------------------------------- the form

	let kind = $state<DiscountKind>('sale');
	let draft = $state<TermsDraft>({
		name: '',
		by: 'percent',
		percent: null,
		dollars: null,
		duration: 'once',
		months: null,
		keys: [],
		from: '',
		until: ''
	});
	let banner = $state('');
	let bannerHref = $state('');
	let theme = $state<Theme | 'none'>('none');
	let code = $state('');
	let limit = $state<number | null>(null);

	/** Why the form cannot be saved, or null when it can. */
	const refusal = $derived.by(() => {
		if (pricing.data?.stripe_configured === false) {
			return 'This server has no Stripe key, so nothing can be created here.';
		}
		if (kind === 'sale' && draft.by !== 'percent') {
			return 'A sale is a percentage off.';
		}
		if (kind === 'one_off' && draft.keys.length === 0) {
			return 'Pick at least one price.';
		}
		const terms = termsRefusal(draft, today);
		if (terms !== null) {
			return terms;
		}
		if (kind === 'sale') {
			const words = banner.trim();
			if (words.length === 0 || words.length > 140) {
				return 'Write a banner of up to 140 characters.';
			}
		}
		if (kind === 'code') {
			if (codeRefusal(code) !== null) {
				return codeRefusal(code);
			}
			if (limit !== null && (!Number.isInteger(limit) || limit < 1)) {
				return 'A redemption limit is at least 1, or empty for none.';
			}
		}
		return null;
	});

	function reset() {
		draft = { ...draft, name: '', percent: null, dollars: null, keys: [], from: '', until: '' };
		banner = '';
		bannerHref = '';
		theme = 'none';
		code = '';
		limit = null;
	}

	const saving = createMutation(() => ({
		mutationFn: () => {
			const terms = termsBody(draft);
			if (kind === 'sale') {
				return api.createSale({
					name: terms.name,
					percent_off: terms.percent_off ?? 0,
					from: terms.from,
					until: terms.until,
					banner: banner.trim(),
					...(bannerHref.trim() === '' ? {} : { banner_href: bannerHref.trim() }),
					...(theme === 'none' ? {} : { theme }),
					duration: terms.duration,
					...(terms.duration_months === undefined
						? {}
						: { duration_months: terms.duration_months })
				});
			}
			if (kind === 'one_off') {
				return api.createOneOff(terms);
			}
			return api.createCode({
				...terms,
				code: code.trim(),
				...(limit === null ? {} : { max_redemptions: limit })
			});
		},
		onSuccess: async (saved: DiscountView) => {
			await queryClient.invalidateQueries({ queryKey: queryKeys.adminPricing });
			reset();
			toast('info', `${saved.name} saved. Stripe has its coupon.`);
		},
		onError: (failure: Error) =>
			toast('error', failure instanceof ApiFailure ? failure.message : 'It was not saved.')
	}));

	let endingId = $state<string | null>(null);
	const ending = createMutation(() => ({
		mutationFn: (id: string) => api.endDiscount(id),
		onSuccess: async (ended: DiscountView) => {
			endingId = null;
			await queryClient.invalidateQueries({ queryKey: queryKeys.adminPricing });
			toast('info', `${ended.name} ended. Checkouts stop applying it now.`);
		},
		onError: (failure: Error) => {
			endingId = null;
			toast('error', failure instanceof ApiFailure ? failure.message : 'It was not ended.');
		}
	}));

	function toggleKey(key: string) {
		draft.keys = draft.keys.includes(key)
			? draft.keys.filter((held) => held !== key)
			: [...draft.keys, key];
	}

	const KINDS: { id: DiscountKind; label: string; sub: string }[] = [
		{ id: 'sale', label: 'Sale', sub: 'Every plan, with a banner' },
		{ id: 'one_off', label: 'One-off discount', sub: 'Chosen prices, no code' },
		{ id: 'code', label: 'Discount code', sub: 'Sellers type it at checkout' }
	];

	const THEMES: { id: Theme | 'none'; label: string }[] = [
		{ id: 'none', label: 'No theme' },
		{ id: 'halloween', label: 'Halloween' },
		{ id: 'christmas', label: 'Christmas' }
	];
</script>

{#snippet list(items: DiscountView[], empty: string)}
	{#if items.length === 0}
		<p class="quiet">{empty}</p>
	{:else}
		<ul class="flow-list">
			{#each items as row (row.id)}
				<li class="flow-item" class:off={row.state === 'over' || row.state === 'ended'}>
					<div class="flow-item-main">
						<div class="flow-item-title">
							{row.name}
							<StatusPill tone={TONE[row.state]} label={STATE_LABEL[row.state]} />
							{#if row.in_stripe === false && (row.state === 'open' || row.state === 'scheduled')}
								<StatusPill tone="bad" label="Missing in Stripe" />
							{/if}
						</div>
						<div class="flow-item-line">
							{discountFigure(row)} · {durationLabel(row)} · {reachLabel(row.price_keys)} ·
							{row.from} to {row.until}
						</div>
						{#if row.banner}
							<div class="flow-item-line">“{row.banner}”{row.theme ? ` · ${row.theme}` : ''}</div>
						{/if}
						{#each row.codes as typed (typed.id)}
							<div class="flow-item-line">
								<span class="mono">{typed.code}</span>
								{typed.max_redemptions === null ? '' : ` · up to ${typed.max_redemptions} uses`}
								{typed.ended ? ' · ended' : ''}
							</div>
						{/each}
						<div class="flow-item-line mono quiet">{row.stripe_coupon_id}</div>
					</div>
					<div class="flow-item-acts">
						{#if row.state === 'open' || row.state === 'scheduled'}
							<Button
								small
								danger
								icon="circle-x"
								disabled={ending.isPending}
								reason={ending.isPending ? 'Ending one already.' : undefined}
								onclick={() => {
									endingId = row.id;
									ending.mutate(row.id);
								}}
							>
								{endingId === row.id ? 'Ending…' : 'End now'}
							</Button>
						{/if}
					</div>
				</li>
			{/each}
		</ul>
	{/if}
{/snippet}

<div class="page flow-page">
	<PageHead
		icon="tag"
		title="Pricing"
		description="Sales, one-off discounts and discount codes. Each one is a Stripe coupon."
	/>

	<div class="flow">
		{#if pricing.isPending}
			<p class="quiet">Loading discounts…</p>
		{:else if pricing.isError}
			<Placeholder icon="circle-alert" headline="We could not load discounts" body="Try reloading the page." />
		{:else}
			{#if !pricing.data.stripe_configured}
				<Banner tone="warn" title="No Stripe key">You can read discounts here but not create them.</Banner>
			{/if}

			<section class="flow-section">
				<div class="flow-section-head">
					<h2>Sales</h2>
					<Explain title="How a sale works" label="How it works">
						<p>
							A sale takes a percentage off every plan between its first and last day (UTC). Every
							plan checkout applies it by itself, and the pricing pages strike the old price and
							show the banner.
						</p>
						<p>
							Only one sale can run at a time. Packs are never on sale. Ending a sale early deletes
							its Stripe coupon, so no new checkout gets it; people who already paid keep their
							discount.
						</p>
					</Explain>
				</div>
				{@render list(sales, 'No sales yet.')}
			</section>

			<section class="flow-section">
				<div class="flow-section-head"><h2>One-off discounts</h2></div>
				{@render list(oneOffs, 'No one-off discounts yet.')}
			</section>

			<section class="flow-section">
				<div class="flow-section-head">
					<h2>Discount codes</h2>
					<Explain title="How codes work" label="How it works">
						<p>
							Sellers type a code on the Billing page, or on Stripe's checkout page when nothing
							else is applied. A code only works on the prices you pick, and not after its last
							day.
						</p>
					</Explain>
				</div>
				{@render list(codes, 'No codes yet.')}
			</section>

			<section class="flow-section op-write">
				<div class="flow-section-head"><h2>New</h2></div>
				<div class="flow-choice" role="radiogroup" aria-label="Kind">
					{#each KINDS as choice (choice.id)}
						<button
							type="button"
							role="radio"
							aria-checked={kind === choice.id}
							onclick={() => {
								kind = choice.id;
								if (choice.id === 'sale') {
									draft.by = 'percent';
									draft.keys = [];
								}
							}}
						>
							{choice.label}<span class="sub">{choice.sub}</span>
						</button>
					{/each}
				</div>

				<div class="pricing-fields">
					{#if kind === 'code'}
						<Field label="Code" id="pricing-code" required hint="Letters, digits, - and _. Case does not matter.">
							<input id="pricing-code" type="text" bind:value={code} placeholder="TEACHER10" autocomplete="off" />
						</Field>
					{/if}
					<Field label="Name" id="pricing-name" required hint="Shown in Stripe and on receipts.">
						<input id="pricing-name" type="text" bind:value={draft.name} placeholder="Halloween 2026" />
					</Field>

					{#if kind !== 'sale'}
						<div class="flow-choice" role="radiogroup" aria-label="Discount by">
							<button type="button" role="radio" aria-checked={draft.by === 'percent'} onclick={() => (draft.by = 'percent')}>
								Percentage
							</button>
							<button type="button" role="radio" aria-checked={draft.by === 'amount'} onclick={() => (draft.by = 'amount')}>
								Amount (USD)
							</button>
						</div>
					{/if}
					{#if draft.by === 'percent'}
						<Field label="Percent off" id="pricing-percent" required>
							<input id="pricing-percent" type="number" min="1" max="100" step="1" bind:value={draft.percent} />
						</Field>
					{:else}
						<Field label="Dollars off" id="pricing-dollars" required>
							<input id="pricing-dollars" type="number" min="0.01" step="0.01" bind:value={draft.dollars} />
						</Field>
					{/if}

					<Field label="First day" id="pricing-from" required>
						<input id="pricing-from" type="date" bind:value={draft.from} />
					</Field>
					<Field label="Last day" id="pricing-until" required hint="Included. Days are UTC.">
						<input id="pricing-until" type="date" bind:value={draft.until} />
					</Field>

					<Field label="Applies to" id="pricing-duration" hint="For a monthly plan, months count invoices.">
						<select id="pricing-duration" bind:value={draft.duration}>
							<option value="once">First payment only</option>
							<option value="repeating">A number of months</option>
						</select>
					</Field>
					{#if draft.duration === 'repeating'}
						<Field label="Months" id="pricing-months" required>
							<input id="pricing-months" type="number" min="1" max="36" step="1" bind:value={draft.months} />
						</Field>
					{/if}

					{#if kind === 'sale'}
						<Field label="Banner" id="pricing-banner" required hint="Shown on the pricing pages while the sale runs.">
							<input
								id="pricing-banner"
								type="text"
								maxlength="140"
								bind:value={banner}
								placeholder="Halloween sale: 25% off every plan until 31 October"
							/>
						</Field>
						<Field label="Banner link" id="pricing-href" hint="Optional. Starts with / or https://.">
							<input id="pricing-href" type="text" bind:value={bannerHref} placeholder="/pricing" />
						</Field>
						<Field label="Theme" id="pricing-theme" hint="Ties the sale to a seasonal theme.">
							<select id="pricing-theme" bind:value={theme}>
								{#each THEMES as option (option.id)}
									<option value={option.id}>{option.label}</option>
								{/each}
							</select>
						</Field>
					{:else}
						<fieldset class="pricing-keys">
							<legend>Prices {kind === 'code' ? '(none picked = every plan)' : ''}</legend>
							{#each priceKeys as price (price.key)}
								<label>
									<input
										type="checkbox"
										checked={draft.keys.includes(price.key)}
										onchange={() => toggleKey(price.key)}
									/>
									<span class="mono">{price.key}</span>
								</label>
							{/each}
						</fieldset>
					{/if}

					{#if kind === 'code'}
						<Field label="Use limit" id="pricing-limit" hint="Empty for no limit.">
							<input id="pricing-limit" type="number" min="1" step="1" bind:value={limit} />
						</Field>
					{/if}
				</div>

				<div class="op-grant-foot">
					<Button
						tier="primary"
						icon="check"
						disabled={refusal !== null || saving.isPending}
						reason={refusal ?? (saving.isPending ? 'Saving.' : undefined)}
						onclick={() => saving.mutate()}
					>
						{saving.isPending ? 'Saving…' : 'Save and create in Stripe'}
					</Button>
					{#if refusal !== null}<span class="quiet">{refusal}</span>{/if}
				</div>
			</section>
		{/if}
	</div>
</div>

<style>
	.pricing-fields {
		display: grid;
		gap: 12px;
		grid-template-columns: 1fr;
		margin-top: 12px;
	}

	@media (min-width: 720px) {
		.pricing-fields {
			grid-template-columns: 1fr 1fr;
		}
	}

	.pricing-keys {
		grid-column: 1 / -1;
		display: flex;
		flex-wrap: wrap;
		gap: 8px 16px;
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		padding: 8px 12px 12px;
		margin: 0;
	}

	.pricing-keys legend {
		font-size: 13px;
		color: var(--muted);
		padding: 0 4px;
	}

	.pricing-keys label {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		font-size: 13px;
	}
</style>
