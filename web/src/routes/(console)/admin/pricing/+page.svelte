<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import Icon from '$lib/Icon.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill, { type Tone } from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import { dollars } from '$lib/pages/account/plans';
	import {
		STATE_LABEL,
		codeRows,
		discountFigure,
		durationLabel,
		formPreview,
		formProblems,
		reachLabel,
		termsBody,
		windowLabel,
		type DiscountKind,
		type DiscountState,
		type DiscountView,
		type FormField,
		type PricingForm,
		type Theme
	} from '$lib/pages/admin/pricing';
	import { SEASONS } from '$lib/site';
	import '$lib/flow.css';
	import '$lib/styles/data.css';

	const queryClient = useQueryClient();

	const pricing = createQuery(() => ({
		queryKey: queryKeys.adminPricing,
		queryFn: () => api.adminPricing()
	}));

	// Top down, as an operator reads the page: what is running, what is
	// coming, the codes, then the form. Codes live in their own table, so
	// the two lists above hold sales and one-off discounts only.
	const rows = $derived(pricing.data?.discounts ?? []);
	const priced = $derived(rows.filter((row) => row.kind !== 'code'));
	const running = $derived(priced.filter((row) => row.state === 'open'));
	const scheduled = $derived(priced.filter((row) => row.state === 'scheduled'));
	const finished = $derived(priced.filter((row) => row.state === 'over' || row.state === 'ended'));
	const codes = $derived(codeRows(rows));
	const priceKeys = $derived(pricing.data?.price_keys ?? []);
	const stripeless = $derived(pricing.data?.stripe_configured === false);

	// UTC, because the server's windows are UTC days.
	const today = new Date().toISOString().slice(0, 10);

	const TONE: Record<DiscountState, Tone> = {
		scheduled: 'run',
		open: 'ok',
		over: 'soon',
		ended: 'soon'
	};

	const KIND_LABEL: Record<DiscountKind, string> = {
		sale: 'Sale',
		one_off: 'One-off discount',
		code: 'Discount code'
	};

	// ------------------------------------------------------------- the form

	function blank(kind: DiscountKind): PricingForm {
		return {
			kind,
			terms: {
				name: '',
				by: 'percent',
				percent: null,
				dollars: null,
				duration: 'once',
				months: null,
				keys: [],
				from: '',
				until: ''
			},
			banner: '',
			bannerHref: '',
			theme: 'none',
			code: '',
			limit: null
		};
	}

	let form = $state<PricingForm>(blank('sale'));
	const problems = $derived(formProblems(form, today));
	const preview = $derived(formPreview(form));

	// A field says what is wrong with it once it has been left, or once Save
	// was pressed — not while the form is still empty.
	let touched = $state<Partial<Record<FormField, boolean>>>({});
	let tried = $state(false);
	const problemOf = (field: FormField) =>
		tried || touched[field] === true ? (problems[field] ?? null) : null;
	const leave = (field: FormField) => () => (touched = { ...touched, [field]: true });

	const saving = createMutation(() => ({
		mutationFn: () => {
			const terms = termsBody(form.terms);
			if (form.kind === 'sale') {
				return api.createSale({
					name: terms.name,
					percent_off: terms.percent_off ?? 0,
					from: terms.from,
					until: terms.until,
					banner: form.banner.trim(),
					...(form.bannerHref.trim() === '' ? {} : { banner_href: form.bannerHref.trim() }),
					...(form.theme === 'none' ? {} : { theme: form.theme }),
					duration: terms.duration,
					...(terms.duration_months === undefined
						? {}
						: { duration_months: terms.duration_months })
				});
			}
			if (form.kind === 'one_off') {
				return api.createOneOff(terms);
			}
			return api.createCode({
				...terms,
				code: form.code.trim(),
				...(form.limit === null ? {} : { max_redemptions: form.limit })
			});
		},
		onSuccess: async (saved: DiscountView) => {
			await queryClient.invalidateQueries({ queryKey: queryKeys.adminPricing });
			form = blank(form.kind);
			touched = {};
			tried = false;
			toast('info', `${saved.name} saved. Stripe has its coupon.`);
		},
		onError: (failure: Error) =>
			toast('error', failure instanceof ApiFailure ? failure.message : 'It was not saved.')
	}));

	function save() {
		tried = true;
		if (Object.keys(problems).length === 0) {
			saving.mutate();
		}
	}

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

	function end(id: string) {
		endingId = id;
		ending.mutate(id);
	}

	function toggleKey(key: string) {
		form.terms.keys = form.terms.keys.includes(key)
			? form.terms.keys.filter((held) => held !== key)
			: [...form.terms.keys, key];
		touched = { ...touched, keys: true };
	}

	function pickKind(kind: DiscountKind) {
		form.kind = kind;
		if (kind === 'sale') {
			form.terms.by = 'percent';
			form.terms.keys = [];
		}
	}

	const KINDS: { id: DiscountKind; label: string; sub: string }[] = [
		{ id: 'sale', label: 'Sale', sub: 'Every plan, with a banner on the pricing pages.' },
		{ id: 'one_off', label: 'One-off discount', sub: 'The prices you pick, no code needed.' },
		{ id: 'code', label: 'Discount code', sub: 'Sellers type it at checkout.' }
	];

	/** The site themes, one for one with the Site page's picker. */
	const THEMES: { id: Theme | 'none'; label: string }[] = [
		{ id: 'none', label: 'No theme' },
		...SEASONS.map((entry) => ({ id: entry.name, label: entry.label }))
	];

	function themeLabel(theme: Theme): string {
		return SEASONS.find((entry) => entry.name === theme)?.label ?? theme;
	}
</script>

{#snippet endButton(id: string, word: string)}
	<Button
		small
		danger
		icon="circle-x"
		disabled={ending.isPending}
		reason={ending.isPending ? 'Ending one already.' : undefined}
		onclick={() => end(id)}
	>
		{endingId === id ? 'Ending…' : word}
	</Button>
{/snippet}

{#snippet discountCard(row: DiscountView)}
	<article class="pr-card" class:off={row.state === 'over' || row.state === 'ended'}>
		<div class="pr-card-main">
			<div class="pr-card-title">
				<h3>{row.name}</h3>
				<StatusPill tone={TONE[row.state]} label={STATE_LABEL[row.state]} />
				<span class="pr-kind">{KIND_LABEL[row.kind]}</span>
				{#if row.in_stripe === false && (row.state === 'open' || row.state === 'scheduled')}
					<StatusPill tone="bad" label="Missing in Stripe" />
				{/if}
			</div>
			<p class="pr-card-figure">
				{discountFigure(row)} {reachLabel(row.price_keys)}, {durationLabel(row)}
			</p>
			<p class="pr-card-line">
				<Icon name="calendar" size={14} />
				{windowLabel(row.from, row.until)}
				<span class="quiet">({row.from} to {row.until}, UTC)</span>
			</p>
			{#if row.banner}
				<p class="pr-card-line">
					<Icon name="tag" size={14} />
					“{row.banner}”{row.theme ? ` · ${themeLabel(row.theme)} theme` : ''}
				</p>
			{/if}
			<p class="pr-card-coupon mono">{row.stripe_coupon_id}</p>
		</div>
		{#if row.state === 'open' || row.state === 'scheduled'}
			<div class="pr-card-acts">
				{@render endButton(row.id, row.state === 'open' ? 'End now' : 'Cancel')}
			</div>
		{/if}
	</article>
{/snippet}

{#snippet problem(field: FormField, hint: string)}
	{@const says = problemOf(field)}
	{#if says === null}
		<span class="hint">{hint}</span>
	{:else}
		<span class="hint pr-error" role="alert">
			<Icon name="circle-alert" size={13} />
			{says}
		</span>
	{/if}
{/snippet}

<div class="page flow-page">
	<PageHead
		icon="tag"
		title="Pricing"
		description="Sales, one-off discounts and discount codes. Each one is a Stripe coupon."
	/>

	{#if pricing.isPending}
		<p class="quiet">Loading discounts…</p>
	{:else if pricing.isError}
		<Placeholder
			icon="circle-alert"
			headline="We could not load discounts"
			body="Try reloading the page."
		/>
	{:else}
		{#if stripeless}
			<Banner tone="warn" title="No Stripe key">
				You can read discounts here but not create them.
			</Banner>
		{/if}

		<ol class="pr-steps">
			<li class="pr-step">
				<span class="pr-step-mark" aria-hidden="true">1</span>
				<section class="pr-step-body" aria-labelledby="pr-running">
					<div class="pr-step-head">
						<h2 id="pr-running">Running now</h2>
						<Explain title="How a sale works" label="How it works">
							<p>
								A sale takes a percentage off every plan between its first and last day (UTC).
								Every plan checkout applies it by itself, and the pricing pages strike the old
								price and show the banner.
							</p>
							<p>
								Only one sale can run at a time. Packs are never on sale. Ending a sale early
								deletes its Stripe coupon, so no new checkout gets it; people who already paid keep
								their discount.
							</p>
						</Explain>
					</div>
					{#if running.length === 0}
						<p class="data-empty">Nothing is running. Checkouts charge list prices.</p>
					{:else}
						{#each running as row (row.id)}{@render discountCard(row)}{/each}
					{/if}
				</section>
			</li>

			<li class="pr-step">
				<span class="pr-step-mark" aria-hidden="true">2</span>
				<section class="pr-step-body" aria-labelledby="pr-scheduled">
					<div class="pr-step-head"><h2 id="pr-scheduled">Scheduled</h2></div>
					{#if scheduled.length === 0}
						<p class="data-empty">Nothing is scheduled.</p>
					{:else}
						{#each scheduled as row (row.id)}{@render discountCard(row)}{/each}
					{/if}
				</section>
			</li>

			<li class="pr-step">
				<span class="pr-step-mark" aria-hidden="true">3</span>
				<section class="pr-step-body" aria-labelledby="pr-codes">
					<div class="pr-step-head">
						<h2 id="pr-codes">Codes</h2>
						<Explain title="How codes work" label="How it works">
							<p>
								Sellers type a code on the Billing page, or on Stripe's checkout page when nothing
								else is applied. A code only works on the prices you pick, and not after its last
								day.
							</p>
						</Explain>
					</div>
					{#if codes.length === 0}
						<p class="data-empty">No codes yet.</p>
					{:else}
						<div class="data-table-wrap">
							<table class="data-table stack">
								<thead>
									<tr>
										<th scope="col">Code</th>
										<th scope="col">Off</th>
										<th scope="col">Window</th>
										<th scope="col">Uses</th>
										<th scope="col">State</th>
										<th scope="col" class="act"><span class="sr-only">End</span></th>
									</tr>
								</thead>
								<tbody>
									{#each codes as row (row.id)}
										<tr class:off={!row.live}>
											<td data-label="Code">
												<span class="mono pr-code">{row.code}</span>
												<span class="sub">{row.name}</span>
											</td>
											<td data-label="Off">{row.off}</td>
											<td class="nowrap" data-label="Window">{row.window}</td>
											<td class="nowrap" data-label="Uses">{row.uses}</td>
											<td data-label="State">
												<StatusPill tone={TONE[row.state]} label={STATE_LABEL[row.state]} />
											</td>
											<td class="act" data-label="">
												{#if row.live}{@render endButton(row.discountId, 'End')}{/if}
											</td>
										</tr>
									{/each}
								</tbody>
							</table>
						</div>
					{/if}
				</section>
			</li>

			<li class="pr-step">
				<span class="pr-step-mark" aria-hidden="true">4</span>
				<section class="pr-step-body" aria-labelledby="pr-create">
					<div class="pr-step-head"><h2 id="pr-create">Create</h2></div>
					<form
						class="pr-form"
						novalidate
						onsubmit={(event) => {
							event.preventDefault();
							save();
						}}
					>
						<div class="pr-form-kind">
							<div class="segmented" role="radiogroup" aria-label="Kind">
								{#each KINDS as choice (choice.id)}
									<button
										type="button"
										role="radio"
										aria-checked={form.kind === choice.id}
										onclick={() => pickKind(choice.id)}>{choice.label}</button
									>
								{/each}
							</div>
							<p class="quiet">{KINDS.find((choice) => choice.id === form.kind)?.sub}</p>
						</div>

						<div class="pr-grid">
							{#if form.kind === 'code'}
								<Field
									label="Code"
									id="pricing-code"
									required
								>
									<input
										id="pricing-code"
										type="text"
										autocomplete="off"
										bind:value={form.code}
										onblur={leave('code')}
										aria-invalid={problemOf('code') !== null}
										placeholder="TEACHER10"
									/>
									{@render problem('code', 'Letters, digits, - and _. Case does not matter.')}
								</Field>
							{/if}
							<Field label="Name" id="pricing-name" required>
								<input
									id="pricing-name"
									type="text"
									bind:value={form.terms.name}
									onblur={leave('name')}
									aria-invalid={problemOf('name') !== null}
									placeholder="Halloween 2026"
								/>
								{@render problem('name', 'Shown in Stripe and on receipts.')}
							</Field>

							{#if form.kind !== 'sale'}
								<div class="field">
									<span id="pricing-by-label">Discount by</span>
									<div class="segmented" role="radiogroup" aria-labelledby="pricing-by-label">
										<button
											type="button"
											role="radio"
											aria-checked={form.terms.by === 'percent'}
											onclick={() => (form.terms.by = 'percent')}>Percentage</button
										>
										<button
											type="button"
											role="radio"
											aria-checked={form.terms.by === 'amount'}
											onclick={() => (form.terms.by = 'amount')}>Amount (USD)</button
										>
									</div>
								</div>
							{/if}
							{#if form.kind === 'sale' || form.terms.by === 'percent'}
								<Field
									label="Percent off"
									id="pricing-percent"
									required
								>
									<input
										id="pricing-percent"
										type="number"
										min="1"
										max="100"
										step="1"
										bind:value={form.terms.percent}
										onblur={leave('percent')}
										aria-invalid={problemOf('percent') !== null}
										placeholder="25"
									/>
									{@render problem('percent', 'A whole number from 1 to 100.')}
								</Field>
							{:else}
								<Field label="Dollars off" id="pricing-dollars" required>
									<input
										id="pricing-dollars"
										type="number"
										min="0.01"
										step="0.01"
										bind:value={form.terms.dollars}
										onblur={leave('dollars')}
										aria-invalid={problemOf('dollars') !== null}
										placeholder="10"
									/>
									{@render problem('dollars', 'Taken off in USD.')}
								</Field>
							{/if}

							<div class="pr-span pr-range">
								<Field label="First day" id="pricing-from" required>
									<input
										id="pricing-from"
										type="date"
										bind:value={form.terms.from}
										onblur={leave('from')}
										aria-invalid={problemOf('from') !== null}
									/>
									{@render problem('from', 'Days are UTC.')}
								</Field>
								<Field label="Last day" id="pricing-until" required>
									<input
										id="pricing-until"
										type="date"
										min={form.terms.from || undefined}
										bind:value={form.terms.until}
										onblur={leave('until')}
										aria-invalid={problemOf('until') !== null}
									/>
									{@render problem('until', 'Included.')}
								</Field>
							</div>

							<Field
								label="Applies to"
								id="pricing-duration"
								hint="For a monthly plan, months count invoices."
							>
								<select id="pricing-duration" bind:value={form.terms.duration}>
									<option value="once">First payment only</option>
									<option value="repeating">A number of months</option>
								</select>
							</Field>
							{#if form.terms.duration === 'repeating'}
								<Field label="Months" id="pricing-months" required>
									<input
										id="pricing-months"
										type="number"
										min="1"
										max="36"
										step="1"
										bind:value={form.terms.months}
										onblur={leave('months')}
										aria-invalid={problemOf('months') !== null}
									/>
									{@render problem('months', 'From 1 to 36.')}
								</Field>
							{:else}
								<span class="pr-gap" aria-hidden="true"></span>
							{/if}

							{#if form.kind === 'sale'}
								<div class="pr-span">
									<Field
										label="Banner"
										id="pricing-banner"
										required
									>
										<input
											id="pricing-banner"
											type="text"
											maxlength="140"
											bind:value={form.banner}
											onblur={leave('banner')}
											aria-invalid={problemOf('banner') !== null}
											placeholder="Halloween sale: 25% off every plan until 31 October"
										/>
										{@render problem('banner', 'Shown on the pricing pages while the sale runs. Up to 140 characters.')}
									</Field>
								</div>
								<Field label="Banner link" id="pricing-href" hint="Optional. Starts with / or https://.">
									<input
										id="pricing-href"
										type="text"
										bind:value={form.bannerHref}
										placeholder="/pricing"
									/>
								</Field>
								<Field label="Theme" id="pricing-theme" hint="Ties the sale to a seasonal theme.">
									<select id="pricing-theme" bind:value={form.theme}>
										{#each THEMES as option (option.id)}
											<option value={option.id}>{option.label}</option>
										{/each}
									</select>
								</Field>
							{:else}
								<fieldset class="pr-span pr-keys">
									<legend>
										Prices{#if form.kind === 'one_off'}<span class="req">Required</span>{/if}
									</legend>
									<div class="pr-key-list">
										{#each priceKeys as price (price.key)}
											<label class="pr-key">
												<input
													type="checkbox"
													checked={form.terms.keys.includes(price.key)}
													onchange={() => toggleKey(price.key)}
												/>
												<span class="mono">{price.key}</span>
												<span class="quiet">{dollars(price.list_cents)}</span>
											</label>
										{/each}
									</div>
									{@render problem(
										'keys',
										form.kind === 'code' ? 'None picked means every plan.' : 'Pick one or more.'
									)}
								</fieldset>
								{#if form.kind === 'code'}
									<Field label="Use limit" id="pricing-limit">
										<input
											id="pricing-limit"
											type="number"
											min="1"
											step="1"
											bind:value={form.limit}
											onblur={leave('limit')}
											aria-invalid={problemOf('limit') !== null}
										/>
										{@render problem('limit', 'Empty for no limit.')}
									</Field>
								{/if}
							{/if}
						</div>

						<div class="pr-form-foot">
							<p class="pr-preview" aria-live="polite">
								<Icon name="info" size={14} />
								<span>{preview}</span>
							</p>
							<Button
								tier="primary"
								type="submit"
								icon="check"
								disabled={stripeless || saving.isPending}
								reason={stripeless
									? 'This server has no Stripe key, so nothing can be created here.'
									: saving.isPending
										? 'Saving.'
										: undefined}
							>
								{saving.isPending ? 'Saving…' : 'Save and create in Stripe'}
							</Button>
						</div>
					</form>
				</section>
			</li>
		</ol>

		{#if finished.length > 0}
			<details class="pr-past">
				<summary>Finished and ended ({finished.length})</summary>
				<div class="pr-past-list">
					{#each finished as row (row.id)}{@render discountCard(row)}{/each}
				</div>
			</details>
		{/if}
	{/if}
</div>

<style>
	/* --- the steps: a numbered rail down the left, one section each ------ */

	.pr-steps {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
	}

	.pr-step {
		position: relative;
		display: grid;
		grid-template-columns: 28px minmax(0, 1fr);
		gap: var(--s-4);
		padding-bottom: var(--s-6);
	}

	.pr-step:not(:last-child)::before {
		content: '';
		position: absolute;
		top: 32px;
		bottom: var(--s-1);
		left: 13px;
		width: 2px;
		border-radius: 1px;
		background: var(--line);
	}

	.pr-step-mark {
		display: grid;
		place-items: center;
		width: 28px;
		height: 28px;
		border-radius: var(--r-pill);
		background: var(--accent-soft);
		color: var(--ok-ink);
		font-size: 13px;
		font-weight: 700;
	}

	.pr-step-body {
		display: flex;
		flex-direction: column;
		gap: var(--s-3);
		min-width: 0;
	}

	.pr-step-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--s-3);
		min-height: 28px;
	}

	.pr-step-head h2 {
		margin: 0;
		font-family: var(--display);
		font-size: 19px;
		font-weight: 600;
		letter-spacing: -0.01em;
	}

	/* --- a sale or one-off discount -------------------------------------- */

	.pr-card {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: var(--s-4);
		padding: var(--s-4) var(--s-5);
		background: var(--card);
		border: 1px solid var(--line);
		border-left: 3px solid var(--accent);
		border-radius: var(--r-panel);
	}

	.pr-card.off {
		border-left-color: var(--line);
	}

	.pr-card.off .pr-card-main {
		color: var(--muted);
	}

	.pr-card-main {
		display: flex;
		flex-direction: column;
		gap: var(--s-1);
		min-width: 0;
	}

	.pr-card-main p {
		margin: 0;
		font-size: 13px;
		line-height: 1.45;
	}

	.pr-card-title {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: var(--s-2);
		margin-bottom: var(--s-1);
	}

	.pr-card-title h3 {
		margin: 0;
		font-size: 15px;
		font-weight: 600;
	}

	.pr-kind {
		font-size: 12px;
		color: var(--muted);
	}

	.pr-card-main .pr-card-figure {
		font-size: 14px;
		font-weight: 600;
	}

	.pr-card-line {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: var(--s-2);
	}

	.pr-card-line :global(svg) {
		flex: none;
		color: var(--muted);
	}

	.pr-card-main .pr-card-coupon {
		font-size: 11.5px;
		color: var(--muted);
		overflow-wrap: anywhere;
	}

	.pr-card-acts {
		flex: none;
	}

	.pr-code {
		font-weight: 600;
	}

	/* --- the create form ------------------------------------------------- */

	.pr-form {
		display: flex;
		flex-direction: column;
		gap: var(--s-5);
		padding: var(--s-5);
		background: var(--card);
		border: 1px solid var(--line);
		border-radius: var(--r-card);
		box-shadow: var(--sh-1);
	}

	.pr-form-kind {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--s-2) var(--s-4);
	}

	.pr-form-kind p {
		margin: 0;
	}

	.pr-grid {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: var(--s-4) var(--s-5);
		align-items: start;
	}

	.pr-span {
		grid-column: 1 / -1;
	}

	.pr-range {
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
		gap: var(--s-4) var(--s-3);
		align-items: start;
	}

	.pr-gap {
		display: none;
	}

	@media (min-width: 721px) {
		.pr-grid {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}

		.pr-range {
			column-gap: var(--s-5);
		}

		.pr-gap {
			display: block;
		}
	}

	/* The number and date inputs take the same frame as the text ones, so
	   the two columns line up edge for edge. */
	.pr-form :global(input[type='date']),
	.pr-form :global(input[type='number']),
	.pr-form :global(input[type='text']),
	.pr-form :global(select) {
		width: 100%;
		box-sizing: border-box;
		min-height: var(--control-h);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		background: var(--card);
		padding: 8px 11px;
		font: inherit;
		font-size: 13px;
		font-weight: 400;
		color: var(--ink);
	}

	.pr-form :global(input[aria-invalid='true']) {
		border-color: var(--bad);
	}

	.pr-form .hint.pr-error {
		display: inline-flex;
		align-items: center;
		gap: var(--s-1);
		font-size: 12.5px;
		font-weight: 500;
		color: var(--bad-ink);
	}

	.pr-keys {
		display: flex;
		flex-direction: column;
		gap: var(--s-2);
		margin: 0;
		padding: 0;
		border: 0;
		min-width: 0;
	}

	.pr-keys legend {
		padding: 0;
		margin-bottom: var(--s-2);
		font-size: 12.5px;
		font-weight: 600;
	}

	.pr-keys .hint {
		font-size: 12.5px;
		color: var(--muted);
	}

	.pr-key-list {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
		gap: var(--s-2);
	}

	.pr-key {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		padding: var(--s-2) var(--s-3);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		font-size: 13px;
		cursor: pointer;
	}

	.pr-key:has(input:checked) {
		border-color: var(--accent);
		background: var(--accent-soft);
	}

	.pr-key .quiet {
		margin-left: auto;
	}

	.pr-form-foot {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: var(--s-3) var(--s-4);
		padding-top: var(--s-4);
		border-top: 1px solid var(--line);
	}

	.pr-preview {
		flex: 1 1 280px;
		display: flex;
		align-items: flex-start;
		gap: var(--s-2);
		margin: 0;
		font-size: 13px;
		line-height: 1.45;
		color: var(--muted);
	}

	.pr-preview :global(svg) {
		flex: none;
		margin-top: 2px;
	}

	.pr-preview span {
		color: var(--ink);
	}

	/* --- history --------------------------------------------------------- */

	.pr-past {
		margin-left: calc(28px + var(--s-4));
	}

	.pr-past summary {
		cursor: pointer;
		font-size: 13px;
		font-weight: 600;
		color: var(--muted);
	}

	.pr-past-list {
		display: flex;
		flex-direction: column;
		gap: var(--s-3);
		margin-top: var(--s-3);
	}

	@media (max-width: 620px) {
		.pr-step {
			grid-template-columns: minmax(0, 1fr);
			gap: var(--s-3);
			padding-bottom: var(--s-5);
		}

		.pr-step:not(:last-child)::before {
			display: none;
		}

		.pr-step-mark {
			display: none;
		}

		.pr-card {
			flex-direction: column;
			padding: var(--s-4);
		}

		.pr-form {
			padding: var(--s-4);
		}

		/* Three kinds across a phone: each takes a third and may wrap to
		   two lines, rather than the last one running off the card. */
		.pr-form-kind .segmented {
			display: flex;
			width: 100%;
			border-radius: var(--r-panel);
		}

		.pr-form-kind .segmented button {
			flex: 1 1 0;
			padding: 6px 8px;
			white-space: normal;
			line-height: 1.25;
			border-radius: var(--r-field);
		}

		.pr-form-foot :global(.cta) {
			width: 100%;
			justify-content: center;
		}

		.pr-past {
			margin-left: 0;
		}
	}
</style>
