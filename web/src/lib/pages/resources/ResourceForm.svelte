<script lang="ts">
	import { createQueries, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import {
		ApiFailure,
		api,
		type CheckView,
		type UploadedView,
		type VocabularyView
	} from '$lib/api';
	import {
		fieldWords,
		licenceGated,
		licenceOptions,
		payloadRefusal,
		requiredFields
	} from '$lib/authoring';
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
	import { refreshAfterCreate } from './after-create';
	import FilesPanel from './FilesPanel.svelte';
	import MarketplacePicker from './MarketplacePicker.svelte';
	import ThumbnailSlots from './ThumbnailSlots.svelte';
	import { createRefusal, sentenceFor } from './refusal';
	import './resources.css';
	import { toast } from '$lib/toast';
	import {
		advisoriesOf,
		applyToAll,
		canonicalValue,
		capOf,
		createBodyOf,
		createdToast,
		diverges,
		divergentOn,
		draftInputOf,
		draftOf,
		emptySlots,
		emptyTptDraft,
		labelOf,
		needsFileBeforeMarketplace,
		patchBodyOf,
		projectionOf,
		refusalsOf,
		shouldLandOnCreated,
		slotsFrom,
		slotsSettling,
		submittable,
		thumbnailHashes,
		thumbnailRefusal,
		standardsHelp,
		suggestedAdditionalLicence,
		withOverride,
		GROUP_HELP,
		OVERRIDABLE,
		UNCOLLECTED_FIELDS,
		type FormMode,
		type Refusal,
		type ThumbnailSlot,
		type TptDraft
	} from '$lib/tpt-form';
	import { loadCore } from '$lib/core';
	import type { InventoryId } from '$lib/generated/vocab';

	// One component, two modes. The create form and the edit form render the
	// same fields because they are the same model: TPT's own create and edit
	// post near-identical bodies, and the edit page's six-field form left
	// seventeen sidecar fields a seller could set once and never change.
	let { mode = { kind: 'create' } as FormMode }: { mode?: FormMode } = $props();

	const editing = $derived(mode.kind === 'edit' ? mode : null);
	// Read-only where a published listing makes the edit unattemptable. Every
	// control takes it, so the fields are shown as stored rather than hidden.
	const locked = $derived((editing?.blockedBy.length ?? 0) > 0);

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
	// The marketplace being added in edit mode, so a second tick cannot start a
	// second add and the rail says which one is in flight.
	let adding = $state<InventoryId | null>(null);
	let serverCheck = $state<CheckView | null>(null);
	let serverRefusal = $state<string | null>(null);
	let tab = $state<string>('canonical');
	// Opened by the act of choosing a marketplace with no file, and by pressing
	// Create in that state. Not by an effect over the condition: an effect would
	// reopen it under a seller who had read it and closed it.
	let fileFirst = $state(false);
	let fileFirstDialog = $state<HTMLDialogElement | null>(null);
	// The four TPT slots, each holding the seller's own bytes as the browser can
	// draw them and, once the upload answers, the handle the create carries.
	let slots = $state<ThumbnailSlot[]>(emptySlots());

	// Seeded once per resource rather than mirrored: a refetch arriving while
	// the seller is typing must not overwrite what they typed, and a move from
	// one resource to the next must not leave the first one's fields in the
	// form — SvelteKit keeps one component across a change of `[id]`, so
	// without the second guard a save would write this resource's title onto
	// that one.
	let seededFor = $state<string | null>(null);
	$effect(() => {
		const stored = editing?.product;
		if (stored !== undefined && seededFor !== stored.id) {
			draft = draftOf(stored, editing?.mapped ?? []);
			slots = slotsFrom(stored.tpt_base?.thumbnail_hashes ?? []);
			seededFor = stored.id;
		}
	});

	/** One slot's chosen file: drawn at once from the browser's own object URL,
	 *  then uploaded, and only a slot the upload answered for reaches the wire.
	 *
	 *  The picture appears before the upload finishes on purpose — it is the
	 *  seller's own file and the browser can already draw it — and the slot says
	 *  "uploading" until the handle lands, so nothing claims to be saved that is
	 *  not yet. `POST /{version}/uploads` takes one file and answers with its own
	 *  handle, so which slot a file belongs to is decided by which request was
	 *  made rather than by anything the wire carries. */
	async function takeThumbnail(index: number, file: File) {
		// Refused before a byte is sent where the browser can already tell. The
		// slot's `accept` is a picker hint and does not survive a drag-and-drop.
		const unusable =
			form === null
				? null
				: thumbnailRefusal(file.type, file.size, form.limits.thumbnail.max_size_bytes);
		if (unusable !== null) {
			slots[index] = { local: null, handle: null, sending: false, refusal: unusable };
			return;
		}
		release(slots[index].local);
		const local = URL.createObjectURL(file);
		slots[index] = { local, handle: null, sending: true, refusal: null };
		try {
			// Kept whole: a thumbnail is one image and exploding it would answer
			// with a list this slot has no way to name.
			const landed = await api.upload(file, 'keep_whole');
			const handle = landed.payload[0]?.hash ?? null;
			slots[index] = {
				local,
				handle,
				sending: false,
				refusal: handle === null ? 'That file was stored with no handle to attach.' : null
			};
		} catch (failure) {
			slots[index] = {
				local,
				handle: null,
				sending: false,
				refusal: createRefusal(failure)
			};
		}
		draft = { ...draft, thumbnails: thumbnailHashes(slots) };
	}

	/** Only an object URL is ours to revoke. A slot seeded from a saved listing
	 *  holds the blob route's own address, and revoking that would be revoking
	 *  a path rather than a handle the browser minted. */
	function release(held: string | null) {
		if (held !== null && held.startsWith('blob:')) {
			URL.revokeObjectURL(held);
		}
	}

	function clearThumbnail(index: number) {
		release(slots[index].local);
		slots[index] = { local: null, handle: null, sending: false, refusal: null };
		draft = { ...draft, thumbnails: thumbnailHashes(slots) };
	}

	// Guarded on the element's own state, as the console's other dialogs are:
	// `showModal` on a dialog that is already modal throws.
	$effect(() => {
		if (fileFirst && fileFirstDialog !== null && !fileFirstDialog.open) {
			fileFirstDialog.showModal();
		} else if (!fileFirst) {
			fileFirstDialog?.close();
		}
	});

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
		return rulesFailed ? [RULES_UNREAD] : refusalsOf(draft, form, known);
	});
	// Same reason as `refusals`: `advisoriesOf` reads the same non-reactive
	// `core()`, so without this it stays empty until a field is touched.
	const advisories = $derived.by(() => {
		void rulesReady;
		return advisoriesOf(draft, form);
	});
	const canCreate = $derived(
		submittable(refusals) && form !== null && !creating && !slotsSettling(slots) && !locked
	);

	/** Why Create cannot run, which the button tier requires of any disabled
	 *  control: the refusals themselves are listed above it, so this names the
	 *  class of thing rather than repeating one of them. */
	const blocking = $derived.by(() => {
		if (locked) {
			return 'A published listing cannot be edited through us.';
		}
		if (creating) {
			return editing === null ? 'The draft is being created.' : 'The change is being saved.';
		}
		if (slotsSettling(slots)) {
			return 'A thumbnail is still uploading.';
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
		serverRefusal = null;
		draft = {
			...draft,
			payload: result?.payload ?? [],
			cover: result?.cover ?? null,
			previews: result?.previews ?? []
		};
		// The mirror image of ticking a marketplace with no file, and it opens
		// the same dialog: a seller who takes back the file under a listing
		// already pointed at a marketplace learns here rather than at Create.
		if (needsFileBeforeMarketplace(draft)) {
			fileFirst = true;
		}
	}

	/** Where the thumbnail this listing already has can be fetched, or `null`.
	 *
	 *  Two routes, because the two modes address the same picture differently:
	 *  a create has no product to name yet and reads the cover by the handle
	 *  its own upload answered with, and a saved resource reads its own cover
	 *  route. Neither is the seller's chosen bytes: the cover is drawn on the
	 *  server at the listing's own size, so drawing the local file here would
	 *  show something other than what buyers get. */
	const coverUrl = $derived.by(() => {
		if (editing !== null) {
			return editing.product.files.some((file) => file.role === 'cover')
				? `/v1/products/${editing.product.id}/cover`
				: null;
		}
		return uploaded === null ? null : `/v1/uploads/${uploaded.cover.hash}`;
	});

	/** Why this marketplace is ticked and cannot be unticked, or `null`.
	 *
	 *  Edit mode is add-only. `POST /{version}/products/{product}/mappings`
	 *  adds one and no route removes one, so a mapped marketplace renders
	 *  ticked and disabled with the reason said rather than offering a control
	 *  that would silently do nothing. Unlisting stays with the delete flow. */
	function heldBy(inventory: InventoryId): string | null {
		if (editing === null || !editing.mapped.includes(inventory)) {
			return null;
		}
		return 'Already listed here. Removing a marketplace is done from Delete, not from this form.';
	}

	function togglePlatform(inventory: InventoryId, on: boolean) {
		if (heldBy(inventory) !== null) {
			return;
		}
		if (editing !== null) {
			if (on) {
				void addMarketplace(inventory);
			}
			return;
		}
		draft = {
			...draft,
			inventories: on
				? [...draft.inventories, inventory]
				: draft.inventories.filter((held) => held !== inventory)
		};
		if (needsFileBeforeMarketplace(draft)) {
			fileFirst = true;
		}
	}

	/** Adds a marketplace to a resource that already exists. A catalogue write
	 *  that contacts nobody; the send stays the seller's own choice on the
	 *  resource page's publish control.
	 *
	 *  D32 is checked here before the request, so a seller with no file reads
	 *  the dialog rather than the server's refusal — the server refuses it too,
	 *  which is what makes this a mirror rather than the rule itself. */
	async function addMarketplace(inventory: InventoryId) {
		if (editing === null || adding !== null) {
			return;
		}
		if (draft.payload.length === 0) {
			fileFirst = true;
			return;
		}
		adding = inventory;
		serverRefusal = null;
		try {
			await api.addMapping(editing.product.id, inventory);
			await queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
			toast('info', `${platformTitle(inventory)} added. Send it when you are ready.`);
		} catch (failure) {
			serverRefusal =
				failure instanceof ApiFailure
					? sentenceFor(failure, `${platformTitle(inventory)} was not added.`)
					: `${platformTitle(inventory)} was not added.`;
		} finally {
			adding = null;
		}
	}

	/** Why this marketplace cannot carry this listing at all, or `null`.
	 *
	 *  Two reasons, and both are said beside the tick box rather than after the
	 *  submit, because a seller who reads them there never reaches the refusal.
	 *
	 *  A TPT create takes exactly one file, so a multi-file listing is Tes-only.
	 *  And a marketplace declaring a required field this form has no control for
	 *  cannot be created here at all, because `required_fields_answered` on the
	 *  server refuses a create that does not answer one. The licence is not that
	 *  case any more — the form asks for it below and the create carries it — so
	 *  it is excluded by name rather than by the list happening to be empty: a
	 *  field the registry adds tomorrow must still stop the tick rather than
	 *  reach the seller as a refusal after the submit. */
	function unselectable(inventory: InventoryId): string | null {
		const view = known.get(inventory);
		if (view === undefined) {
			return null;
		}
		const unanswerable = requiredFields(view).filter((field) => field !== 'licence');
		if (unanswerable.length > 0) {
			const words = unanswerable.map((field) => fieldWords(field)).join(' and ');
			return `${platformTitle(inventory)} needs ${words}, which this form does not ask for yet.`;
		}
		if (draft.payload.length === 0) {
			return null;
		}
		const refusal = payloadRefusal(view.authoring.payload_files, draft.payload.length);
		return refusal === null ? null : `${platformTitle(inventory)} ${refusal}.`;
	}

	/** The marketplaces this listing reaches that gate a licence, and the values
	 *  they offer under the pricing branch this listing is on. The Tes gate
	 *  refuses a Creative Commons licence with a price and refuses `TES-PAID`
	 *  without one, so ticking Free changes the list rather than only the price. */
	const licensing = $derived(licenceGated(draft.inventories, known));
	const licences = $derived(
		licenceOptions(known.get(licensing[0] ?? 'Tpt'), draft.free ? 'free' : 'paid')
	);

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

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		// Said as a dialog rather than as one more line in the list at the foot,
		// because it is the one refusal that is about an action the seller just
		// took rather than about a field they have not reached yet.
		if (needsFileBeforeMarketplace(draft)) {
			fileFirst = true;
			return;
		}
		if (!canCreate) {
			return;
		}
		await (editing === null ? create() : save(editing.product.id));
	}

	async function create() {
		const body = createBodyOf(draft, known);
		if (body === null) {
			return;
		}
		creating = true;
		serverRefusal = null;
		// Where the seller was when they pressed Create, so a create that
		// outlives their presence on this page does not drag them back to it.
		const submittedFrom = page.url.pathname;
		try {
			// The server decides. The inline messages are a mirror so a seller
			// reads one as they type; this is what the model actually refuses,
			// and a client that drifted still gets the same answer.
			serverCheck = await api.checkDraft(draftInputOf(draft));
			if (!serverCheck.submittable) {
				return;
			}
			const created = await api.createProduct(body);
			await refreshAfterCreate(queryClient);
			toast('info', createdToast(created.mappings.length));
			if (shouldLandOnCreated(submittedFrom, page.url.pathname)) {
				await goto(`/resources/${created.product}`);
			}
		} catch (failure) {
			serverRefusal = createRefusal(failure);
		} finally {
			creating = false;
		}
	}

	/** The same fields, sent as a replace of what is stored.
	 *
	 *  No `checkDraft` pre-flight, unlike the create: `PATCH` answers the
	 *  model's own rules itself, against the product's stored payload and its
	 *  stored mappings. That is the one check a browser cannot make here, and
	 *  the reason the stub it replaced was worth changing, so the server's
	 *  refusal is read back rather than anticipated. */
	async function save(product: string) {
		const body = patchBodyOf(draft, known);
		if (body === null) {
			return;
		}
		creating = true;
		serverRefusal = null;
		try {
			const patched = await api.patchProduct(product, body);
			await queryClient.invalidateQueries({ queryKey: queryKeys.product(product) });
			await queryClient.invalidateQueries({ queryKey: queryKeys.products });
			toast(
				'info',
				patched.reaches.length === 0
					? 'Saved. This listing is on no marketplace yet.'
					: `Saved. Reaches ${patched.reaches.map(platformTitle).join(', ')} on the next send.`
			);
		} catch (failure) {
			serverRefusal = editRefusalOf(failure);
		} finally {
			creating = false;
		}
	}

	function editRefusalOf(failure: unknown): string {
		if (!(failure instanceof ApiFailure)) {
			return 'The edit was not saved.';
		}
		if (failure.code() === 'uncaptured_transition') {
			return 'This listing is live on a platform whose edit-published transition we have not captured, so the edit cannot be attempted.';
		}
		return sentenceFor(failure, 'The edit was not saved.');
	}

	function gigabytes(bytes: number): string {
		if (bytes >= 1024 ** 3) {
			return `${Math.round(bytes / 1024 ** 3)} GB`;
		}
		return `${Math.round(bytes / 1024 ** 2)} MB`;
	}
</script>

<div class={editing === null ? 'page resources-page' : 'resources-page'}>
	{#if editing === null}
		<PageHead
			icon="circle-plus"
			title="New resource"
			description="Fill this in once. Keep it here as a draft, or choose the marketplaces that should carry it."
		>
			{#snippet aside()}
				<Button href="/resources">Cancel</Button>
			{/snippet}
		</PageHead>
	{/if}

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

	<form class="res-form" onsubmit={submit}>
		<!-- One `disabled` for the whole form rather than one per control. A
		     published listing on a platform whose edit transition we have not
		     captured cannot be edited through us, and the server refuses the
		     request, so every field is shown as stored and none of them takes an
		     edit. A `fieldset` disables its descendants by the user agent's own
		     rule, which is why this is the wrapper rather than a class. -->
		<fieldset class="res-lock" disabled={locked}>
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
				{#if editing === null}
					<UploadField
						{uploaded}
						{keepWhole}
						onUploaded={takeUpload}
						onKeepWhole={(whole) => (keepWhole = whole)}
						onCleared={() => takeUpload(null)}
					/>
				{:else}
					<!-- A saved resource's files are a sub-resource with three routes of
					     their own, and this panel is what drives them. Not the upload
					     control: before a product exists there is nothing for those
					     routes to address, and once one does an upload here would be a
					     second way to change the same thing. -->
					<FilesPanel
						product={editing.product.id}
						files={editing.product.files}
						inventories={editing.mapped}
					/>
				{/if}
				{#if form}
					<p class="res-foot">
						Downloadable File up to {gigabytes(form.limits.product_file.max_size_bytes)}, Preview up
						to {gigabytes(form.limits.preview.max_size_bytes)}, Video Preview up to
						{gigabytes(form.limits.video_preview.max_size_bytes)}, each thumbnail up to
						{gigabytes(form.limits.thumbnail.max_size_bytes)}.
					</p>

					<ThumbnailSlots
						{form}
						mode={draft.thumbnailMode}
						{slots}
						{coverUrl}
						{gigabytes}
						onMode={(id) => set('thumbnailMode', id)}
						onPick={(index, file) => void takeThumbnail(index, file)}
						onClear={clearThumbnail}
					/>
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
					Write it as plain text. Every marketplace gets it in the form you wrote it in.
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
							<!-- Three states, because the sidecar holds three. A product
							     that was never asked draws the box indeterminate rather
							     than unticked: the two look different, and reading them
							     the same way is what would let a seller's first save turn
							     "not stated" into "explicitly no" without their touching
							     the control. Ticking it either way is an answer, and the
							     third state cannot be returned to from here. -->
							<input
								type="checkbox"
								checked={draft.appropriateForCountry === true}
								indeterminate={draft.appropriateForCountry === null}
								onchange={(event) => set('appropriateForCountry', event.currentTarget.checked)}
							/>
							{form.localisation.label ?? form.localisation.generic_label}
						</label>
						{#if draft.appropriateForCountry === null}
							<span class="res-note">
								Not answered yet. It stays unanswered until you tick or untick it.
							</span>
						{/if}
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

				<MarketplacePicker
					chosen={draft.inventories}
					{known}
					refusalOf={unselectable}
					heldOf={heldBy}
					{licensing}
					{licences}
					licence={draft.licence}
					free={draft.free}
					note={editing === null
						? 'Where this listing goes. Choose none to keep it here as a draft and decide later; you can add a marketplace from the resource itself at any time.'
						: 'Where this listing goes. Ticking one adds it now; nothing is sent until you send it. A marketplace already carrying this listing cannot be removed here.'}
					onToggle={togglePlatform}
					onLicence={(value) => set('licence', value)}
				/>
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

		<Panel
			title={editing === null ? 'Create the draft' : 'Save the changes'}
			description={editing === null
				? 'Nothing is sent to a marketplace yet.'
				: 'Saving writes your catalogue. It reaches a marketplace on the next send.'}
		>
			{#if editing !== null && editing.blockedBy.length > 0}
				<Banner tone="warn" title="This resource is live and cannot be edited through us">
					{editing.blockedBy.map(platformTitle).join(', ')} has a published listing, and neither
					editing a published listing nor taking one back to draft is a transition we have
					captured there. The fields above are shown as stored and the edit is held back rather
					than sent and refused.
				</Banner>
			{:else if draft.inventories.length === 0}
				<p class="res-note">
					{editing === null
						? 'This will be saved here as a draft. Nobody else sees it, and no file is needed until you send it to a marketplace.'
						: 'This resource is on no marketplace. Saving writes your own catalogue and contacts nobody.'}
				</p>
			{/if}

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

			{#if UNCOLLECTED_FIELDS.length > 0}
				<p class="res-foot">
					Everything above is saved when you {editing === null ? 'create the draft' : 'save'},
					apart from
					{UNCOLLECTED_FIELDS.join('; ')}. It is said here so nothing reads as saved that was not.
				</p>
			{/if}

			<div class="res-acts">
				<Button tier="primary" type="submit" disabled={!canCreate} reason={blocking}>
					{#if creating}
						{editing === null ? 'Creating…' : 'Saving…'}
					{:else}
						{editing === null ? 'Create draft' : 'Save changes'}
					{/if}
				</Button>
				<Button href="/resources">Cancel</Button>
				<span class="res-note">
					{draft.payload.length}
					{draft.payload.length === 1 ? 'file' : 'files'} ·
					{draft.inventories.length === 0
						? 'kept here'
						: `${draft.inventories.length} ${draft.inventories.length === 1 ? 'marketplace' : 'marketplaces'}`}
					{#if divergentOn(draft, 'name').length + divergentOn(draft, 'description').length + divergentOn(draft, 'price').length > 0}
						· some marketplace values differ
					{/if}
				</span>
			</div>
		</Panel>
		</fieldset>
	</form>

	<!-- Centred, and a modal rather than a banner, because it answers an action
	     the seller has just taken and a banner further down the page is exactly
	     what they would not see. `showModal` centres it; the page's own dialog
	     rules do the rest. -->
	<dialog
		class="res-warn"
		bind:this={fileFirstDialog}
		aria-labelledby="file-first-title"
		onclose={() => (fileFirst = false)}
	>
		<div class="dialog-body">
			<h2 id="file-first-title">Add your file first</h2>
			<p>
				A marketplace cannot list something buyers cannot download. Upload the file, and then this
				listing can be drafted or made live on
				{draft.inventories.length === 1
					? platformTitle(draft.inventories[0])
					: 'the marketplaces you chose'}.
			</p>
			<p class="res-note">
				Until then it is saved here as a draft, and nothing is sent anywhere.
			</p>
			<div class="actions">
				<button class="btn cta" type="button" onclick={() => (fileFirst = false)}>
					Upload the file
				</button>
			</div>
		</div>
	</dialog>
</div>
