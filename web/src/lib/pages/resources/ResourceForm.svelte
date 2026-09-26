<script lang="ts">
	import { createQueries, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { tick, type Snippet } from 'svelte';
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
		platformTitle
	} from '$lib/platforms';
	import { MARKETPLACE_OF } from '$lib/listings-view';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import FlowDiagram from '$lib/FlowDiagram.svelte';
	import FlowStep from '$lib/FlowStep.svelte';
	import FormSection from '$lib/FormSection.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import { DISCLAIMER } from '$lib/pages/marketplaces/catalogue';
	import StatusPill from '$lib/StatusPill.svelte';
	import Stepper, { type StepMark } from '$lib/Stepper.svelte';
	import TabBar from '$lib/TabBar.svelte';
	import UploadField from '$lib/UploadField.svelte';
	import { queryKeys } from '$lib/query';
	import { refreshAfterCreate } from './after-create';
	import FilesPanel from './FilesPanel.svelte';
	import MarketplacePicker from './MarketplacePicker.svelte';
	import PreviewField from './PreviewField.svelte';
	import { desktopInvoker, libraryEntries } from '$lib/desktop';
	import { keptPdfSource } from './file-viewer';
	import ThumbnailSlots from './ThumbnailSlots.svelte';
	import { coverUrlOf } from './files';
	import CategoriesPanel from './panels/CategoriesPanel.svelte';
	import DescriptionPanel from './panels/DescriptionPanel.svelte';
	import DetailsPanel from './panels/DetailsPanel.svelte';
	import MarketplacePanel from './panels/MarketplacePanel.svelte';
	import PricePanel from './panels/PricePanel.svelte';
	import StandardsPanel from './panels/StandardsPanel.svelte';
	import StatusPanel from './panels/StatusPanel.svelte';
	import { createRefusal, sentenceFor } from './refusal';
	import { templateKeys, templates } from '$lib/pages/templates/api';
	import { filledLine, mergeIntoEmpty } from '$lib/pages/templates/resource-template';
	import './resources.css';
	import { toast } from '$lib/toast';
	import {
		advisoriesOf,
		applyToAll,
		canonicalValue,
		createBodyOf,
		createdToast,
		diverges,
		draftInputOf,
		draftOf,
		emptySlots,
		emptyTptDraft,
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
		thumbnailHashes,
		thumbnailRefusal,
		suggestedAdditionalLicence,
		withMarketplaces,
		withOverride,
		AI_FILL_SOON,
		AI_FILL_SOON_HINT,
		GRADE_LABELS_KEY,
		GROUP_HELP,
		OVERRIDABLE,
		type FormAnchor,
		type FormMode,
		type GradeLabels,
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
	let {
		mode = { kind: 'create' } as FormMode,
		listed,
		publish,
		after
	}: {
		mode?: FormMode;
		/** The resource page's own marketplace tiles, drawn in step 3 above the
		 *  picker: where it is listed now, with each listing's status. */
		listed?: Snippet;
		/** The resource page's Publish control, beside Save in the sticky bar. */
		publish?: Snippet;
		/** What the resource page shows under the steps: sends, labels,
		 *  collections. */
		after?: Snippet;
	} = $props();

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

	/** The templates this seller has saved, for the picker at the head of a new
	 *  resource. Heads only, which is what the list serves: the draft is read
	 *  when one is picked, so opening the form costs one small request rather
	 *  than every draft the seller holds. Not read at all in edit mode — a
	 *  template is a starting point, and a saved resource has already
	 *  started. */
	const templateHeads = createQuery(() => ({
		queryKey: templateKeys.all,
		queryFn: () => templates.list(),
		enabled: mode.kind === 'create'
	}));

	/** What the last template filled, and what the draft was before it did, so
	 *  Undo puts back exactly what the seller had. Cleared by the next pick
	 *  and by the Undo itself. */
	let started = $state<{ name: string; filled: string[]; before: TptDraft } | null>(null);
	let starting = $state(false);
	let startRefusal = $state<string | null>(null);

	/** Reads one template and fills the fields this draft has not answered.
	 *
	 *  Fill-the-empty-ones rather than replace: a seller who picks a template
	 *  after typing keeps what they typed, which is the same rule the apply
	 *  route follows over saved resources. */
	async function startFrom(id: string) {
		if (id === '') {
			return;
		}
		starting = true;
		startRefusal = null;
		try {
			const template = await queryClient.fetchQuery({
				queryKey: templateKeys.one(id),
				queryFn: () => templates.read(id)
			});
			const merged = mergeIntoEmpty(draft, template.draft);
			draft = merged.draft;
			started = { name: template.name, filled: merged.filled, before: merged.before };
		} catch (failure) {
			startRefusal =
				failure instanceof ApiFailure
					? failure.message
					: 'That template didn’t load, so nothing was filled in.';
		} finally {
			starting = false;
		}
	}

	function undoStart() {
		if (started === null) {
			return;
		}
		draft = started.before;
		started = null;
	}

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
	/** The last PDF chosen in this session, and one of the two things a
	 *  preview can be cut out of: the maker reads the bytes in this browser. */
	let sourcePdf = $state<File | null>(null);
	/** The other: a stored payload the Teachouse app keeps on this machine.
	 *  Read once, from the application, and empty in a browser. */
	const invoke = desktopInvoker();
	let kept = $state<Set<string>>(new Set());
	$effect(() => {
		void (async () => {
			const answer = await libraryEntries(invoke);
			if (answer.kind === 'ok') {
				kept = new Set(answer.value.map((entry) => entry.hash));
			}
		})();
	});
	const keptSource = $derived(
		editing === null ? null : keptPdfSource(invoke, editing.product.files, kept)
	);
	let keepWhole = $state(false);
	let creating = $state(false);
	// The marketplace being added in edit mode, so a second tick cannot start a
	// second add and the rail says which one is in flight.
	let adding = $state<InventoryId | null>(null);
	let serverCheck = $state<CheckView | null>(null);
	let serverRefusal = $state<string | null>(null);
	/** Which marketplace the "what each marketplace shows" fold is reading. */
	let tab = $state<string>('');
	// Opened by the act of choosing a marketplace with no file, and by pressing
	// Create in that state. Not by an effect over the condition: an effect would
	// reopen it under a seller who had read it and closed it.
	let fileFirst = $state(false);
	let fileFirstDialog = $state<HTMLDialogElement | null>(null);
	// The four TPT slots, each holding the seller's own bytes as the browser can
	// draw them and, once the upload answers, the handle the create carries.
	let slots = $state<ThumbnailSlot[]>(emptySlots());

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
				refusal: handle === null ? 'That file didn’t upload properly. Try again.' : null
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
		message: 'The form didn’t load properly. Reload the page before you save.'
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
			return 'You can’t edit a published listing here.';
		}
		if (creating) {
			return editing === null ? 'Creating your listing…' : 'Saving your changes…';
		}
		if (slotsSettling(slots)) {
			return 'A thumbnail is still uploading.';
		}
		if (rulesFailed) {
			return 'The form didn’t load properly. Reload the page to try again.';
		}
		if (vocabulary.isError) {
			return 'The form didn’t load properly. Reload the page to try again.';
		}
		if (form === null) {
			return 'The form is still loading.';
		}
		return canCreate ? undefined : 'Fix the errors listed above.';
	});

	const selected = $derived(
		AUTHORABLE_PLATFORMS.filter((inventory) => draft.inventories.includes(inventory))
	);

	/** One segment per marketplace this listing will reach. A marketplace's
	 *  count is how many of its values differ from the listing's, which the
	 *  seller would otherwise have to open each tab to discover. */
	const overrideTabs = $derived(
		selected.map((inventory) => ({
			id: inventory,
			label: platformTitle(inventory),
			count: OVERRIDABLE.filter((entry) => diverges(draft, inventory, entry.field)).length,
			hint: 'How many fields here are different from your main listing.'
		}))
	);
	const overrideTab = $derived<InventoryId | null>(
		selected.find((inventory) => inventory === tab) ?? selected[0] ?? null
	);
	$effect(() => {
		if (overrideTab !== null && tab !== overrideTab) {
			tab = overrideTab;
		}
	});

	// --- the steps ---------------------------------------------------------

	let openSteps = $state({ files: true, details: true, where: true, publish: true });

	const payloadCount = $derived(
		editing === null
			? added.length
			: editing.product.files.filter((file) => file.role === 'payload').length
	);
	const hasPayload = $derived(payloadCount > 0);
	const filesSummary = $derived(
		payloadCount === 0 ? 'No file yet' : payloadCount === 1 ? '1 file' : `${payloadCount} files`
	);
	const DETAIL_GROUPS: readonly FormAnchor[] = [
		'name',
		'description',
		'price',
		'categories',
		'education_standards',
		'details'
	];
	const detailsDone = $derived(
		draft.name.trim().length > 0 &&
			refusals.every((refusal) => !DETAIL_GROUPS.includes(refusal.group))
	);
	const detailsSummary = $derived(
		draft.name.trim().length === 0
			? 'No title yet'
			: `${draft.name.trim()} · ${draft.free ? 'Free' : draft.price === '' ? 'No price' : draft.price}`
	);
	const stepMarks = $derived<StepMark[]>([
		{ id: 'files', label: 'Files', done: hasPayload },
		{ id: 'details', label: 'Details', done: detailsDone },
		{ id: 'where', label: 'Where it is listed', done: selected.length > 0 },
		{ id: 'publish', label: editing === null ? 'Create' : 'Publish', done: false }
	]);

	/** The step a band sits in, so an error can open its step before
	 *  scrolling to it: a closed step draws none of its controls. */
	function stepOf(group: FormAnchor): keyof typeof openSteps {
		if (group === 'files' || group === 'preview' || group === 'thumbnails') {
			return 'files';
		}
		if (
			group === 'marketplaces' ||
			group === 'tpt_options' ||
			group === 'tes_options' ||
			group === 'copyright'
		) {
			return 'where';
		}
		return group === 'product_status' ? 'publish' : 'details';
	}

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
	 *  show something other than what buyers get.
	 *
	 *  Both are content-addressed: the upload route by its handle, the saved
	 *  route by `?v=<digest>`, so a replaced first file (a redraw on the
	 *  server, a re-derived `draft.cover` here) is a new URL the `<img>` loads. */
	const coverUrl = $derived.by(() => {
		if (editing !== null) {
			return coverUrlOf(editing.product.id, editing.product.files);
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
					? sentenceFor(failure, 'The preview wasn’t added.')
					: 'The preview wasn’t added.';
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
					? sentenceFor(failure, 'The preview wasn’t removed.')
					: 'The preview wasn’t removed.';
		}
	}

	/** A changed preview takes the old one's place. A draft swaps the handle
	 *  where it stood; a saved resource swaps the file in one write, which keeps
	 *  its role and never leaves the listing with both or neither. */
	async function replacePreview(hash: string, handle: FileHandle) {
		if (editing === null) {
			set(
				'previews',
				draft.previews.map((held) => (held.hash === hash ? handle : held))
			);
			return;
		}
		const stored = editing.product.files.find(
			(file) => file.role === 'preview' && file.hash === hash
		);
		try {
			if (stored === undefined) {
				await api.addProductFile(editing.product.id, 'preview', handle);
			} else {
				await api.replaceProductFile(editing.product.id, stored.id, handle);
			}
			await queryClient.invalidateQueries({ queryKey: queryKeys.product(editing.product.id) });
		} catch (failure) {
			serverRefusal =
				failure instanceof ApiFailure
					? sentenceFor(failure, 'The preview wasn’t changed.')
					: 'The preview wasn’t changed.';
		}
	}

	async function renamePreview(hash: string, name: string) {
		if (editing === null) {
			set(
				'previews',
				draft.previews.map((held) => (held.hash === hash ? { ...held, name } : held))
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
			await api.renameProductFile(editing.product.id, stored.id, name);
			await queryClient.invalidateQueries({ queryKey: queryKeys.product(editing.product.id) });
		} catch (failure) {
			serverRefusal =
				failure instanceof ApiFailure
					? sentenceFor(failure, 'The preview wasn’t renamed.')
					: 'The preview wasn’t renamed.';
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
		return held ? 'Already listed here. Use Delete to remove it.' : null;
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
	 *  is the plain case; the rest are the marketplace's own rules. */
	function tileRefusal(marketplace: Marketplace): string | null {
		const tile = tileOf(marketplace);
		if (tile === undefined) {
			return null;
		}
		if (!tile.authorable) {
			return 'Coming soon';
		}
		return unselectable(tile.inventory);
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
		if (editing !== null) {
			// Edit mode adds through the mappings route, so the tick opens the
			// panel and the write happens there; one tile is one inventory, so
			// ticking it is the write.
			draft = { ...draft, marketplaces: next };
			const only = tileOf(marketplace)?.inventory;
			if (on && only !== undefined) {
				void addMarketplace(only);
			}
			return;
		}
		draft = withMarketplaces(draft, next);
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
			draft = withMarketplaces(draft, kept);
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

	/** The band a refusal belongs to, scrolled to and its first control
	 *  focused. Clicking an error is how the founder asked to reach the field,
	 *  so the page moves and the caret lands rather than only the URL changing. */
	async function goTo(group: FormAnchor) {
		openSteps[stepOf(group)] = true;
		await tick();
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
					? 'Saved. This listing isn’t on any marketplace yet.'
					: `Saved. Your changes go to ${patched.reaches.map(platformTitle).join(', ')} next time you send.`
			);
		} catch (failure) {
			serverRefusal = editRefusalOf(failure);
		} finally {
			creating = false;
		}
	}

	function editRefusalOf(failure: unknown): string {
		if (!(failure instanceof ApiFailure)) {
			return 'Your changes weren’t saved.';
		}
		if (failure.code() === 'uncaptured_transition') {
			return 'This listing is live on a marketplace we can’t edit yet, so your change wasn’t sent.';
		}
		return sentenceFor(failure, 'Your changes weren’t saved.');
	}
</script>

<div
	class={editing === null ? 'page resources-page flow-page' : 'resources-page'}
>
	{#if editing === null}
		<!-- Cancel is in the sticky bar beside Create, so the head holds none. -->
		<PageHead icon="circle-plus" title="New resource" guide="new-resource" />
	{/if}

	{#if vocabulary.isError}
		<Banner tone="bad" title="The form didn’t load properly">
			Reload to try again.
		</Banner>
	{/if}

	<!-- Above the steps rather than inside them: a template is where this form
	     starts, not one of the things it asks. Only on a new resource, and only
	     where the seller has saved one. -->
	{#if editing === null && (templateHeads.data?.length ?? 0) > 0}
		<div class="res-start">
			<Field label="Start from a template" id="start-from-template">
				<select
					id="start-from-template"
					disabled={starting}
					onchange={(event) => void startFrom(event.currentTarget.value)}
				>
					<option value="">{starting ? 'Loading the template…' : 'None'}</option>
					{#each templateHeads.data ?? [] as head (head.id)}
						<option value={head.id}>
							{head.name}{head.scope === null ? '' : ` — ${MARKETPLACE_WORD[head.scope]}`}
						</option>
					{/each}
				</select>
			</Field>
			{#if started !== null}
				<p class="res-start-said">
					{filledLine(started.name, started.filled)}
					<Button small tier="quiet" onclick={undoStart}>Undo</Button>
				</p>
			{/if}
			{#if startRefusal !== null}
				<p class="res-start-said">{startRefusal}</p>
			{/if}
		</div>
	{/if}

	<form class="res-form" onsubmit={submit}>
			<div class="flow">
				<Stepper steps={stepMarks} label="Resource steps" />

				{#if editing !== null && editing.blockedBy.length > 0}
					<Banner tone="warn" title="This resource is live, so you can’t edit it here">
						{editing.blockedBy.map(platformTitle).join(', ')} has a published listing, so this
						change won’t be sent.
					</Banner>
				{/if}

				<FlowStep
					n={1}
					id="files"
					title="Files"
					hint="Add the file buyers download, a preview and a thumbnail."
					summary={filesSummary}
					done={hasPayload}
					bind:open={openSteps.files}
				>
					{#snippet aside()}
						<Explain title="Files, previews and thumbnails">
							<p>The file is what buyers download after they buy.</p>
							<p>The preview is a free sample buyers see first. Make it from a few pages of your PDF.</p>
							<p>TPT shows up to four thumbnails. The first one is the cover.</p>
							{#if form}
								<p>
									A thumbnail can be up to {sizeWords(form.limits.thumbnail.max_size_bytes)}.
								</p>
							{/if}
						</Explain>
					{/snippet}

					<FlowDiagram
						from={{ icon: 'package', label: payloadCount === 1 ? '1 file' : payloadCount > 0 ? `${payloadCount} files` : 'Your file' }}
						rule="makes"
						via={{ icon: 'image', label: 'Thumbnail', image: coverUrl }}
						viaRule="goes to"
						to={selected.map((inventory) => ({ inventory }))}
						pending="Step 3"
						label="Your file makes the thumbnail, and both go to {selected.length === 0 ? 'the marketplaces you choose in step 3' : selected.map(platformTitle).join(' and ')}"
					/>

					<!-- One `disabled` per step's controls rather than one per
					     control: a published listing on a platform whose edit we have
					     not captured cannot be edited through us, so every field is
					     shown as stored and takes no edit. The step toggles, the
					     Explains and the marketplace tiles stay live outside it. -->
					<fieldset class="res-lock" disabled={locked}>
						<FormSection group="files" icon="package" {refusals}>
							{#snippet badge()}
								<!-- A notice and not a control: the feature is not built. -->
								{#if editing === null}
									<span class="res-soon" title={AI_FILL_SOON_HINT}>{AI_FILL_SOON}</span>
								{/if}
							{/snippet}
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
								     routes of their own, and this panel is what drives them. -->
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
								source={sourcePdf ?? keptSource}
								uploadLabel={sourcePdf === null && keptSource !== null
									? 'Upload preview to Teachouse'
									: 'Make preview'}
								sellerName={organisation.data?.name ?? ''}
								onAdd={(handle) => void addPreview(handle)}
								onReplace={(hash, handle) => void replacePreview(hash, handle)}
								onRename={(hash, name) => void renamePreview(hash, name)}
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
								<p class="res-note">Loading the thumbnail options…</p>
							{/if}
						</FormSection>
					</fieldset>
				</FlowStep>

				<FlowStep
					n={2}
					id="details"
					title="Details"
					hint="Name it, describe it and set the price."
					summary={detailsSummary}
					done={detailsDone}
					bind:open={openSteps.details}
				>
					<fieldset class="res-lock" disabled={locked}>
						<FormSection group="name" icon="tag" {refusals}>
							<Field label="Title" id="draft-title" required>
								<input
									id="draft-title"
									type="text"
									required
									placeholder="Name your resource"
									maxlength={form === null ? undefined : form.limits.title_max_utf16_units}
									value={draft.name}
									oninput={(event) => set('name', event.currentTarget.value)}
								/>
								<!-- After the control: label, field, count is the order a
								     screen reader and a tab both take it in. -->
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

						<DescriptionPanel {draft} {form} {refusals} {set} />

						<PricePanel {draft} {form} {refusals} {advisories} {set} />

						<CategoriesPanel
							{draft}
							{form}
							{refusals}
							{gradeLabels}
							onLabels={setGradeLabels}
							{set}
						/>

						<StandardsPanel {draft} {form} {refusals} {set} />

						<DetailsPanel {draft} {form} {refusals} {set} />
					</fieldset>
				</FlowStep>

				<FlowStep
					n={3}
					id="where"
					title="Where it is listed"
					hint="Choose the marketplaces for this resource."
					summary={selected.length === 0 ? 'No marketplace yet' : selected.map(platformTitle).join(' · ')}
					done={selected.length > 0}
					bind:open={openSteps.where}
				>
					{#snippet aside()}
						<Explain title="How listing works">
							<p>Your catalogue holds the resource once. Each marketplace gets its own listing of it.</p>
							<p>Nothing changes on a marketplace until you publish.</p>
							<p>{DISCLAIMER}</p>
						</Explain>
					{/snippet}

					<FlowDiagram
						from={{ icon: 'layout-list', label: 'Your catalogue' }}
						to={selected.map((inventory) => ({ inventory }))}
						rule={selected.length === 0 ? null : 'listed on'}
						empty="Pick below"
						pending="Not chosen"
						label={selected.length === 0
							? 'No marketplace chosen yet'
							: `Your catalogue is listed on ${selected.map(platformTitle).join(' and ')}`}
					/>

					{#if listed}{@render listed()}{/if}

					<fieldset class="res-lock" disabled={locked}>
						<!-- The founder's rule: the marketplace is the decision the
						     per-marketplace questions below are asked under. -->
						<FormSection group="marketplaces" icon="store" help={GROUP_HELP.marketplaces} {refusals}>
							<MarketplacePicker
								chosen={draft.marketplaces}
								refusalOf={tileRefusal}
								heldOf={heldBy}
								onToggle={toggleTile}
							/>
						</FormSection>

						<!-- One panel per marketplace the teacher ticked, headed by its
						     own mark, holding only what that marketplace asks for. -->
						{#each panels as panel (panel.marketplace)}
							<MarketplacePanel
								marketplace={panel.marketplace}
								{draft}
								{form}
								{refusals}
								gatesLicence={gatesLicence(panel.marketplace)}
								{licences}
								{set}
							/>
						{/each}

						{#if selected.length > 0 && overrideTab !== null}
							{@const where = overrideTab}
							{@const view = known.get(where)}
							{@const projection = projectionOf(draft, where, view ?? null)}
							<!-- What each marketplace will show, folded: most sellers
							     never change a value for one marketplace alone. -->
							<details class="flow-more res-shows">
								<summary>What each marketplace shows</summary>
								<TabBar tabs={overrideTabs} bind:current={tab} />
								{#each projection.rows.filter((row) => row.kind === 'field') as row (row.key)}
									{@const own = row.values[0]}
									{@const differs = row.decided_by?.by === 'listing_override'}
									<div class="res-group">
										<span class="res-group-label" id={`override-${row.key}`}>
											{row.label}
											{#if differs}<StatusPill label="changed here" />{/if}
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
													Your main listing still says “{canonicalValue(draft, row.key)}”.
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
													<StatusPill label="best match, if you turn it on" />
												{:else}
													<StatusPill label="you choose" />
												{/if}
											</span>
											<div class="res-note">
												{#if facts.stated.length > 0}
													You chose {facts.stated.join(', ')}.
												{/if}
												It goes in this marketplace's “{facts.native}” field.
												{#if facts.cap !== null}
													It takes up to {facts.cap}.
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
											This marketplace has no {axis} field, so any {axis} you add here won’t show
											there.
										</p>
									{/each}
								{/if}
							</details>
						{/if}
					</fieldset>
				</FlowStep>

				<FlowStep
					n={4}
					id="publish"
					title={editing === null ? 'Create' : 'Publish'}
					hint={editing === null
						? 'Choose how it starts, then create it.'
						: 'Save your changes, then publish them to your marketplaces.'}
					done={false}
					bind:open={openSteps.publish}
				>
					<fieldset class="res-lock" disabled={locked}>
						<StatusPanel {draft} {form} {refusals} {set} />
					</fieldset>

					{#if refusals.length > 0 || (serverCheck !== null && !serverCheck.submittable) || serverRefusal !== null}
						<section class="res-sec">
							{#if refusals.length > 0}
								<p class="res-errs-h">Click an error to go to it.</p>
								<ul class="res-errs">
									{#each refusals as refusal, index (`${refusal.group}-${index}`)}
										<li>
											<button type="button" class="btn small" onclick={() => void goTo(refusal.group)}>
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
				</FlowStep>

				{#if after}{@render after()}{/if}
			</div>


		<!-- Save and Publish where the hand is: stuck to the foot of the
		     screen while any step is in view, above the phone's tab bar. -->
		<div class="res-actbar">
			<!-- A phone leaves by its tab bar, so the bar gives Cancel's room to
			     the two actions. -->
			<span class="res-actbar-cancel"><Button href="/resources">Cancel</Button></span>
			<Button
				tier={publish === undefined ? 'primary' : 'outline'}
				type="submit"
				disabled={!canCreate}
				reason={blocking}
			>
				{#if creating}
					{editing === null ? 'Creating…' : 'Saving…'}
				{:else}
					{editing === null ? 'Create resource' : 'Save changes'}
				{/if}
			</Button>
			{#if publish}{@render publish()}{/if}
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
				Once your file is added, this listing can go to
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
