<script lang="ts">
	import { createQueries, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import {
		api,
		type CheckView,
		type UploadedView,
		type VocabularyView
	} from '$lib/api';
	import { payloadRefusal } from '$lib/authoring';
	import { AUTHORABLE_PLATFORMS, platformTitle } from '$lib/platforms';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import FacetPicker from '$lib/FacetPicker.svelte';
	import FormSection from '$lib/FormSection.svelte';
	import GradeGrid from '$lib/GradeGrid.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import StandardsPicker from '$lib/StandardsPicker.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import TabBar from '$lib/TabBar.svelte';
	import UploadField from '$lib/UploadField.svelte';
	import { queryKeys } from '$lib/query';
	import { createRefusal } from './refusal';
	import './resources.css';
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
		standardsHelp,
		suggestedAdditionalLicence,
		withOverride,
		GROUP_HELP,
		OVERRIDABLE,
		UNCOLLECTED_FIELDS,
		type Refusal,
		type TptDraft
	} from '$lib/tpt-form';
	import { loadCore } from '$lib/core';
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
	let tab = $state<string>('canonical');

	const form = $derived(vocabulary.data ?? null);
	const known = $derived(perPlatform.known);
	// Whether the compiled rules have arrived, and whether they failed to.
	//
	// Both are `$state` because `core()` is neither: it reads a module-level
	// `let` that the loader assigns once the wasm is instantiated, and nothing
	// about that assignment invalidates a `$derived` that called it. So
	// `refusals` was computed once at mount, while the module was still in
	// flight, and stood at "the form's rules are still loading" for as long as
	// the seller left the fields alone — with Create draft disabled behind it.
	// Typing one character recomputed it and the sentence vanished, which is
	// how this was found: the rules had been ready the whole time.
	//
	// The promise is already in flight from `$lib/tpt-form`'s own module scope;
	// this awaits the same one and records which way it went.
	let rulesReady = $state(false);
	let rulesFailed = $state(false);
	$effect(() => {
		void loadCore().then(
			() => {
				rulesReady = true;
			},
			() => {
				rulesFailed = true;
			}
		);
	});

	const RULES_UNREAD: Refusal = {
		group: 'name',
		control: null,
		message: 'The form’s own rules could not be loaded, so nothing can be submitted yet.'
	};

	const refusals = $derived.by(() => {
		// Read so this recomputes when the rules land; `refusalsOf` asks `core()`
		// for them and `core()` cannot say when it changed.
		void rulesReady;
		return rulesFailed ? [RULES_UNREAD] : refusalsOf(draft, form);
	});
	// Same reason as `refusals`: `advisoriesOf` reads the same non-reactive
	// `core()`, so without this it stays empty until a field is touched.
	const advisories = $derived.by(() => {
		void rulesReady;
		return advisoriesOf(draft, form);
	});
	const canCreate = $derived(submittable(refusals) && form !== null && !creating);

	/** Why Create cannot run, which the button tier requires of any disabled
	 *  control: the refusals themselves are listed above it, so this names the
	 *  class of thing rather than repeating one of them. */
	const blocking = $derived.by(() => {
		if (creating) {
			return 'The draft is being created.';
		}
		if (rulesFailed) {
			return 'The form’s own rules could not be loaded, so nothing can be created.';
		}
		if (vocabulary.isError) {
			return "The form's own vocabulary could not be read, so nothing can be created.";
		}
		if (form === null) {
			return "The form's own vocabulary has not been read yet.";
		}
		return canCreate ? undefined : 'Some groups above are still refusing.';
	});
	const selected = $derived(
		AUTHORABLE_PLATFORMS.filter((inventory) => draft.inventories.includes(inventory))
	);

	/** The rail: the listing, then one segment per marketplace it will reach.
	 *  A marketplace's count is how many of its values differ from the
	 *  listing's, which is what the seller would otherwise have to open each
	 *  tab to discover; the listing itself has nothing to count. */
	const tabs = $derived([
		{ id: 'canonical', label: 'This listing', count: null, hint: 'The values every marketplace takes unless one is changed.' },
		...selected.map((inventory) => ({
			id: inventory,
			label: platformTitle(inventory),
			count: OVERRIDABLE.filter((entry) => diverges(draft, inventory, entry.field)).length,
			hint: 'How many values here differ from the listing.'
		}))
	]);

	function set<K extends keyof TptDraft>(field: K, value: TptDraft[K]) {
		draft = { ...draft, [field]: value };
		// The server's answer described the draft as it was submitted. Editing any
		// field makes it history, so it stops being listed beside a Create button
		// that has re-enabled; the next submit asks again.
		serverCheck = null;
		serverRefusal = null;
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
			serverRefusal = createRefusal(failure);
		} finally {
			creating = false;
		}
	}

	function gigabytes(bytes: number): string {
		if (bytes >= 1024 ** 3) {
			return `${Math.round(bytes / 1024 ** 3)} GB`;
		}
		return `${Math.round(bytes / 1024 ** 2)} MB`;
	}
</script>

<div class="page resources-page">
	<PageHead
		icon="circle-plus"
		title="New resource"
		description="Author the resource once, on the same fields TPT asks for, and choose which marketplaces carry it."
	>
		{#snippet aside()}
			<Button href="/inventory">Cancel</Button>
		{/snippet}
	</PageHead>

	{#if vocabulary.isError}
		<Banner tone="bad" title="The form's own vocabulary could not be read">
			Without it this form cannot render its pickers or state their limits, so creating is held
			back. Reload to try again.
		</Banner>
	{/if}

	<!-- The destination rail, first, as the split editor has it: the canonical
	     form and then one tab per marketplace it will reach. The count on a
	     marketplace is how many of its values the seller has changed away from
	     the listing's own, which is the figure a dot used to stand for; the
	     listing itself carries no count, and the bar prints the bare label. -->
	<TabBar {tabs} bind:current={tab} />

	<form class="res-form" onsubmit={create}>
		{#if tab === 'canonical'}
			<FormSection group="name" {refusals}>
				<Field label="Title" id="draft-title" required>
					{#if form}
						<span class="res-count" class:over={draft.name.length > form.limits.title_max_utf16_units}>
							{draft.name.length} of {form.limits.title_max_utf16_units}
						</span>
					{/if}
					<input
						id="draft-title"
						type="text"
						required
						placeholder="Name your product"
						maxlength={form === null ? undefined : form.limits.title_max_utf16_units}
						value={draft.name}
						oninput={(event) => set('name', event.currentTarget.value)}
					/>
				</Field>
			</FormSection>

			<FormSection group="files" help={GROUP_HELP.files} {refusals}>
				<UploadField
					{uploaded}
					{keepWhole}
					onUploaded={takeUpload}
					onKeepWhole={(whole) => (keepWhole = whole)}
				/>
				{#if form}
					<p class="res-foot">
						Downloadable File up to {gigabytes(form.limits.product_file.max_size_bytes)}, Preview up
						to {gigabytes(form.limits.preview.max_size_bytes)}, Video Preview up to
						{gigabytes(form.limits.video_preview.max_size_bytes)}, each thumbnail up to
						{gigabytes(form.limits.thumbnail.max_size_bytes)}.
					</p>

					<fieldset class="res-choices res-stack">
						<legend>Thumbnails</legend>
						<p class="res-note">
							The images that front the listing. The four slots appear only under "Upload
							thumbnails now", exactly as they do on TPT.
						</p>
						<div class="res-choices">
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
							<div class="res-slots">
								{#each [0, 1, 2, 3] as slot (slot)}
									<div class="res-slot">
										<b>{slot === 0 ? 'Main Cover' : 'Thumbnail (Optional)'}</b>
										<span class="res-note">Select file or drag and drop</span>
										<span class="res-note">
											Up to {gigabytes(form.limits.thumbnail.max_size_bytes)}
										</span>
									</div>
								{/each}
							</div>
							<p class="res-foot">
								The four slots are rendered here so the layout matches TPT's. Attaching bytes to
								them needs a per-slot upload the byte endpoint does not offer yet, so nothing is
								collected and nothing is claimed.
							</p>
						{/if}
					</fieldset>
				{/if}
			</FormSection>

			<FormSection group="description" {refusals}>
				<Field label="Description" id="draft-description" required>
					{#if form}
						<span
							class="res-count"
							class:over={draft.description.length > form.limits.description_max_length}
						>
							{draft.description.length} of {form.limits.description_max_length}
						</span>
					{/if}
					<textarea
						id="draft-description"
						placeholder="Describe your product and how it can be helpful to another educator"
						value={draft.description}
						oninput={(event) => set('description', event.currentTarget.value)}
					></textarea>
				</Field>
				<p class="res-foot">
					Written as Markdown. TPT's wire is HTML and renders this into it; every Tes site carries
					the format you wrote in.
				</p>
			</FormSection>

			<FormSection group="price" help={GROUP_HELP.price} {refusals} {advisories}>
				<div class="res-choices">
					<label>
						<input
							type="checkbox"
							checked={draft.free}
							onchange={(event) => set('free', event.currentTarget.checked)}
						/>
						Free Resource
					</label>
					{#if form}
						<span class="res-note">
							Free resources should be {form.limits.free_resource_page_guidance} pages or fewer.
						</span>
					{/if}
				</div>

				{#if !draft.free}
					<div class="res-row">
						<Field label="Price" id="draft-price" required>
							<input
								id="draft-price"
								type="text"
								inputmode="decimal"
								placeholder="0.00"
								value={draft.price}
								oninput={(event) => set('price', event.currentTarget.value)}
							/>
						</Field>
						<!-- The percentage is TPT's own and is stated only when TPT has been
						     asked. A literal here read as a rule of theirs on a page whose
						     banner said their rules could not be read. -->
						<Field
							label="Multiple Licenses"
							id="draft-additional-licence"
							required
							hint={form === null
								? 'Yours to set. Whatever you put here is what every sync writes.'
								: `Pre-filled at ${form.limits.additional_licence_percentage}% of the price, and yours to change. Whatever you set here is what every sync writes.`}
						>
							<input
								id="draft-additional-licence"
								type="text"
								inputmode="decimal"
								placeholder="0.00"
								value={draft.additionalLicence}
								oninput={(event) => set('additionalLicence', event.currentTarget.value)}
							/>
						</Field>
						<Field label="Bundle Discount Price" id="draft-bundle-discount">
							<input
								id="draft-bundle-discount"
								type="text"
								inputmode="decimal"
								placeholder="0.00"
								value={draft.bundleDiscount}
								oninput={(event) => set('bundleDiscount', event.currentTarget.value)}
							/>
						</Field>
					</div>

					<Field
						label="Tax Code"
						id="draft-tax-code"
						required
						hint="Completion of this field is required in order for sales tax to be collected on this product. It is never chosen for you: designating it is yours under TPT's terms."
					>
						<select
							id="draft-tax-code"
							value={draft.taxCode ?? ''}
							onchange={(event) =>
								set('taxCode', event.currentTarget.value === '' ? null : event.currentTarget.value)}
						>
							<option value="">Select a tax code</option>
							{#each form?.tax_codes ?? [] as code (code.id)}
								<option value={code.id}>{code.label}</option>
							{/each}
						</select>
					</Field>
				{:else}
					<p class="res-foot">
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

					<Field
						label="Custom Category"
						id="draft-custom-category"
						hint="A custom category is any word or phrase you would like to use to group your own resources. These are your own shelves rather than a marketplace vocabulary, which is why they sit apart from the three pickers above."
					>
						<input
							id="draft-custom-category"
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
							<div class="res-chips" role="list">
								{#each draft.customCategories as shelf (shelf)}
									<span class="res-chip" role="listitem">
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
					</Field>
				{/if}

				{#if form}
					<div class="res-choices">
						<label>
							<input
								type="checkbox"
								checked={draft.appropriateForCountry}
								onchange={(event) => set('appropriateForCountry', event.currentTarget.checked)}
							/>
							{form.localisation.label ?? form.localisation.generic_label}
						</label>
					</div>
				{/if}
			</FormSection>

			<FormSection
				group="education_standards"
				help={standardsHelp(form?.standards_frameworks.length ?? 0)}
				{refusals}
			>
				{#if form}
					<StandardsPicker
						frameworks={form.standards_frameworks}
						chosen={draft.standards}
						onChange={(standards) => set('standards', standards)}
					/>
				{/if}
			</FormSection>

			<FormSection group="details" help={GROUP_HELP.details} {refusals}>
				<div class="res-row">
					<Field label="Teaching Duration" id="draft-teaching-duration">
						<select
							id="draft-teaching-duration"
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
					</Field>
					<Field label="Number of Pages or Slides" id="draft-pages">
						<input
							id="draft-pages"
							type="text"
							inputmode="numeric"
							placeholder="Total pages or slides"
							value={draft.pagesOrSlides}
							oninput={(event) => set('pagesOrSlides', event.currentTarget.value)}
						/>
					</Field>
					<Field
						label="Answer Key"
						id="draft-answer-key"
						hint="Ordered as the marketplace stores it rather than as its own menu shows it, which is not the same order and is where a mis-mapped answer key comes from."
					>
						<select
							id="draft-answer-key"
							value={draft.answerKey ?? ''}
							onchange={(event) =>
								set('answerKey', event.currentTarget.value === '' ? null : event.currentTarget.value)}
						>
							<option value="">N/A</option>
							{#each form?.answer_keys ?? [] as key (key.id)}
								<option value={key.id}>{key.label}</option>
							{/each}
						</select>
					</Field>
				</div>
			</FormSection>

			<FormSection group="copyright" {refusals}>
				{#if form}
					<p class="res-lede">{form.copyright.preamble}</p>
					<div class="res-picks" role="radiogroup" aria-label="Copyright">
						{#each form.copyright.options as option (option.id)}
							<label class="res-pick">
								<input
									type="radio"
									name="copyright"
									checked={draft.copyright === option.id}
									onchange={() => set('copyright', option.id)}
								/>
								<span class="res-pick-t">{option.label}</span>
							</label>
						{/each}
					</div>
					<p class="res-foot">
						Nothing is selected for you. TPT's own form arrives with the first of these ticked; a
						default here would make the attestation ours rather than yours, so the draft cannot be
						created until you choose.
					</p>
				{/if}
			</FormSection>

			<FormSection group="product_status" help={GROUP_HELP.product_status} {refusals}>
				<div class="res-choices" role="radiogroup" aria-label="Product status">
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

				<div class="res-group res-rail">
					<span class="res-group-label" id="rail-label">Marketplaces</span>
					<span class="res-note">
						The marketplaces this draft is created on. You can add another from the resource
						itself afterwards.
					</span>
					<div class="res-picks" role="group" aria-labelledby="rail-label">
						{#each AUTHORABLE_PLATFORMS as inventory (inventory)}
							{@const refusal = unselectable(inventory)}
							<label class="res-pick" class:off={refusal !== null}>
								<input
									type="checkbox"
									checked={draft.inventories.includes(inventory)}
									disabled={refusal !== null}
									onchange={(event) => togglePlatform(inventory, event.currentTarget.checked)}
								/>
								<span class="res-pick-t">{platformTitle(inventory)}</span>
								{#if refusal !== null}
									<span class="res-pick-why bad">{refusal}</span>
								{:else if known.get(inventory)?.authoring.payload_files === 'exactly_one'}
									<span class="res-pick-why">Takes exactly one file.</span>
								{:else}
									<span class="res-pick-why">Carries every payload file this product holds.</span>
								{/if}
							</label>
						{/each}
					</div>
				</div>
			</FormSection>
		{:else}
			{@const where = tab as InventoryId}
			{@const view = known.get(where)}
			{@const projection = projectionOf(draft, where, view ?? null)}
			<Panel
				title={platformTitle(where)}
				description="What this marketplace will carry. Values follow the listing unless you change one here; a changed value stays changed until you reset it."
			>

				{#each projection.rows.filter((row) => row.kind === 'field') as row (row.key)}
					{@const own = row.values[0]}
					{@const differs = row.decided_by?.by === 'listing_override'}
					<div class="res-group">
						<span class="res-group-label" id={`override-${row.key}`}>
							{row.label}
							{#if differs}<StatusPill label="differs" />{/if}
						</span>
						<input
							type="text"
							aria-labelledby={`override-${row.key}`}
							value={own}
							oninput={(event) =>
								(draft = withOverride(draft, where, row.key, event.currentTarget.value))}
						/>
						{#if differs}
							<div class="res-acts">
								<Button small onclick={() => (draft = applyToAll(draft, row.key, own))}>
									Update all
								</Button>
								<Button
									small
									onclick={() => (draft = withOverride(draft, where, row.key, null))}
								>
									Reset
								</Button>
								<span class="res-note">
									The listing still reads “{canonicalValue(draft, row.key)}”. Neither control
									fires on its own.
								</span>
							</div>
						{/if}
					</div>
				{/each}

				{#each projection.rows.filter((row) => row.axis !== undefined) as row (row.key)}
					{@const facts = row.axis}
					{#if facts}
						<div class="res-group">
							<span class="res-group-label">
								{row.label}
								{#if facts.delegable}
									<StatusPill label="best fit, once you opt in" />
								{:else}
									<StatusPill label="yours to decide" />
								{/if}
							</span>
							<div class="res-note">
								{#if facts.stated.length > 0}
									You chose {facts.stated.join(', ')}.
								{/if}
								It lands in this platform's “{facts.native}” field.
								{#if facts.cap !== null}
									It takes {facts.cap}.
								{/if}
								{#if row.loss}
									<span class="res-loss">{row.loss}</span>
								{/if}
							</div>
							<div class="res-note">
								{#if facts.delegable}
									Nothing has resolved this yet, and nothing here will: the mapping lives on the
									server and a value chosen in the browser would be one nobody recorded.
								{:else}
									This one is never computed for you. No opt-in admits a machine answer, on this
									form or anywhere else.
								{/if}
							</div>
						</div>
					{/if}
				{/each}

				{#if view}
					{#each view.absent_axes as axis (axis)}
						<p class="res-disclose">
							This platform has no {axis} field at all. A {axis} you state elsewhere does not reach
							it, and that loss is disclosed rather than silent.
						</p>
					{/each}
					{#if view.authoring.attestation}
						<p class="res-disclose">
							Its create carries the copyright declaration you chose on this listing, recorded
							against your connection.
						</p>
					{/if}
					{#if view.authoring.price_floor_minor_units !== undefined}
						<p class="res-disclose">
							It refuses a price below {view.authoring.price_floor_minor_units} minor units when the
							listing is written.
						</p>
					{/if}
				{/if}

				<p class="res-foot">
					Only the three fields above carry a per-marketplace value you can edit here. The axes
					below them are settled by the taxonomy relation on the server rather than on this form,
					so none of them shows a value yet and none offers an override; anything the relation
					cannot place is raised in Reconciliation. The standards and the details project the same
					way and carry no axis of their own.
				</p>
			</Panel>
		{/if}

		<Panel title="Create the draft" description="Nothing is sent to a marketplace yet.">

			{#if refusals.length > 0}
				<ul class="res-refusals">
					{#each refusals as refusal, index (`${refusal.group}-${index}`)}
						<li class="res-refusal">
							<a href="#group-{refusal.group}">{refusal.message}</a>
						</li>
					{/each}
				</ul>
			{/if}

			{#if serverCheck !== null && !serverCheck.submittable}
				<ul class="res-refusals">
					{#each serverCheck.refusals as refusal, index (`server-${index}`)}
						<li class="res-refusal">{refusal.message}</li>
					{/each}
				</ul>
			{/if}

			{#if serverRefusal !== null}
				<Banner tone="bad">{serverRefusal}</Banner>
			{/if}

			<p class="res-foot">
				Every group above is stored when the draft is created. One thing is not collected:
				{UNCOLLECTED_FIELDS.join('; ')}. It is stated here rather than implied, so nothing reads as
				saved that was not.
			</p>

			<div class="res-acts">
				<Button tier="primary" type="submit" disabled={!canCreate} reason={blocking}>
					{creating ? 'Creating…' : 'Create draft'}
				</Button>
				<Button href="/inventory">Cancel</Button>
				<span class="res-note">
					{draft.payload.length}
					{draft.payload.length === 1 ? 'file' : 'files'} · {draft.inventories.length}
					{draft.inventories.length === 1 ? 'marketplace' : 'marketplaces'}
					{#if divergentOn(draft, 'name').length + divergentOn(draft, 'description').length + divergentOn(draft, 'price').length > 0}
						· some marketplace values differ
					{/if}
				</span>
			</div>
		</Panel>
	</form>
</div>
