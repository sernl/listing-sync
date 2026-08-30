<script lang="ts">
	import { createQueries, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { ApiFailure, api, type UploadedView, type VocabularyView } from '$lib/api';
	import {
		axisControls,
		axisKey,
		createBodyOf,
		emptyDraft,
		licenceOptions,
		measure,
		payloadRefusal,
		priceOf,
		quotaSentence,
		refusalsOf,
		submittable,
		toggleSubject,
		CURRENCY_OPTIONS,
		type Draft
	} from '$lib/authoring';
	import { AUTHORABLE_PLATFORMS, platformTitle } from '$lib/platforms';
	import AxisField from '$lib/AxisField.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import UploadField from '$lib/UploadField.svelte';
	import type { InventoryId } from '$lib/generated/vocab';

	const queryClient = useQueryClient();

	// One read per platform rather than one for all of them: each is its own
	// cache entry, so this page and the product page share whichever they both
	// need instead of refetching a combined payload.
	const vocabularies = createQueries(() => ({
		queries: AUTHORABLE_PLATFORMS.map((inventory) => ({
			queryKey: queryKeys.vocabulary(inventory),
			queryFn: () => api.vocabulary(inventory),
			staleTime: Infinity
		})),
		combine: (results: { data?: VocabularyView; isPending: boolean; isError: boolean }[]) => ({
			known: new Map(
				results.flatMap((result) =>
					result.data === undefined
						? []
						: [[result.data.inventory, result.data] as [InventoryId, VocabularyView]]
				)
			),
			pending: results.some((result) => result.isPending),
			failed: results.some((result) => result.isError)
		})
	}));

	// The canonical subjects: ours rather than any one marketplace's, so one read
	// serves the whole form. Cached like a vocabulary — forty-odd terms that
	// change only when the server does are not worth refetching.
	const subjects = createQuery(() => ({
		queryKey: queryKeys.taxonomyTerms('subject'),
		queryFn: () => api.terms('subject').then((view) => view.terms),
		staleTime: Infinity
	}));

	const known = $derived(vocabularies.known);
	const loadingVocabulary = $derived(vocabularies.pending);
	const vocabularyFailed = $derived(vocabularies.failed);

	let draft = $state<Draft>(emptyDraft());
	let uploaded = $state<UploadedView | null>(null);
	let keepWhole = $state(false);
	let creating = $state(false);
	let serverRefusal = $state<string | null>(null);

	function takeUpload(result: UploadedView | null) {
		uploaded = result;
		draft = {
			...draft,
			payload: result?.payload ?? [],
			cover: result?.cover ?? null,
			previews: result?.previews ?? []
		};
	}

	function togglePlatform(inventory: InventoryId, on: boolean) {
		draft = {
			...draft,
			inventories: on
				? [...draft.inventories, inventory]
				: draft.inventories.filter((held) => held !== inventory)
		};
	}

	function setAxis(inventory: InventoryId, native: string, values: string[]) {
		draft = { ...draft, axes: { ...draft.axes, [axisKey(inventory, native)]: values } };
	}

	/** Why this platform cannot be selected at all, or `null` where it can.
	 *
	 *  The payload rule is the only one: a TPT create takes exactly one file, so
	 *  a multi-file product is TES-only and the tick box says so rather than
	 *  letting the seller choose a platform whose write is already refused. */
	function unselectable(inventory: InventoryId): string | null {
		const view = known.get(inventory);
		if (view === undefined || draft.payload.length === 0) {
			return null;
		}
		const refusal = payloadRefusal(view.authoring.payload_files, draft.payload.length);
		return refusal === null ? null : `This platform ${refusal}.`;
	}

	// Selecting a platform, then uploading a file it cannot carry, would leave
	// it ticked and refused. Dropping it here keeps the tick boxes and the
	// refusal list saying the same thing.
	$effect(() => {
		const kept = draft.inventories.filter((inventory) => unselectable(inventory) === null);
		if (kept.length !== draft.inventories.length) {
			draft = { ...draft, inventories: kept };
		}
	});

	const selected = $derived(
		AUTHORABLE_PLATFORMS.filter((inventory) => draft.inventories.includes(inventory))
	);
	const licensing = $derived(
		selected.filter((inventory) => known.get(inventory)?.authoring.licence !== undefined)
	);
	const licenceChoices = $derived(
		licensing.length === 0 ? [] : licenceOptions(known.get(licensing[0]), draft.branch)
	);
	const refusals = $derived(refusalsOf(draft, known));
	const canCreate = $derived(submittable(refusals) && !loadingVocabulary && !creating);

	// A licence the price branch no longer offers is a licence Tes refuses to
	// write, so switching between free and paid clears it rather than leaving a
	// selection the seller can no longer see in the list.
	$effect(() => {
		if (draft.licence !== null && !licenceChoices.some((choice) => choice.id === draft.licence)) {
			draft = { ...draft, licence: null };
		}
	});

	function capOf(inventory: InventoryId, field: string) {
		return known.get(inventory)?.canonical.find((entry) => entry.field === field)?.cap;
	}

	/** The tightest cap any selected platform declares for a field, so one
	 *  counter speaks for the whole selection. */
	const bodyCap = $derived.by(() => {
		const caps = selected
			.map((inventory) => capOf(inventory, 'description'))
			.filter((cap) => cap !== undefined);
		return caps.length === 0 ? null : caps.reduce((a, b) => (a.limit <= b.limit ? a : b));
	});

	async function create(event: SubmitEvent) {
		event.preventDefault();
		const body = createBodyOf(draft, known);
		if (body === null || !canCreate) {
			return;
		}
		creating = true;
		serverRefusal = null;
		try {
			const created = await api.createProduct(body);
			await queryClient.invalidateQueries({ queryKey: queryKeys.products });
			await queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
			toast(
				'info',
				created.mappings.length === 1
					? 'Draft created on one marketplace. Publish when you are ready.'
					: `Draft created on ${created.mappings.length} marketplaces. Publish when you are ready.`
			);
			await goto(`/listings/${created.product}`);
		} catch (failure) {
			serverRefusal = createRefusalOf(failure);
		} finally {
			creating = false;
		}
	}

	function createRefusalOf(failure: unknown): string {
		if (!(failure instanceof ApiFailure)) {
			return 'The draft was not created.';
		}
		const entry = failure.body?.errors[0];
		switch (failure.code()) {
			case 'quota_exceeded':
				return quotaSentence(entry?.detail) ?? entry?.message ?? failure.message;
			case 'payload_missing':
				return 'The bytes have to be uploaded before the draft is created.';
			case 'upload_rejected':
				return `${failure.message} Upload the file again.`;
			default:
				return failure.message;
		}
	}
</script>

<div class="page">
	<PageHead
		icon="✚"
		title="New listing"
		description="Author the resource once, and choose which marketplaces carry it."
	>
		{#snippet aside()}
			<a class="btn" href="/listings">Cancel</a>
		{/snippet}
	</PageHead>

	{#if vocabularyFailed}
		<div class="attn">
			<div class="t">The marketplace requirements could not be read</div>
			<p>
				Without them this form cannot say what each platform needs, so creating is held back.
				Reload to try again.
			</p>
		</div>
	{/if}

	<form class="form wide" onsubmit={create}>
		<Panel
			title="The file"
			description="Upload first: a product with no file cannot be listed anywhere, and the catalogue refuses one."
		>
			<UploadField
				{uploaded}
				{keepWhole}
				onUploaded={takeUpload}
				onKeepWhole={(whole) => (keepWhole = whole)}
			/>
		</Panel>

		<Panel title="The listing" description="What a buyer reads, and what it costs them.">
			<label class="field">
				Title
				<input
					type="text"
					required
					maxlength="500"
					value={draft.title}
					oninput={(event) => (draft = { ...draft, title: event.currentTarget.value })}
				/>
			</label>

			<label class="field" style="margin-top: 12px">
				Description
				{#if bodyCap}
					{@const used = measure(draft.body, bodyCap.unit)}
					<span class="counter {used > bodyCap.limit ? 'over' : ''}">
						{used} of {bodyCap.limit}
					</span>
				{/if}
				<textarea
					value={draft.body}
					oninput={(event) => (draft = { ...draft, body: event.currentTarget.value })}
				></textarea>
			</label>

			<div class="inline-choices" style="margin-top: 10px">
				{#each ['Markdown', 'Html'] as const as format (format)}
					<label>
						<input
							type="radio"
							name="body-format"
							checked={draft.bodyFormat === format}
							onchange={() => (draft = { ...draft, bodyFormat: format })}
						/>
						{format}
					</label>
				{/each}
			</div>

			<div class="field" style="margin-top: 14px">
				<span id="subjects-label">Subjects</span>
				<span class="hint">
					What this resource teaches, from the taxonomy this catalogue holds rather than any
					one marketplace's. A subject left unset is resolved later in Reconciliation.
				</span>
				{#if subjects.isPending}
					<p class="quiet">Reading the taxonomy…</p>
				{:else if subjects.isError}
					<p class="refusal">
						The subjects could not be read. A draft can still be created without one, and the
						subject is resolved in Reconciliation.
					</p>
				{:else if (subjects.data ?? []).length === 0}
					<p class="quiet">
						The taxonomy holds no subjects yet, so there is nothing to choose from here.
					</p>
				{:else}
					<div class="pick-list" role="group" aria-labelledby="subjects-label">
						{#each subjects.data ?? [] as term (term.id)}
							<label class="choice">
								<input
									type="checkbox"
									checked={draft.subjects.includes(term.id)}
									onchange={(event) =>
										(draft = {
											...draft,
											subjects: toggleSubject(
												draft.subjects,
												term.id,
												event.currentTarget.checked
											)
										})}
								/>
								<span class="t">{term.label}</span>
							</label>
						{/each}
					</div>
					<span class="hint">
						{draft.subjects.length === 0
							? 'None chosen; Reconciliation will raise the subject when a platform needs one.'
							: `${draft.subjects.length} chosen.`}
					</span>
				{/if}
			</div>

			<div class="inline-choices" style="margin-top: 14px">
				<label>
					<input
						type="radio"
						name="price-branch"
						checked={draft.branch === 'free'}
						onchange={() => (draft = { ...draft, branch: 'free' })}
					/>
					Free
				</label>
				<label>
					<input
						type="radio"
						name="price-branch"
						checked={draft.branch === 'paid'}
						onchange={() => (draft = { ...draft, branch: 'paid' })}
					/>
					Paid
				</label>
			</div>

			{#if draft.branch === 'paid'}
				<div class="field-row" style="margin-top: 12px">
					<label class="field">
						Price
						<input
							type="text"
							inputmode="decimal"
							placeholder="4.50"
							value={draft.amount}
							oninput={(event) => (draft = { ...draft, amount: event.currentTarget.value })}
						/>
					</label>
					<label class="field">
						Currency
						<select
							value={draft.currency}
							onchange={(event) => (draft = { ...draft, currency: event.currentTarget.value })}
						>
							{#each CURRENCY_OPTIONS as option (option.value)}
								<option value={option.value}>{option.code}</option>
							{/each}
						</select>
					</label>
				</div>
			{/if}
		</Panel>

		<Panel
			title="The marketplaces"
			description="Chosen once, here. There is no way to add a platform to a listing afterwards, so pick every one this resource belongs on."
		>
			{#if loadingVocabulary}
				<p class="quiet">Reading what each marketplace requires…</p>
			{:else}
				{#each AUTHORABLE_PLATFORMS as inventory (inventory)}
					{@const refusal = unselectable(inventory)}
					<label class="choice {refusal === null ? '' : 'off'}">
						<input
							type="checkbox"
							checked={draft.inventories.includes(inventory)}
							disabled={refusal !== null}
							onchange={(event) => togglePlatform(inventory, event.currentTarget.checked)}
						/>
						<span class="t">{platformTitle(inventory)}</span>
						{#if refusal !== null}
							<span class="why bad">{refusal}</span>
						{:else if known.get(inventory)?.authoring.payload_files === 'exactly_one'}
							<span class="why">Takes exactly one file.</span>
						{:else}
							<span class="why">Carries every payload file this product holds.</span>
						{/if}
					</label>
				{/each}
			{/if}
		</Panel>

		{#if licensing.length > 0}
			<Panel
				title="The licence"
				description="The one field a marketplace declares required. It is yours to state: issuing a rights grant is never ours to guess."
			>
				<label class="field">
					Licence
					<span class="hint">
						{draft.branch === 'free'
							? 'A free resource carries a Creative Commons grant.'
							: 'A paid resource carries the marketplace’s own paid licence.'}
						The write is gated on the price, so changing it changes this list.
					</span>
					<select
						value={draft.licence ?? ''}
						onchange={(event) =>
							(draft = {
								...draft,
								licence: event.currentTarget.value === '' ? null : event.currentTarget.value
							})}
					>
						<option value="">Choose a licence</option>
						{#each licenceChoices as choice (choice.id)}
							<option value={choice.id}>{choice.label}</option>
						{/each}
					</select>
				</label>
				<p class="foot-note">
					Applies to {licensing.map(platformTitle).join(', ')}. Every Tes site serves the same
					licence list, so one choice answers all of them. This one is never ours to choose for
					you: issuing a rights grant is the seller's. Every other axis is yours to decide too
					today, because handing one to a best-fit choice needs a setting we do not model yet.
				</p>
			</Panel>
		{/if}

		{#if selected.length > 0}
			<Panel
				title="Per marketplace"
				description="Only where the platforms genuinely differ. Nothing here is required unless it says so."
			>
				{#each selected as inventory (inventory)}
					{@const view = known.get(inventory)}
					{#if view}
						<div class="platform-card">
							<h3>{platformTitle(inventory)}</h3>
							<p class="desc">
								{view.authoring.body_wire === 'renders_to_html'
									? 'Its wire is HTML; a Markdown description is rendered into it.'
									: 'It carries the description format you chose.'}
							</p>
							{#each axisControls(view) as control (control.native)}
								<AxisField
									{control}
									chosen={draft.axes[axisKey(inventory, control.native)] ?? []}
									onChange={(values) => setAxis(inventory, control.native, values)}
								/>
							{/each}
							{#each view.absent_axes as axis (axis)}
								<p class="disclosure">
									This platform has no {axis} field at all. A {axis} you state elsewhere does not
									reach it, and that loss is disclosed rather than silent.
								</p>
							{/each}
							{#if view.authoring.attestation}
								<p class="disclosure">
									Its create carries <b>{view.authoring.attestation.native}</b>, the copyright
									declaration held against your connection. It is not asked again here.
								</p>
							{/if}
							{#if view.authoring.price_floor_minor_units !== undefined}
								<p class="disclosure">
									It refuses a price below {view.authoring.price_floor_minor_units} minor units when
									the listing is written.
								</p>
							{/if}
						</div>
					{/if}
				{/each}
				<p class="foot-note">
					Topics are not set here: they come from the taxonomy this catalogue holds, and
					anything a platform cannot place is raised in Reconciliation.
				</p>
			</Panel>
		{/if}

		<Panel title="Create the draft" description="Nothing is sent to a marketplace yet.">
			{#if refusals.length > 0}
				<ul class="refusals">
					{#each refusals as refusal, index (`${refusal.field}-${index}`)}
						<li class={refusal.blocking ? 'blocking' : ''}>{refusal.message}</li>
					{/each}
				</ul>
			{/if}
			{#if serverRefusal !== null}
				<p class="refusal">{serverRefusal}</p>
			{/if}
			<div class="actions">
				<button class="cta" type="submit" disabled={!canCreate}>
					{creating ? 'Creating…' : 'Create draft'}
				</button>
				<a class="btn" href="/listings">Cancel</a>
				{#if priceOf(draft) !== null && draft.payload.length > 0}
					<span class="s">
						{draft.payload.length}
						{draft.payload.length === 1 ? 'file' : 'files'} · {draft.inventories.length}
						{draft.inventories.length === 1 ? 'marketplace' : 'marketplaces'}
					</span>
				{/if}
			</div>
		</Panel>
	</form>
</div>
