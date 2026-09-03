<script lang="ts">
	import { createQueries, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import {
		ApiFailure,
		api,
		type CheckView,
		type UploadedView,
		type VocabularyView
	} from '$lib/api';
	import { payloadRefusal, quotaSentence } from '$lib/authoring';
	import { AUTHORABLE_PLATFORMS, platformTitle } from '$lib/platforms';
	import FacetPicker from '$lib/FacetPicker.svelte';
	import FormSection from '$lib/FormSection.svelte';
	import GradeGrid from '$lib/GradeGrid.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import StandardsPicker from '$lib/StandardsPicker.svelte';
	import UploadField from '$lib/UploadField.svelte';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import {
		advisoriesOf,
		applyToAll,
		canonicalValue,
		capOf,
		createBodyOf,
		diverges,
		divergentOn,
		draftInputOf,
		emptyTptDraft,
		labelOf,
		projectionOf,
		refusalsOf,
		submittable,
		suggestedAdditionalLicence,
		withOverride,
		GROUP_HELP,
		OVERRIDABLE,
		UNCOLLECTED_FIELDS,
		type TptDraft
	} from '$lib/tpt-form';
	import type { InventoryId } from '$lib/generated/vocab';

	const queryClient = useQueryClient();

	// One read for the whole form: every controlled list its controls render,
	// served from the committed TPT capture. Cached forever — it is the
	// server's own data and changes only when the server does.
	const vocabulary = createQuery(() => ({
		queryKey: queryKeys.formVocabulary,
		queryFn: () => api.formVocabulary(),
		staleTime: Infinity
	}));

	// One read per marketplace, for the destination tabs: what each platform
	// does to the canonical values, which is a different question from what the
	// canonical form holds.
	const perPlatform = createQueries(() => ({
		queries: AUTHORABLE_PLATFORMS.map((inventory) => ({
			queryKey: queryKeys.vocabulary(inventory),
			queryFn: () => api.vocabulary(inventory),
			staleTime: Infinity
		})),
		combine: (results: { data?: VocabularyView }[]) => ({
			known: new Map(
				results.flatMap((result) =>
					result.data === undefined
						? []
						: [[result.data.inventory, result.data] as [InventoryId, VocabularyView]]
				)
			)
		})
	}));

	let draft = $state<TptDraft>(emptyTptDraft());
	let uploaded = $state<UploadedView | null>(null);
	let keepWhole = $state(false);
	let creating = $state(false);
	let serverCheck = $state<CheckView | null>(null);
	let serverRefusal = $state<string | null>(null);
	let tab = $state<InventoryId | 'canonical'>('canonical');

	const form = $derived(vocabulary.data ?? null);
	const known = $derived(perPlatform.known);
	const refusals = $derived(refusalsOf(draft, form));
	const advisories = $derived(advisoriesOf(draft, form));
	const canCreate = $derived(submittable(refusals) && form !== null && !creating);
	const selected = $derived(
		AUTHORABLE_PLATFORMS.filter((inventory) => draft.inventories.includes(inventory))
	);

	function set<K extends keyof TptDraft>(field: K, value: TptDraft[K]) {
		draft = { ...draft, [field]: value };
	}

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

	/** Why this platform cannot carry this product at all, or `null`.
	 *
	 *  A TPT create takes exactly one file, so a multi-file product is TES-only
	 *  and the tick box says so rather than letting a seller choose a platform
	 *  whose write is already refused. */
	function unselectable(inventory: InventoryId): string | null {
		const view = known.get(inventory);
		if (view === undefined || draft.payload.length === 0) {
			return null;
		}
		const refusal = payloadRefusal(view.authoring.payload_files, draft.payload.length);
		return refusal === null ? null : `This platform ${refusal}.`;
	}

	// Selecting a platform, then uploading a file it cannot carry, would leave
	// it ticked and refused. Dropping it here keeps the rail and the refusals
	// saying the same thing.
	$effect(() => {
		const kept = draft.inventories.filter((inventory) => unselectable(inventory) === null);
		if (kept.length !== draft.inventories.length) {
			draft = { ...draft, inventories: kept };
		}
	});

	// The Multiple Licenses pre-fill, seeded once and never afterwards: TPT's
	// help centre says the seller may choose any discount, so a figure they
	// typed is theirs and recomputing it would overwrite it (D6).
	$effect(() => {
		if (!draft.free && draft.additionalLicence === '' && draft.price !== '' && form !== null) {
			const seeded = suggestedAdditionalLicence(draft.price, form.limits.additional_licence_percentage);
			if (seeded !== '') {
				draft = { ...draft, additionalLicence: seeded };
			}
		}
	});

	async function create(event: SubmitEvent) {
		event.preventDefault();
		const body = createBodyOf(draft);
		if (body === null || !canCreate) {
			return;
		}
		creating = true;
		serverRefusal = null;
		try {
			// The server decides. The inline messages are a mirror so a seller
			// reads one as they type; this is what the model actually refuses,
			// and a client that drifted still gets the same answer.
			serverCheck = await api.checkDraft(draftInputOf(draft));
			if (!serverCheck.submittable) {
				return;
			}
			const created = await api.createProduct(body);
			await queryClient.invalidateQueries({ queryKey: queryKeys.products });
			await queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
			toast(
				'info',
				created.mappings.length === 1
					? 'Draft created on one marketplace. Publish when you are ready.'
					: `Draft created on ${created.mappings.length} marketplaces. Publish when you are ready.`
			);
			await goto(`/inventory/${created.product}`);
		} catch (failure) {
			serverRefusal = refusalOf(failure);
		} finally {
			creating = false;
		}
	}

	function refusalOf(failure: unknown): string {
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

	function gigabytes(bytes: number): string {
		if (bytes >= 1024 ** 3) {
			return `${Math.round(bytes / 1024 ** 3)} GB`;
		}
		return `${Math.round(bytes / 1024 ** 2)} MB`;
	}
</script>

<div class="page">
	<PageHead
		icon="✚"
		title="New item"
		description="Author the item once, on the same fields TPT asks for, and choose which marketplaces carry it."
	>
		{#snippet aside()}
			<a class="btn" href="/inventory">Cancel</a>
		{/snippet}
	</PageHead>

	{#if vocabulary.isError}
		<div class="attn">
			<div class="t">The form's own vocabulary could not be read</div>
			<p>
				Without it this form cannot render its pickers or state their limits, so creating is held
				back. Reload to try again.
			</p>
		</div>
	{/if}

	<!-- The destination rail, first, as the split editor has it: the
	     canonical form and then one tab per marketplace it will reach. -->
	<div class="tabs" role="tablist" aria-label="Destination">
		<button
			type="button"
			role="tab"
			class="tab"
			aria-selected={tab === 'canonical'}
			onclick={() => (tab = 'canonical')}
		>
			This listing
		</button>
		{#each selected as inventory (inventory)}
			<button
				type="button"
				role="tab"
				class="tab"
				aria-selected={tab === inventory}
				onclick={() => (tab = inventory)}
			>
				{platformTitle(inventory)}
				{#if OVERRIDABLE.some((entry) => diverges(draft, inventory, entry.field))}
					<span class="dot warn"></span>
				{/if}
			</button>
		{/each}
	</div>

	<form class="form wide" onsubmit={create}>
		{#if tab === 'canonical'}
			<FormSection group="name" {refusals}>
				<label class="field">
					Title <span class="req">Required</span>
					{#if form}
						<span class="counter {draft.name.length > form.limits.title_max_utf16_units ? 'over' : ''}">
							{draft.name.length} of {form.limits.title_max_utf16_units}
						</span>
					{/if}
					<input
						type="text"
						required
						placeholder="Name your product"
						maxlength={form?.limits.title_max_utf16_units ?? 80}
						value={draft.name}
						oninput={(event) => set('name', event.currentTarget.value)}
					/>
				</label>
			</FormSection>

			<FormSection group="files" help={GROUP_HELP.files} {refusals}>
				<UploadField
					{uploaded}
					{keepWhole}
					onUploaded={takeUpload}
					onKeepWhole={(whole) => (keepWhole = whole)}
				/>
				{#if form}
					<p class="foot-note">
						Downloadable File up to {gigabytes(form.limits.product_file.max_size_bytes)}, Preview up
						to {gigabytes(form.limits.preview.max_size_bytes)}, Video Preview up to
						{gigabytes(form.limits.video_preview.max_size_bytes)}, each thumbnail up to
						{gigabytes(form.limits.thumbnail.max_size_bytes)}.
					</p>

					<div class="field" style="margin-top: 14px">
						<span id="thumbs-label">Thumbnails</span>
						<span class="hint">
							The images that front the listing. The four slots appear only under "Upload
							thumbnails now", exactly as they do on TPT.
						</span>
						<div class="inline-choices" role="radiogroup" aria-labelledby="thumbs-label">
							{#each form.thumbnail_modes as mode (mode.id)}
								<label>
									<input
										type="radio"
										name="thumbnail-mode"
										checked={draft.thumbnailMode === mode.id}
										onchange={() => set('thumbnailMode', mode.id)}
									/>
									{mode.label}
								</label>
							{/each}
						</div>
						{#if draft.thumbnailMode === '2'}
							<div class="thumb-slots">
								{#each [0, 1, 2, 3] as slot (slot)}
									<div class="thumb-slot">
										<b>{slot === 0 ? 'Main Cover' : 'Thumbnail (Optional)'}</b>
										<span class="s">Select file or drag and drop</span>
										<span class="s">Up to {gigabytes(form.limits.thumbnail.max_size_bytes)}</span>
									</div>
								{/each}
							</div>
							<p class="foot-note">
								The four slots are rendered here so the layout matches TPT's. Attaching bytes to
								them needs a per-slot upload the byte endpoint does not offer yet, so nothing is
								collected and nothing is claimed.
							</p>
						{/if}
					</div>
				{/if}
			</FormSection>

			<FormSection group="description" {refusals}>
				<label class="field">
					Description <span class="req">Required</span>
					{#if form}
						<span class="counter {draft.description.length > form.limits.description_max_length ? 'over' : ''}">
							{draft.description.length} of {form.limits.description_max_length}
						</span>
					{/if}
					<textarea
						placeholder="Describe your product and how it can be helpful to another educator"
						value={draft.description}
						oninput={(event) => set('description', event.currentTarget.value)}
					></textarea>
				</label>
				<p class="foot-note">
					Written as Markdown. TPT's wire is HTML and renders this into it; every Tes site carries
					the format you wrote in.
				</p>
			</FormSection>

			<FormSection group="price" help={GROUP_HELP.price} {refusals} {advisories}>
				<div class="inline-choices">
					<label>
						<input
							type="checkbox"
							checked={draft.free}
							onchange={(event) => set('free', event.currentTarget.checked)}
						/>
						Free Resource
					</label>
					{#if form}
						<span class="hint">
							Free resources should be {form.limits.free_resource_page_guidance} pages or fewer.
						</span>
					{/if}
				</div>

				{#if !draft.free}
					<div class="field-row" style="margin-top: 12px">
						<label class="field">
							Price <span class="req">Required</span>
							<input
								type="text"
								inputmode="decimal"
								placeholder="0.00"
								value={draft.price}
								oninput={(event) => set('price', event.currentTarget.value)}
							/>
						</label>
						<label class="field">
							Multiple Licenses <span class="req">Required</span>
							<span class="hint">
								Pre-filled at {form?.limits.additional_licence_percentage ?? 90}% of the price, and
								yours to change. Whatever you set here is what every sync writes.
							</span>
							<input
								type="text"
								inputmode="decimal"
								placeholder="0.00"
								value={draft.additionalLicence}
								oninput={(event) => set('additionalLicence', event.currentTarget.value)}
							/>
						</label>
						<label class="field">
							Bundle Discount Price
							<input
								type="text"
								inputmode="decimal"
								placeholder="0.00"
								value={draft.bundleDiscount}
								oninput={(event) => set('bundleDiscount', event.currentTarget.value)}
							/>
						</label>
					</div>

					<label class="field" style="margin-top: 12px">
						Tax Code <span class="req">Required</span>
						<span class="hint">
							Completion of this field is required in order for sales tax to be collected on this
							product. It is never chosen for you: designating it is yours under TPT's terms.
						</span>
						<select
							value={draft.taxCode ?? ''}
							onchange={(event) =>
								set('taxCode', event.currentTarget.value === '' ? null : event.currentTarget.value)}
						>
							<option value="">Select a tax code</option>
							{#each form?.tax_codes ?? [] as code (code.id)}
								<option value={code.id}>{code.label}</option>
							{/each}
						</select>
					</label>
				{:else}
					<p class="foot-note">
						A free resource collects no price and needs no tax code, which is what TPT's own form
						does when Free Resource is ticked.
					</p>
				{/if}
			</FormSection>

			<FormSection group="categories" help={GROUP_HELP.categories} {refusals}>
				{#if form}
					<GradeGrid vocabulary={form} chosen={draft.grades} onChange={(grades) => set('grades', grades)} />

					<FacetPicker
						label="Subject Area"
						required
						placeholder="Select up to three subject areas"
						facets={form.subject_areas}
						chosen={draft.subjectAreas}
						cap={capOf(form.caps, 'subjectAreas')}
						hint="TPT's form states three and a create it accepted posted four, so no limit is enforced here."
						onChange={(values) => set('subjectAreas', values)}
					/>

					<FacetPicker
						label="Tag (Theme, Audience, Language)"
						required
						placeholder="Select up to six tags"
						facets={form.tags}
						chosen={draft.tags}
						cap={capOf(form.caps, 'tags')}
						onChange={(values) => set('tags', values)}
					/>

					<FacetPicker
						label="Format"
						placeholder="Select up to three formats"
						facets={form.formats}
						chosen={draft.formats}
						cap={capOf(form.caps, 'formats')}
						onChange={(values) => set('formats', values)}
					/>

					<div class="field custom-shelf">
						<span>Custom Category</span>
						<span class="hint">
							A custom category is any word or phrase you'd like to use to categorize your
							items. These are your own shelves rather than a marketplace vocabulary, which is
							why they sit apart from the three pickers above.
						</span>
						<input
							type="text"
							placeholder="Add a shelf and press Enter"
							onkeydown={(event) => {
								if (event.key === 'Enter') {
									event.preventDefault();
									const typed = event.currentTarget.value.trim();
									if (typed.length > 0 && !draft.customCategories.includes(typed)) {
										set('customCategories', [...draft.customCategories, typed]);
									}
									event.currentTarget.value = '';
								}
							}}
						/>
						{#if draft.customCategories.length > 0}
							<div class="chips" role="list">
								{#each draft.customCategories as shelf (shelf)}
									<span class="chip-pick" role="listitem">
										{shelf}
										<button
											type="button"
											aria-label="Remove {shelf}"
											onclick={() =>
												set(
													'customCategories',
													draft.customCategories.filter((held) => held !== shelf)
												)}
										>
											×
										</button>
									</span>
								{/each}
							</div>
						{/if}
					</div>
				{/if}
			</FormSection>

			<FormSection group="education_standards" help={GROUP_HELP.education_standards} {refusals}>
				{#if form}
					<StandardsPicker
						frameworks={form.standards_frameworks}
						chosen={draft.standards}
						onChange={(standards) => set('standards', standards)}
					/>
				{/if}
			</FormSection>

			<FormSection group="details" help={GROUP_HELP.details} {refusals}>
				<div class="field-row">
					<label class="field">
						Teaching Duration
						<select
							value={draft.teachingDuration ?? ''}
							onchange={(event) =>
								set(
									'teachingDuration',
									event.currentTarget.value === '' ? null : event.currentTarget.value
								)}
						>
							<option value="">N/A</option>
							{#each form?.teaching_durations ?? [] as duration (duration.id)}
								<option value={duration.id}>{duration.label}</option>
							{/each}
						</select>
					</label>
					<label class="field">
						Number of Pages or Slides
						<input
							type="text"
							inputmode="numeric"
							placeholder="Total pages or slides"
							value={draft.pagesOrSlides}
							oninput={(event) => set('pagesOrSlides', event.currentTarget.value)}
						/>
					</label>
					<label class="field">
						Answer Key
						<span class="hint">
							Ordered as the marketplace stores it rather than as its own menu shows it, which is
							not the same order and is where a mis-mapped answer key comes from.
						</span>
						<select
							value={draft.answerKey ?? ''}
							onchange={(event) =>
								set('answerKey', event.currentTarget.value === '' ? null : event.currentTarget.value)}
						>
							<option value="">N/A</option>
							{#each form?.answer_keys ?? [] as key (key.id)}
								<option value={key.id}>{key.label}</option>
							{/each}
						</select>
					</label>
				</div>
			</FormSection>

			<FormSection group="copyright" {refusals}>
				{#if form}
					<p class="preamble">{form.copyright.preamble}</p>
					<div role="radiogroup" aria-label="Copyright">
						{#each form.copyright.options as option (option.id)}
							<label class="choice">
								<input
									type="radio"
									name="copyright"
									checked={draft.copyright === option.id}
									onchange={() => set('copyright', option.id)}
								/>
								<span class="t">{option.label}</span>
							</label>
						{/each}
					</div>
					<p class="foot-note">
						Nothing is selected for you. TPT's own form arrives with the first of these ticked; a
						default here would make the attestation ours rather than yours, so the draft cannot be
						created until you choose.
					</p>
				{/if}
			</FormSection>

			<FormSection group="product_status" help={GROUP_HELP.product_status} {refusals}>
				<div class="inline-choices" role="radiogroup" aria-label="Product status">
					{#each form?.statuses ?? [] as status (status.id)}
						<label>
							<input
								type="radio"
								name="status"
								checked={draft.status === status.id}
								onchange={() => set('status', status.id)}
							/>
							{status.label}
						</label>
					{/each}
				</div>

				<div class="field" style="margin-top: 16px">
					<span id="rail-label">Marketplaces</span>
					<span class="hint">
						Chosen once, here. There is no way to add a platform to a listing afterwards.
					</span>
					<div role="group" aria-labelledby="rail-label">
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
					</div>
				</div>
			</FormSection>
		{:else}
			{@const view = known.get(tab)}
			<section class="panel">
				<div class="head-row">
					<div>
						<h2>{platformTitle(tab)}</h2>
						<div class="desc">
							What this marketplace will carry. Values follow the listing unless you change one
							here; a changed value stays changed until you reset it.
						</div>
					</div>
				</div>

				{#each projectionOf(draft, tab).rows as row (row.key)}
					{@const own = row.values[0]}
					{@const differs = row.decided_by.by === 'listing_override'}
					<div class="field override">
						<span>
							{row.label}
							{#if differs}<span class="pill mut">differs</span>{/if}
						</span>
						<input
							type="text"
							value={own}
							oninput={(event) =>
								(draft = withOverride(draft, tab as InventoryId, row.key, event.currentTarget.value))}
						/>
						{#if differs}
							<div class="inline-choices">
								<button
									class="btn small"
									type="button"
									onclick={() => (draft = applyToAll(draft, row.key, own))}
								>
									Update all
								</button>
								<button
									class="btn small"
									type="button"
									onclick={() => (draft = withOverride(draft, tab as InventoryId, row.key, null))}
								>
									Reset
								</button>
								<span class="hint">
									The listing still reads “{canonicalValue(draft, row.key)}”. Neither control
									fires on its own.
								</span>
							</div>
						{/if}
					</div>
				{/each}

				{#if view}
					{#each view.absent_axes as axis (axis)}
						<p class="disclosure">
							This platform has no {axis} field at all. A {axis} you state elsewhere does not reach
							it, and that loss is disclosed rather than silent.
						</p>
					{/each}
					{#if view.authoring.attestation}
						<p class="disclosure">
							Its create carries the copyright declaration you chose on this listing, recorded
							against your connection.
						</p>
					{/if}
					{#if view.authoring.price_floor_minor_units !== undefined}
						<p class="disclosure">
							It refuses a price below {view.authoring.price_floor_minor_units} minor units when the
							listing is written.
						</p>
					{/if}
				{/if}

				<p class="foot-note">
					Only these three fields carry a per-marketplace value today. The rest of the listing —
					the categories, the standards and the details — projects through the taxonomy relation,
					and anything a platform cannot place is raised in Reconciliation rather than edited here.
				</p>
			</section>
		{/if}

		<section class="panel">
			<div class="head-row">
				<div>
					<h2>Create the draft</h2>
					<div class="desc">Nothing is sent to a marketplace yet.</div>
				</div>
			</div>

			{#if refusals.length > 0}
				<ul class="refusals">
					{#each refusals as refusal, index (`${refusal.group}-${index}`)}
						<li class="blocking">
							<a href="#group-{refusal.group}">{refusal.message}</a>
						</li>
					{/each}
				</ul>
			{/if}

			{#if serverCheck !== null && !serverCheck.submittable}
				<ul class="refusals">
					{#each serverCheck.refusals as refusal, index (`server-${index}`)}
						<li class="blocking">{refusal.message}</li>
					{/each}
				</ul>
			{/if}

			{#if serverRefusal !== null}
				<p class="refusal">{serverRefusal}</p>
			{/if}

			<p class="foot-note">
				Every group above is stored when the draft is created. One thing is not collected:
				{UNCOLLECTED_FIELDS.join('; ')}. It is stated here rather than implied, so nothing reads as
				saved that was not.
			</p>

			<div class="actions">
				<button class="cta" type="submit" disabled={!canCreate}>
					{creating ? 'Creating…' : 'Create draft'}
				</button>
				<a class="btn" href="/inventory">Cancel</a>
				<span class="s">
					{draft.payload.length}
					{draft.payload.length === 1 ? 'file' : 'files'} · {draft.inventories.length}
					{draft.inventories.length === 1 ? 'marketplace' : 'marketplaces'}
					{#if divergentOn(draft, 'name').length + divergentOn(draft, 'description').length + divergentOn(draft, 'price').length > 0}
						· some marketplace values differ
					{/if}
				</span>
			</div>
		</section>
	</form>
</div>
