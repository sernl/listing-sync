<script lang="ts">
	import { createQueries, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import {
		ApiFailure,
		api,
		type CheckView,
		type FileHandle,
		type VocabularyView
	} from '$lib/api';
	import { licenceGated, licenceOptions, payloadRefusal, requiredFields, fieldWords } from '$lib/authoring';
	import {
		AUTHORABLE_PLATFORMS,
		MARKETPLACE_TILES,
		MARKETPLACE_WORD,
		MARK_SRC,
		TES_CURRICULA,
		platformTitle
	} from '$lib/platforms';
	import { MARKETPLACE_OF } from '$lib/listings-view';
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
	import PreviewField from './PreviewField.svelte';
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
		draftInputOf,
		draftOf,
		emptySlots,
		emptyTptDraft,
		markUp,
		marketplacesReached,
		needsFileBeforeMarketplace,
		patchBodyOf,
		payloadOf,
		projectionOf,
		refusalsOf,
		shouldLandOnCreated,
		sizeWords,
		slotsFrom,
		slotsSettling,
		submittable,
		tesNeedsCurriculum,
		thumbnailHashes,
		thumbnailRefusal,
		suggestedAdditionalLicence,
		withMarketplaces,
		withOverride,
		GRADE_LABELS_KEY,
		GROUP_HELP,
		OVERRIDABLE,
		STANDARDS_HELP,
		type FormAnchor,
		type FormMode,
		type GradeLabels,
		type MarkKind,
		type Refusal,
		type StoredFile,
		type ThumbnailSlot,
		type TptDraft
	} from '$lib/tpt-form';
	import { loadCore } from '$lib/core';
	import type { InventoryId, Marketplace } from '$lib/generated/vocab';

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

	// Whose name the preview maker writes across the pages. The teacher's own
	// shop name, never one composed here, and the maker asks before it writes
	// anything at all.
	const organisation = createQuery(() => ({ queryKey: queryKeys.org, queryFn: () => api.org() }));

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
	/** The files the teacher added in this session, which is what the create
	 *  carries. The edit path has a sub-resource of its own and does not use
	 *  this. */
	let added = $state<StoredFile[]>([]);
	/** The last PDF chosen in this session, and the only thing a preview can be
	 *  cut out of: the maker reads the bytes in this browser. */
	let sourcePdf = $state<File | null>(null);
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
	let descriptionBox = $state<HTMLTextAreaElement | null>(null);

	/** Which words the grade grid is written in.
	 *
	 *  Remembered per browser rather than per listing: a teacher who works in
	 *  years works in years on every listing, and being asked again on each one
	 *  is the thing the toggle was added to stop. */
	let gradeLabels = $state<GradeLabels>('american');
	$effect(() => {
		const held = localStorage.getItem(GRADE_LABELS_KEY);
		if (held === 'american' || held === 'british') {
			gradeLabels = held;
		}
	});

	function setGradeLabels(chosen: GradeLabels) {
		gradeLabels = chosen;
		localStorage.setItem(GRADE_LABELS_KEY, chosen);
	}

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
			// with a list this slot has no way to name. Slot-bound, so the server
			// refuses bytes that are not a picture rather than storing a
			// worksheet a seller dropped here; no progress is reported because a
			// thumbnail is small enough that a meter would only flicker.
			const landed = await api.upload(file, 'keep_whole', undefined, 'image');
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
	// the seller left the fields alone — with Create disabled behind it.
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
			return editing === null ? 'The listing is being created.' : 'The change is being saved.';
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
		return canCreate ? undefined : 'Fix the errors listed above.';
	});

	const selected = $derived(
		AUTHORABLE_PLATFORMS.filter((inventory) => draft.inventories.includes(inventory))
	);

	/** The rail: the listing, then one segment per marketplace it will reach.
	 *  A marketplace's count is how many of its values differ from the
	 *  listing's, which is what the seller would otherwise have to open each
	 *  tab to discover; the listing itself has nothing to count. */
	const tabs = $derived([
		{
			id: 'canonical',
			label: 'This listing',
			count: null,
			hint: 'The values every marketplace takes unless one is changed.'
		},
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

	/** The files the teacher has added, and what the create carries for them.
	 *
	 *  The cover is the first file's, because the thumbnail is drawn from the
	 *  file buyers see first; removing that file promotes the next one's, which
	 *  is what a teacher reordering by removal would expect. */
	function takeFiles(files: StoredFile[]) {
		added = files;
		serverRefusal = null;
		draft = { ...draft, ...payloadOf(files) };
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
		return draft.cover === null ? null : `/v1/uploads/${draft.cover.hash}`;
	});

	/** The previews this listing carries, whichever mode composed them. */
	const previews = $derived.by((): FileHandle[] => {
		if (editing === null) {
			return draft.previews;
		}
		return editing.product.files
			.filter((file) => file.role === 'preview')
			.map((file) => ({
				hash: file.hash,
				kind: file.kind,
				byte_len: file.byte_len,
				...(file.name === undefined ? {} : { name: file.name })
			}));
	});

	async function addPreview(handle: FileHandle) {
		if (editing === null) {
			set('previews', [...draft.previews, handle]);
			return;
		}
		try {
			await api.addProductFile(editing.product.id, 'preview', handle);
			await queryClient.invalidateQueries({ queryKey: queryKeys.product(editing.product.id) });
		} catch (failure) {
			serverRefusal =
				failure instanceof ApiFailure
					? sentenceFor(failure, 'The preview was not added.')
					: 'The preview was not added.';
		}
	}

	async function dropPreview(hash: string) {
		if (editing === null) {
			set(
				'previews',
				draft.previews.filter((held) => held.hash !== hash)
			);
			return;
		}
		const stored = editing.product.files.find(
			(file) => file.role === 'preview' && file.hash === hash
		);
		if (stored === undefined) {
			return;
		}
		try {
			await api.removeProductFile(editing.product.id, stored.id);
			await queryClient.invalidateQueries({ queryKey: queryKeys.product(editing.product.id) });
		} catch (failure) {
			serverRefusal =
				failure instanceof ApiFailure
					? sentenceFor(failure, 'The preview was not removed.')
					: 'The preview was not removed.';
		}
	}

	/** Why this marketplace is ticked and cannot be unticked, or `null`.
	 *
	 *  Edit mode is add-only. `POST /{version}/products/{product}/mappings`
	 *  adds one and no route removes one, so a mapped marketplace renders
	 *  ticked and disabled with the reason said rather than offering a control
	 *  that would silently do nothing. Unlisting stays with the delete flow. */
	function heldBy(marketplace: Marketplace): string | null {
		const mapped = editing?.mapped ?? [];
		const held = mapped.some((inventory) => MARKETPLACE_OF[inventory] === marketplace);
		return held ? 'Already listed here. Remove it from Delete, not from this form.' : null;
	}

	/** The tile one marketplace is drawn as, or undefined for a marketplace a
	 *  server ahead of this bundle named and this grid does not draw. */
	function tileOf(marketplace: Marketplace) {
		return MARKETPLACE_TILES.find((entry) => entry.marketplace === marketplace);
	}

	/** Why this marketplace cannot carry this listing at all, or `null`.
	 *
	 *  Said on the tile rather than after the submit, because a teacher who
	 *  reads it there never reaches the refusal. A marketplace with no adapter
	 *  is the plain case; the rest are the marketplace's own rules, and a Tes
	 *  tile stands for three catalogues, so it is refused only where all three
	 *  refuse it. */
	function tileRefusal(marketplace: Marketplace): string | null {
		const tile = tileOf(marketplace);
		if (tile === undefined) {
			return null;
		}
		if (!tile.authorable) {
			return 'Coming soon';
		}
		const reasons = tile.inventories.map((inventory) => unselectable(inventory));
		return reasons.every((reason) => reason !== null) ? reasons[0] : null;
	}

	/** Why this inventory cannot carry this listing, or `null`.
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
			return `${MARKETPLACE_WORD[MARKETPLACE_OF[inventory]]} needs ${words}, which this form does not ask for yet.`;
		}
		if (draft.payload.length === 0) {
			return null;
		}
		const refusal = payloadRefusal(view.authoring.payload_files, draft.payload.length);
		return refusal === null
			? null
			: `${MARKETPLACE_WORD[MARKETPLACE_OF[inventory]]} ${refusal}.`;
	}

	function toggleTile(marketplace: Marketplace, on: boolean) {
		if (heldBy(marketplace) !== null) {
			return;
		}
		const next = on
			? [...draft.marketplaces, marketplace]
			: draft.marketplaces.filter((held) => held !== marketplace);
		// Unticking Tes takes its curriculum answers with it: they only mean
		// anything under a ticked Tes, and leaving them would silently re-list
		// three catalogues the next time it was ticked.
		const curricula = !on && marketplace === 'Tes' ? [] : draft.curricula;
		if (editing !== null) {
			// Edit mode adds through the mappings route, so the tick opens the
			// panel and the write happens per catalogue. TPT is one catalogue,
			// so ticking it is the write.
			draft = { ...draft, marketplaces: next, curricula };
			// A one-catalogue marketplace has nothing more to ask, so ticking it
			// is the write; Tes waits for a curriculum.
			const only = tileOf(marketplace)?.inventories;
			if (on && only?.length === 1) {
				void addMarketplace(only[0]);
			}
			return;
		}
		draft = withMarketplaces(draft, next, curricula);
		if (needsFileBeforeMarketplace(draft)) {
			fileFirst = true;
		}
	}

	function toggleCurriculum(inventory: InventoryId, on: boolean) {
		const next = on
			? [...draft.curricula, inventory]
			: draft.curricula.filter((held) => held !== inventory);
		if (editing !== null) {
			draft = { ...draft, curricula: next };
			if (on) {
				void addMarketplace(inventory);
			}
			return;
		}
		draft = withMarketplaces(draft, draft.marketplaces, next);
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
		if (editing.product.files.every((file) => file.role !== 'payload')) {
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

	/** The marketplaces this listing reaches that gate a licence, and the values
	 *  they offer under the pricing branch this listing is on. The Tes gate
	 *  refuses a Creative Commons licence with a price and refuses `TES-PAID`
	 *  without one, so ticking Free changes the list rather than only the price. */
	const licensing = $derived(licenceGated(draft.inventories, known));
	const licences = $derived(
		licenceOptions(known.get(licensing[0] ?? 'Tpt'), draft.free ? 'free' : 'paid')
	);

	/** Whether this marketplace's own panel asks for a licence. Read per
	 *  marketplace so the panel that asks is the panel the refusal points at. */
	function gatesLicence(marketplace: Marketplace): boolean {
		return licensing.some((inventory) => MARKETPLACE_OF[inventory] === marketplace);
	}

	/** The panels to draw, in tile order: one per marketplace the teacher
	 *  ticked, each holding only what that marketplace asks for. */
	const panels = $derived(
		MARKETPLACE_TILES.filter(
			(tile) => tile.authorable && draft.marketplaces.includes(tile.marketplace)
		)
	);

	// Selecting a marketplace, then uploading a file it cannot carry, would
	// leave it ticked and refused. Dropping it here keeps the tiles and the
	// refusals saying the same thing.
	$effect(() => {
		if (editing !== null) {
			return;
		}
		const kept = draft.marketplaces.filter((marketplace) => tileRefusal(marketplace) === null);
		if (kept.length !== draft.marketplaces.length) {
			draft = withMarketplaces(draft, kept, draft.curricula);
		}
	});

	// The Multiple Licenses pre-fill, seeded once and never afterwards: TPT's
	// help centre says the seller may choose any discount, so a figure they
	// typed is theirs and recomputing it would overwrite it (D6).
	$effect(() => {
		if (!draft.free && draft.additionalLicence === '' && draft.price !== '' && form !== null) {
			const seeded = suggestedAdditionalLicence(
				draft.price,
				form.limits.additional_licence_percentage
			);
			if (seeded !== '') {
				draft = { ...draft, additionalLicence: seeded };
			}
		}
	});

	/** One toolbar button: Markdown written around what the teacher selected,
	 *  and the caret put back where they would type next. */
	function format(kind: MarkKind) {
		const box = descriptionBox;
		if (box === null) {
			return;
		}
		const marked = markUp(box.value, box.selectionStart, box.selectionEnd, kind);
		set('description', marked.text);
		// After the render that carries the new value, or the browser would
		// restore the caret into the old one.
		queueMicrotask(() => {
			box.focus();
			box.setSelectionRange(marked.start, marked.end);
		});
	}

	/** The band a refusal belongs to, scrolled to and its first control
	 *  focused. Clicking an error is how the founder asked to reach the field,
	 *  so the page moves and the caret lands rather than only the URL changing. */
	function goTo(group: FormAnchor) {
		const band = document.getElementById(`group-${group}`);
		if (band === null) {
			return;
		}
		band.scrollIntoView({ behavior: 'smooth', block: 'start' });
		const control = band.querySelector<HTMLElement>(
			'input:not([type="hidden"]):not(:disabled), select:not(:disabled), textarea:not(:disabled)'
		);
		control?.focus({ preventScroll: true });
	}

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
			toast(
				'info',
				createdToast(marketplacesReached(created.mappings.map((mapping) => mapping.inventory)))
			);
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
					: `Saved. It reaches ${patched.reaches.map(platformTitle).join(', ')} on the next send.`
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
			return 'This listing is live on a marketplace whose edits we cannot make yet, so the change was not sent.';
		}
		return sentenceFor(failure, 'The edit was not saved.');
	}
</script>

<div class={editing === null ? 'page resources-page' : 'resources-page'}>
	{#if editing === null}
		<PageHead
			icon="circle-plus"
			title="New resource"
			description="Fill this in once, then choose where it goes."
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

	<form class="res-form" onsubmit={submit}>
		<!-- One surface rather than a dozen cards: the tabs are its head strip,
		     the sections are hairline-separated bands inside it, and the actions
		     are the band at its foot. The tabs sit outside the lock
		     deliberately — a published listing holds the fields back, and a
		     seller who cannot edit can still read what each marketplace
		     carries. -->
		<div class="res-card">
			<TabBar {tabs} bind:current={tab} />
			<!-- One `disabled` for the whole form rather than one per control. A
			     published listing on a platform whose edit transition we have not
			     captured cannot be edited through us, and the server refuses the
			     request, so every field is shown as stored and none of them takes
			     an edit. A `fieldset` disables its descendants by the user
			     agent's own rule, which is why this is the wrapper rather than a
			     class. -->
			<fieldset class="res-lock" disabled={locked}>
				{#if tab === 'canonical'}
					<!-- Where this goes, first. The founder's rule: the marketplace
					     is the decision every other field on this page is made
					     under, so it is asked before them rather than after. -->
					<FormSection
						group="marketplaces"
						icon="store"
						help={GROUP_HELP.marketplaces}
						{refusals}
					>
						<MarketplacePicker
							chosen={draft.marketplaces}
							refusalOf={tileRefusal}
							heldOf={heldBy}
							onToggle={toggleTile}
						/>
					</FormSection>

					<FormSection group="name" icon="tag" {refusals}>
						<Field label="Title" id="draft-title" required>
							<input
								id="draft-title"
								type="text"
								required
								placeholder="Name your product"
								maxlength={form === null ? undefined : form.limits.title_max_utf16_units}
								value={draft.name}
								oninput={(event) => set('name', event.currentTarget.value)}
							/>
							<!-- After the control rather than before it: read in order
							     this is label, field, count, which is the order a screen
							     reader and a tab both take it in. -->
							{#if form}
								<span
									class="res-count"
									class:over={draft.name.length > form.limits.title_max_utf16_units}
								>
									{draft.name.length} of {form.limits.title_max_utf16_units} characters
								</span>
							{/if}
						</Field>
					</FormSection>

					<FormSection group="files" icon="package" help={GROUP_HELP.files} {refusals}>
						{#if editing === null}
							<UploadField
								files={added}
								{keepWhole}
								limits={form?.limits ?? null}
								onFiles={takeFiles}
								onPdf={(file) => (sourcePdf = file)}
								onKeepWhole={(whole) => (keepWhole = whole)}
							/>
						{:else}
							<!-- A saved resource's files are a sub-resource with three
							     routes of their own, and this panel is what drives them.
							     Not the upload control: before a product exists there is
							     nothing for those routes to address, and once one does an
							     upload here would be a second way to change the same
							     thing. -->
							<FilesPanel
								product={editing.product.id}
								files={editing.product.files}
								inventories={editing.mapped}
							/>
						{/if}
					</FormSection>

					<FormSection group="preview" icon="eye" help={GROUP_HELP.preview} {refusals}>
						<PreviewField
							{previews}
							limits={form?.limits ?? null}
							source={sourcePdf}
							sellerName={organisation.data?.name ?? ''}
							onAdd={(handle) => void addPreview(handle)}
							onRemove={(hash) => void dropPreview(hash)}
						/>
					</FormSection>

					<FormSection group="thumbnails" icon="image" {refusals}>
						{#if form}
							<ThumbnailSlots
								{form}
								mode={draft.thumbnailMode}
								{slots}
								{coverUrl}
								onMode={(id) => set('thumbnailMode', id)}
								onPick={(index, file) => void takeThumbnail(index, file)}
								onClear={clearThumbnail}
							/>
						{:else}
							<p class="res-note">The thumbnail slots are still being read.</p>
						{/if}
					</FormSection>

					<FormSection
						group="description"
						icon="book-open"
						help={GROUP_HELP.description}
						{refusals}
					>
						<Field label="Description" id="draft-description" required>
							<div class="res-md" role="group" aria-label="Formatting">
								<button type="button" class="res-md-b" onclick={() => format('bold')}>
									<b>B</b><span class="sr-only">Bold</span>
								</button>
								<button type="button" class="res-md-b" onclick={() => format('italic')}>
									<i>I</i><span class="sr-only">Italic</span>
								</button>
								<button type="button" class="res-md-b" onclick={() => format('bullets')}>
									•<span class="sr-only">Bulleted list</span>
								</button>
								<button type="button" class="res-md-b" onclick={() => format('numbers')}>
									1.<span class="sr-only">Numbered list</span>
								</button>
							</div>
							<textarea
								id="draft-description"
								bind:this={descriptionBox}
								aria-required="true"
								placeholder="Describe your product and how it can be helpful to another educator"
								value={draft.description}
								oninput={(event) => set('description', event.currentTarget.value)}
							></textarea>
							{#if form}
								<span
									class="res-count"
									class:over={draft.description.length > form.limits.description_max_length}
								>
									{draft.description.length} of {form.limits.description_max_length} characters
								</span>
							{/if}
						</Field>
						<p class="res-foot">
							Use the buttons to format. Each marketplace shows it in its own style.
						</p>
					</FormSection>

					<FormSection
						group="price"
						icon="credit-card"
						help={GROUP_HELP.price}
						{refusals}
						{advisories}
					>
						<fieldset class="res-choices">
							<legend>Free or paid</legend>
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
									Keep a free resource to {form.limits.free_resource_page_guidance} pages or fewer.
								</span>
							{/if}
						</fieldset>

						{#if !draft.free}
							<div class="res-row">
								<Field label="Price" id="draft-price" required>
									<input
										id="draft-price"
										type="text"
										inputmode="decimal"
										aria-required="true"
										placeholder="0.00"
										value={draft.price}
										oninput={(event) => set('price', event.currentTarget.value)}
									/>
								</Field>
								<!-- The percentage is TPT's own and is stated only when TPT
								     has been asked. A literal here read as a rule of theirs
								     on a page whose banner said their rules could not be
								     read. -->
								<Field
									label="Multiple Licenses"
									id="draft-additional-licence"
									required
									hint={form === null
										? 'Set the price for extra copies.'
										: `We suggest ${form.limits.additional_licence_percentage}% of the price. Change it if you like.`}
								>
									<input
										id="draft-additional-licence"
										type="text"
										inputmode="decimal"
										aria-required="true"
										placeholder="0.00"
										value={draft.additionalLicence}
										oninput={(event) => set('additionalLicence', event.currentTarget.value)}
									/>
								</Field>
								<Field
									label="Bundle Discount Price"
									id="draft-bundle-discount"
									hint="Enter the price for the whole bundle. A price is easier for buyers to compare than a percentage."
								>
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
						{/if}
					</FormSection>

					<FormSection
						group="categories"
						icon="layout-template"
						help={GROUP_HELP.categories}
						{refusals}
					>
						{#if form}
							<GradeGrid
								vocabulary={form}
								chosen={draft.grades}
								labels={gradeLabels}
								onChange={(grades) => set('grades', grades)}
								onLabels={setGradeLabels}
							/>

							<FacetPicker
								label="Subject Area"
								required
								placeholder="Select up to three subject areas"
								facets={form.subject_areas}
								chosen={draft.subjectAreas}
								cap={capOf(form.caps, 'subjectAreas')}
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
								hint="A custom category is any word or phrase you would like to use to group your own resources. These are your own categories rather than a marketplace vocabulary, which is why they are different from the three options above."
							>
								<input
									id="draft-custom-category"
									type="text"
									placeholder="Add a category and press Enter"
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
										{#each draft.customCategories as category (category)}
											<span class="res-chip" role="listitem">
												{category}
												<button
													type="button"
													aria-label="Remove {category}"
													onclick={() =>
														set(
															'customCategories',
															draft.customCategories.filter((held) => held !== category)
														)}
												>
													×
												</button>
											</span>
										{/each}
									</div>
								{/if}
							</Field>
						{:else}
							<p class="res-note">The category lists are still being read.</p>
						{/if}
					</FormSection>

					<FormSection
						group="education_standards"
						icon="shield-check"
						help={(form?.standards_frameworks.length ?? 0) > 0 ? STANDARDS_HELP : undefined}
						{refusals}
					>
						{#if form}
							<StandardsPicker
								frameworks={form.standards_frameworks}
								chosen={draft.standards}
								onChange={(standards) => set('standards', standards)}
							/>
						{:else}
							<p class="res-note">The standards frameworks are still being read.</p>
						{/if}
					</FormSection>

					<FormSection
						group="details"
						icon="sliders-horizontal"
						help={GROUP_HELP.details}
						{refusals}
					>
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
							<Field label="Answer Key" id="draft-answer-key">
								<select
									id="draft-answer-key"
									value={draft.answerKey ?? ''}
									onchange={(event) =>
										set(
											'answerKey',
											event.currentTarget.value === '' ? null : event.currentTarget.value
										)}
								>
									<option value="">N/A</option>
									{#each form?.answer_keys ?? [] as key (key.id)}
										<option value={key.id}>{key.label}</option>
									{/each}
								</select>
							</Field>
						</div>
					</FormSection>

					<!-- One panel per marketplace the teacher ticked, headed by its
					     own mark, holding only what that marketplace asks for. The
					     founder's rule: a field that belongs to one marketplace has
					     to say so where it is asked. -->
					{#each panels as panel (panel.marketplace)}
						{@const anchor = panel.marketplace === 'Tpt' ? 'tpt_options' : 'tes_options'}
						<FormSection group={anchor} mark={MARK_SRC[panel.marketplace]} {refusals}>
							{#if panel.marketplace === 'Tpt'}
								{#if form}
									<div class="res-picks" role="radiogroup" aria-label="Copyright">
										<p class="res-lede">{form.copyright.preamble}</p>
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

									{#if !draft.free}
										<Field
											label="Tax Code"
											id="draft-tax-code"
											required
											hint="Needed so sales tax can be collected on TPT."
										>
											<select
												id="draft-tax-code"
												aria-required="true"
												value={draft.taxCode ?? ''}
												onchange={(event) =>
													set(
														'taxCode',
														event.currentTarget.value === '' ? null : event.currentTarget.value
													)}
											>
												<option value="">Select a tax code</option>
												{#each form.tax_codes as code (code.id)}
													<option value={code.id}>{code.label}</option>
												{/each}
											</select>
										</Field>
									{/if}

									<fieldset class="res-choices">
										<legend>Localization</legend>
										<label>
											<!-- Three states, because the sidecar holds three. A
											     product that was never asked draws the box
											     indeterminate rather than unticked: reading the two
											     the same way is what would let a first save turn
											     "not stated" into "explicitly no". -->
											<input
												type="checkbox"
												checked={draft.appropriateForCountry === true}
												indeterminate={draft.appropriateForCountry === null}
												onchange={(event) =>
													set('appropriateForCountry', event.currentTarget.checked)}
											/>
											{form.localisation.label ?? form.localisation.generic_label}
										</label>
									</fieldset>
								{:else}
									<p class="res-note">TPT's own questions are still being read.</p>
								{/if}
							{:else}
								<div class="res-group">
									<span class="res-group-label" id="tes-curriculum">
										Curriculum<span class="req">Required</span>
									</span>
									<span class="res-note">Choose where on Tes this should be listed.</span>
									<div class="res-picks" role="group" aria-labelledby="tes-curriculum">
										{#each TES_CURRICULA as entry (entry.inventory)}
											{@const mapped = editing?.mapped.includes(entry.inventory) ?? false}
											<label class="res-pick">
												<input
													type="checkbox"
													checked={mapped || draft.curricula.includes(entry.inventory)}
													disabled={mapped || adding !== null}
													onchange={(event) =>
														toggleCurriculum(entry.inventory, event.currentTarget.checked)}
												/>
												<span class="res-pick-t">{entry.label}</span>
											</label>
										{/each}
									</div>
								</div>
							{/if}

							{#if gatesLicence(panel.marketplace)}
								<Field
									label="Licence"
									id="draft-licence"
									required
									hint="{MARKETPLACE_WORD[panel.marketplace]} needs a licence to list this."
								>
									<select
										id="draft-licence"
										aria-required="true"
										value={draft.licence ?? ''}
										onchange={(event) =>
											set('licence', event.currentTarget.value === '' ? null : event.currentTarget.value)}
									>
										<option value="">Choose a licence</option>
										{#each licences as option (option.id)}
											<option value={option.id}>{option.label}</option>
										{/each}
									</select>
								</Field>
							{/if}
						</FormSection>
					{/each}

					<FormSection
						group="product_status"
						icon="circle-check"
						help={GROUP_HELP.product_status}
						{refusals}
					>
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
					</FormSection>
				{:else}
					{@const where = tab as InventoryId}
					{@const view = known.get(where)}
					{@const projection = projectionOf(draft, where, view ?? null)}
					<Panel
						title={platformTitle(where)}
						description="What this marketplace will carry. Values follow the listing unless you change one here."
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
											The listing still reads “{canonicalValue(draft, row.key)}”.
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
										It lands in this marketplace's “{facts.native}” field.
										{#if facts.cap !== null}
											It takes {facts.cap}.
										{/if}
										{#if row.loss}
											<span class="res-loss">{row.loss}</span>
										{/if}
									</div>
								</div>
							{/if}
						{/each}

						{#if view}
							{#each view.absent_axes as axis (axis)}
								<p class="res-disclose">
									This marketplace has no {axis} field at all, so a {axis} you state here does
									not reach it.
								</p>
							{/each}
						{/if}
					</Panel>
				{/if}

				{#if refusals.length > 0 || (serverCheck !== null && !serverCheck.submittable) || serverRefusal !== null || (editing !== null && editing.blockedBy.length > 0)}
					<section class="res-sec">
						{#if editing !== null && editing.blockedBy.length > 0}
							<Banner tone="warn" title="This resource is live and cannot be edited through us">
								{editing.blockedBy.map(platformTitle).join(', ')} has a published listing, and
								editing one there is not something we can do yet. The fields above are shown as
								stored and the change is held back.
							</Banner>
						{/if}

						{#if refusals.length > 0}
							<p class="res-errs-h">
								These are the errors that need to be fixed. Click one to go to that section.
							</p>
							<ul class="res-errs">
								{#each refusals as refusal, index (`${refusal.group}-${index}`)}
									<li>
										<button
											type="button"
											class="btn small"
											onclick={() => goTo(refusal.group)}
										>
											{refusal.message}
										</button>
									</li>
								{/each}
							</ul>
						{/if}

						{#if serverCheck !== null && !serverCheck.submittable}
							<ul class="res-errs">
								{#each serverCheck.refusals as refusal, index (`server-${index}`)}
									<li><span class="res-note">{refusal.message}</span></li>
								{/each}
							</ul>
						{/if}

						{#if serverRefusal !== null}
							<Banner tone="bad">{serverRefusal}</Banner>
						{/if}
					</section>
				{/if}

				<!-- The action band, in flow at the foot of the card: Cancel and
				     the primary at the right, in the order a tab takes them. -->
				<div class="res-actbar">
					<Button href="/resources">Cancel</Button>
					<Button tier="primary" type="submit" disabled={!canCreate} reason={blocking}>
						{#if creating}
							{editing === null ? 'Creating…' : 'Saving…'}
						{:else}
							{editing === null ? 'Create listing' : 'Save changes'}
						{/if}
					</Button>
				</div>
			</fieldset>
		</div>
	</form>

	<!-- Centred, and a modal rather than a banner, because it answers an action
	     the seller has just taken and a banner further down the page is exactly
	     what they would not see. -->
	<dialog
		class="res-warn"
		bind:this={fileFirstDialog}
		aria-labelledby="file-first-title"
		onclose={() => (fileFirst = false)}
	>
		<div class="dialog-body">
			<h2 id="file-first-title">Add your file first</h2>
			<p>
				A marketplace cannot list something buyers cannot download. Add the file, and then this
				listing can go to
				{draft.marketplaces.length === 1
					? MARKETPLACE_WORD[draft.marketplaces[0]]
					: 'the marketplaces you chose'}.
			</p>
			<div class="actions">
				<button class="btn cta" type="button" onclick={() => (fileFirst = false)}>
					Add the file
				</button>
			</div>
		</div>
	</dialog>
</div>
