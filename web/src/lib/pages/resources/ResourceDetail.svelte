<script lang="ts">
	import { createQueries, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import {
		ApiFailure,
		api,
		type MappingHead,
		type VocabularyView
	} from '$lib/api';
	import {
		CURRENCY_OPTIONS,
		editBlockedBy,
		editSeedOf,
		measure,
		patchBodyOf,
		licenceOptions,
		type EditSeed
	} from '$lib/authoring';
	import { AUTHORABLE_PLATFORMS, platformTitle } from '$lib/platforms';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import DeleteDialog from '$lib/DeleteDialog.svelte';
	import { agoLabel } from '$lib/elapsed';
	import { createLedger, type Ledger } from '$lib/ledger';
	import {
		WORK_RUNS,
		chipFor,
		newestWork,
		runsFor,
		type MarketplaceChip,
		type RunRow,
		type WorkItem
	} from '$lib/inventory';
	import { formatPrice, rowStatus } from '$lib/listings-view';
	import Field from '$lib/Field.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import PublishDialog from '$lib/PublishDialog.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { connectionFor, readinessOf } from '$lib/publish-readiness';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import type { InventoryId } from '$lib/generated/vocab';
	import { PILL_TONE, fullStop } from './list';
	import { sentenceFor } from './refusal';
	import {
		IDLE,
		advance,
		busy,
		fileRefusal,
		LAST_FILE_LOSES_THUMBNAIL,
		ROLE_WORD,
		fileName,
		fileWords,
		reachSentence,
		redrawsCover,
		removable,
		replaceable,
		retiresCover,
		stalesCover,
		type FileAction,
		type FileVerb
	} from './files';
	import './resources.css';

	const queryClient = useQueryClient();
	const id = $derived(page.params.id ?? '');

	const product = createQuery(() => ({
		queryKey: queryKeys.product(id),
		queryFn: () => api.product(id),
		enabled: id.length > 0
	}));
	const allMappings = createQuery(() => ({
		queryKey: queryKeys.mappings,
		queryFn: () => api.mappings().then((view) => view.mappings)
	}));
	const connections = createQuery(() => ({
		queryKey: queryKeys.connections,
		queryFn: () => api.connections().then((view) => view.connections)
	}));
	const statuses = createQuery(() => ({
		queryKey: queryKeys.status,
		queryFn: () => api.status().then((view) => view.inventories)
	}));
	// What is happening to each of this listing's mappings, and which runs
	// carried it. Shares the inventory board's cache entry, because it is the
	// same bounded read of the newest runs' items.
	const work = createQuery(() => ({
		queryKey: queryKeys.inventoryWork,
		queryFn: async () => {
			const first = await api.jobs();
			const heads = first.jobs.slice(0, WORK_RUNS);
			const pages = await Promise.all(heads.map((head) => api.items(head.job)));
			const items: WorkItem[] = heads.flatMap((head, index) =>
				pages[index].items.map((item) => ({ job: head.job, item }))
			);
			return newestWork(items);
		}
	}));

	const mappings = $derived(
		(allMappings.data ?? []).filter((mapping: MappingHead) => mapping.product === id)
	);

	/** Whether the resource is actually absent, rather than unreadable. Only a
	 *  404 says so; every other failure is a fault on our side or in between. */
	const gone = $derived(product.error instanceof ApiFailure && product.error.status === 404);
	const inventories = $derived(mappings.map((mapping: MappingHead) => mapping.inventory));

	// The vocabularies, behind an object rather than as a bare `Map`.
	//
	// The wrapper is the whole fix: @tanstack/svelte-query 6.1.48 copies only
	// the own enumerable keys of whatever `combine` returns, so a `Map` reaches
	// the component as `{}` and the first `.get()` throws inside the row loop
	// below — the panel then reported "no marketplace mapping" for a resource
	// listed on two, which is what shipped before this redraw. `ResourceForm`
	// already reads its own vocabularies this way.
	//
	// The symptom is worth recording, because it does not point at the cause and
	// cost two of us an afternoon each. It presents as lost reactivity, not as a
	// thrown error: the same `$derived` reads two mappings at page scope and
	// none inside a panel's snippet, in one DOM snapshot, which no reactivity
	// rule explains. The throw is swallowed by the block that was rendering, so
	// what is left on screen is a stale-looking read. If a value that came
	// through `combine` ever seems not to update, check its shape before its
	// reactivity.
	//
	// The query list is the fixed authorable set rather than this resource's own
	// inventories, so it no longer depends on `mappings`: four entries, each
	// cached for the session and shared with the create form, and the unmapped
	// rows offer to cross-list onto exactly these.
	const vocabulary = createQueries(() => ({
		queries: AUTHORABLE_PLATFORMS.map((inventory: InventoryId) => ({
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

	const vocabularies = $derived(vocabulary.known);

	// The ledger the sync pages already read. A publish enqueues on the same
	// path every other write travels, so progress arrives here the same way:
	// an event moves a mapping, and the mappings are refetched rather than
	// patched from a payload this page would have to interpret.
	let live = $state(false);
	let ledger: Ledger | null = null;

	$effect(() => {
		ledger = createLedger((cursor) => new EventSource(`/v1/events/stream?cursor=${cursor}`));
		const unsubscribe = ledger.subscribe((state) => {
			live = state.connected;
			if (state.events.length > 0 || state.resyncs > 0) {
				void queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
			}
		});
		return () => {
			unsubscribe();
			ledger?.close();
		};
	});

	let seed = $state<EditSeed | null>(null);
	let saving = $state(false);
	let editRefusal = $state<string | null>(null);
	let publishing = $state(false);
	let deleting = $state(false);
	// The marketplace being added, so the row that started it is the one that
	// shows the wait, and a second click cannot start a second add.
	let adding = $state<InventoryId | null>(null);
	let addRefusal = $state<string | null>(null);
	// Which mapping is having a listing attached to it, and what the seller has
	// typed. One at a time: the form opens on the row it belongs to, so a URL
	// cannot be pasted against a marketplace the seller is not looking at.
	let attaching = $state<string | null>(null);
	let attachUrl = $state('');
	let attachRefusal = $state<string | null>(null);
	let attachSending = $state(false);
	// Runs started from this page in this session, kept only until the bounded
	// read below carries them: a run the server has not listed yet would
	// otherwise vanish between starting it and the refetch landing.
	let justStarted = $state<{ inventory: InventoryId; job: string }[]>([]);

	// Seeded once per resource rather than mirrored: a refetch arriving while the
	// seller is typing must not overwrite what they typed, and a move from one
	// resource to the next must not leave the first one's fields in the form —
	// SvelteKit keeps one component across a change of `[id]`, so without the
	// second guard a save would write this resource's title onto that one.
	let seededFor = $state<string | null>(null);
	$effect(() => {
		const stored = product.data;
		if (stored !== undefined && (seed === null || seededFor !== stored.id)) {
			seed = editSeedOf(stored);
			seededFor = stored.id;
		}
	});

	const chips = $derived.by(() => {
		const found = new Map<InventoryId, MarketplaceChip>();
		for (const mapping of mappings) {
			found.set(
				mapping.inventory,
				chipFor({
					product: id,
					inventory: mapping.inventory,
					mapping,
					work: (work.data ?? new Map()).get(mapping.id),
					connection: connectionFor(mapping.inventory, connections.data ?? []),
					status: (statuses.data ?? []).find((one) => one.inventory === mapping.inventory)
				})
			);
		}
		return found;
	});

	// The runs the bounded read knows about, and any this page started since,
	// newest first. A run appearing in both is one row: the read is the
	// authority and the local note only fills the gap before it lands.
	const runs = $derived.by(() => {
		const known: RunRow[] = runsFor(mappings, work.data ?? new Map());
		const seen = new Set(known.map((row) => row.job));
		return [
			...justStarted
				.filter((run) => !seen.has(run.job))
				.map((run) => ({ job: run.job, inventory: run.inventory, state: null })),
			...known.map((row) => ({ job: row.job, inventory: row.inventory, state: row.state }))
		];
	});

	// The marketplaces this console authors for that this resource is not mapped
	// onto. Rendered as a disabled control rather than omitted, so the one thing
	// a seller most wants to do next is visible and the reason it cannot be done
	// is stated where they look for it.
	const unmapped = $derived(
		AUTHORABLE_PLATFORMS.filter(
			(inventory: InventoryId) => !inventories.includes(inventory)
		)
	);

	const needing = $derived([...chips.values()].filter((chip) => chip.action !== null && chip.tone === 'bad'));

	const blockedBy = $derived(editBlockedBy(mappings));
	const licensing = $derived(
		inventories.filter(
			(inventory: InventoryId) => vocabularies.get(inventory)?.authoring.licence !== undefined
		)
	);
	const licenceChoices = $derived(
		seed === null || licensing.length === 0
			? []
			: licenceOptions(vocabularies.get(licensing[0]), seed.branch)
	);
	const payloadFiles = $derived(
		(product.data?.files ?? []).filter((file) => file.role === 'payload').length
	);
	const hasRights = $derived(product.data?.rights !== undefined);
	const bodyCap = $derived.by(() => {
		const caps = inventories
			.map(
				(inventory: InventoryId) =>
					vocabularies.get(inventory)?.canonical.find((entry) => entry.field === 'description')
						?.cap
			)
			.filter((cap) => cap !== undefined);
		return caps.length === 0 ? null : caps.reduce((a, b) => (a.limit <= b.limit ? a : b));
	});

	async function save(event: SubmitEvent) {
		event.preventDefault();
		if (seed === null || blockedBy.length > 0) {
			return;
		}
		const body = patchBodyOf(seed, licensing[0] ?? product.data?.rights?.inventory ?? null);
		if (body === null) {
			editRefusal = 'A paid price is a positive amount, written in the currency’s own units.';
			return;
		}
		saving = true;
		editRefusal = null;
		try {
			const patched = await api.patchProduct(id, body);
			await queryClient.invalidateQueries({ queryKey: queryKeys.product(id) });
			await queryClient.invalidateQueries({ queryKey: queryKeys.products });
			toast(
				'info',
				patched.reaches.length === 0
					? 'Saved. This listing is on no marketplace yet.'
					: `Saved. Reaches ${patched.reaches.map(platformTitle).join(', ')} on the next send.`
			);
		} catch (failure) {
			editRefusal = editRefusalOf(failure);
		} finally {
			saving = false;
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

	function published(started: { inventory: InventoryId; job: string }[]) {
		publishing = false;
		justStarted = [...started, ...justStarted];
		void queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
		void queryClient.invalidateQueries({ queryKey: queryKeys.inventoryWork });
		if (started.length === 1) {
			toast('info', 'Send started.');
			void goto(`/sync/${started[0].job}`);
			return;
		}
		toast('info', `Send started on ${started.length} marketplaces.`);
	}

	/** Adds a marketplace this resource does not reach, then opens the send it is
	 *  the first half of. The add writes the catalogue and contacts nobody; how
	 *  the listing is sent, and whether it goes live, stays the seller's choice
	 *  on the dialog rather than one this control makes for them. */
	async function crossListTo(inventory: InventoryId) {
		adding = inventory;
		addRefusal = null;
		try {
			await api.addMapping(id, inventory);
			await queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
			toast('info', `${platformTitle(inventory)} added. Choose how it is sent.`);
			publishing = true;
		} catch (failure) {
			addRefusal =
				failure instanceof ApiFailure
					? sentenceFor(failure, `${platformTitle(inventory)} was not added.`)
					: `${platformTitle(inventory)} was not added.`;
		} finally {
			adding = null;
		}
	}

	function openAttach(mapping: string) {
		attaching = mapping;
		attachUrl = '';
		attachRefusal = null;
	}

	/** Attaches a listing that already exists on the marketplace to this resource.
	 *  A catalogue write and nothing else: no marketplace is contacted, so the
	 *  binding is a claim the next sync reads back rather than a verified
	 *  fact. */
	async function attach(mapping: string) {
		if (attachUrl.trim().length === 0) {
			return;
		}
		attachSending = true;
		attachRefusal = null;
		try {
			await api.bindMapping(mapping, attachUrl.trim());
			await queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
			attaching = null;
			attachUrl = '';
			toast('info', 'Listing attached. The next sync reads it back.');
		} catch (failure) {
			attachRefusal =
				failure instanceof ApiFailure
					? sentenceFor(failure, 'That listing could not be attached.')
					: 'That listing could not be attached.';
		} finally {
			attachSending = false;
		}
	}

	// The Files panel's own state. One action at a time, so a seller cannot
	// remove the file an upload is in the middle of replacing; the machine in
	// `files.ts` is what holds that, and it is tested there.
	let fileAction = $state<FileAction>(IDLE);
	// What a file change reaches, kept from the last one so the panel can say
	// where it landed rather than only that it landed.
	let fileReach = $state<string | null>(null);
	let picking: HTMLInputElement | null = $state(null);
	// Which row the picker was opened for, and `null` for the add. Held here
	// rather than in the machine because it outlives the confirmation: the
	// picker fires long after the control was pressed.
	let pickingFor: string | null = null;
	let pickingVerb: 'add' | 'replace' = 'replace';
	let chosen: File | null = null;

	const storedFiles = $derived(product.data?.files ?? []);

	function fileEvent(event: Parameters<typeof advance>[1]) {
		fileAction = advance(fileAction, event);
	}

	/** Opens the file picker for one row, or for the add. Nothing is destroyed
	 *  yet: a replacement asks for confirmation once the file is chosen, so the
	 *  question can name both files rather than only the one being lost. */
	function choose(verb: 'add' | 'replace', file: string | null) {
		if (busy(fileAction)) {
			return;
		}
		pickingVerb = verb;
		pickingFor = file;
		chosen = null;
		picking?.click();
	}

	function picked(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0] ?? null;
		// Cleared so choosing the same file twice in a row still fires a change.
		input.value = '';
		if (file === null) {
			return;
		}
		chosen = file;
		if (pickingVerb === 'add') {
			void sendFile('add', null, file);
			return;
		}
		fileEvent({
			kind: 'ask',
			verb: 'replace',
			file: pickingFor ?? '',
			chosen: file.name
		});
	}

	/** Uploads the chosen bytes, then names the handle back. `keep_whole`
	 *  because one replacement is one file: `explode` would answer with a
	 *  handle per entry for an archive, and the row takes exactly one. */
	async function sendFile(verb: 'add' | 'replace', file: string | null, bytes: File) {
		fileEvent({ kind: 'send', verb, file });
		try {
			const uploaded = await api.upload(bytes, 'keep_whole', (fraction) =>
				fileEvent({ kind: 'progress', fraction })
			);
			fileEvent({ kind: 'stored' });
			const stored = uploaded.payload[0];
			if (stored === undefined) {
				fileEvent({ kind: 'failed', sentence: 'That upload carried no file we can store.' });
				return;
			}
			// The name is the client's word: the upload sends a raw body, which
			// carries no filename, so it travels back beside the handle.
			const handle = { ...stored, name: bytes.name };
			if (verb === 'add') {
				const added = await api.addProductFile(id, 'payload', handle);
				await settleFile(added.reaches, 'File added.');
				return;
			}
			// No thumbnail is sent: the server renders one from these bytes
			// where it decides a redraw is due, and reports what it drew. A
			// handle offered from here would be a handle a client could point
			// at any file, which is the hole that closed by removing the field.
			const replaced = await api.replaceProductFile(id, file ?? '', handle);
			await settleFile(
				replaced.reaches,
				replaced.cover === undefined ? 'File replaced.' : 'File replaced, thumbnail redrawn.'
			);
		} catch (failure) {
			fileEvent({ kind: 'failed', sentence: fileRefusal(failure, verb) });
		}
	}

	async function removeFile(file: string) {
		fileEvent({ kind: 'send', verb: 'remove', file });
		try {
			const landed = await api.removeProductFile(id, file);
			await settleFile(
				landed.reaches,
				landed.thumbnail.state === 'redrawn'
					? 'File removed, thumbnail redrawn.'
					: landed.thumbnail.state === 'retired'
						? 'File removed, and the thumbnail with it.'
						: 'File removed.'
			);
		} catch (failure) {
			fileEvent({ kind: 'failed', sentence: fileRefusal(failure, 'remove') });
		}
	}

	async function settleFile(reaches: InventoryId[], said: string) {
		fileReach = reachSentence(reaches);
		fileEvent({ kind: 'settled' });
		await queryClient.invalidateQueries({ queryKey: queryKeys.product(id) });
		await queryClient.invalidateQueries({ queryKey: queryKeys.products });
		toast('info', said);
	}

	function confirmFile() {
		if (fileAction.kind !== 'confirming') {
			return;
		}
		const { verb, file } = fileAction;
		if (verb === 'remove') {
			void removeFile(file);
			return;
		}
		if (chosen !== null) {
			void sendFile('replace', file, chosen);
		}
	}

	async function deleted() {
		deleting = false;
		await queryClient.invalidateQueries({ queryKey: queryKeys.products });
		await queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
		toast('info', 'Listing deleted.');
		await goto('/inventory');
	}
</script>

<div class="page resources-page">
	{#if product.isPending || allMappings.isPending}
		<p class="res-note">Loading the resource…</p>
	{:else if gone}
		<PageHead icon="layout-list" title="Resource" description="This resource is not here." />
		<Placeholder
			icon="search"
			headline="No such resource"
			body="It may have been deleted. The catalogue still has everything else."
		>
			{#snippet actions()}
				<Button href="/inventory" icon="layout-list">Back to Resources</Button>
			{/snippet}
		</Placeholder>
	{:else if product.isError || product.data === undefined}
		<!-- A fault is not a deletion. Only a 404 says the resource is gone; a
		     500, a dropped connection or the retry backoff all reach here, and
		     telling a seller their resource was deleted over a bad minute is the
		     worse of the two wrong answers. -->
		<PageHead icon="layout-list" title="Resource" description="This resource could not be read." />
		<Placeholder
			icon="triangle-alert"
			headline="This resource could not be read"
			body="Nothing has happened to it; the reading failed. Reload to try again."
		>
			{#snippet actions()}
				<Button href="/inventory" icon="layout-list">Back to Resources</Button>
			{/snippet}
		</Placeholder>
	{:else if allMappings.isError}
		<!-- Which marketplaces carry this resource is a separate read, and every
		     control below depends on it: without it the panel reports no mapping at
		     all and offers to cross-list onto marketplaces the resource is already
		     listed on, and the edit form offers to save a live listing it cannot
		     know is live. -->
		<PageHead
			icon="layout-list"
			title={product.data.title}
			description={`${formatPrice(product.data.price)} · updated ${agoLabel(product.data.updated_at, Date.now())}`}
		/>
		<Placeholder
			icon="triangle-alert"
			headline="Which marketplaces carry this resource could not be read"
			body="Nothing below would be true without it, so nothing below is shown. Reload to try again."
		>
			{#snippet actions()}
				<Button href="/inventory" icon="layout-list">Back to Resources</Button>
			{/snippet}
		</Placeholder>
	{:else}
		{@const stored = product.data}
		{@const status = rowStatus(mappings)}
		<PageHead
			icon="layout-list"
			title={stored.title}
			description={`${formatPrice(stored.price)} · updated ${agoLabel(stored.updated_at, Date.now())}`}
		>
			{#snippet aside()}
				<!-- Actions only. The two badges that used to sit here moved into the
				     Marketplaces panel, which is what they describe: four unshrinkable
				     pills in a row that does not wrap ran off the right edge of a
				     390px viewport, and the specification's header band carries
				     right-aligned page actions and no badges. -->
				<Button tier="primary" onclick={() => (publishing = true)}>Publish…</Button>
				<Button danger onclick={() => (deleting = true)}>Delete…</Button>
			{/snippet}
		</PageHead>

		{#if needing.length > 0}
			<Banner
				tone="warn"
				title={`${needing.length} ${needing.length === 1 ? 'marketplace needs' : 'marketplaces need'} you`}
			>
				{needing.map((chip) => platformTitle(chip.inventory)).join(', ')}. Nothing is sent to
				{needing.length === 1 ? 'it' : 'them'} until this is cleared, and the listing already there
				stands where it stood.
				{#snippet action()}
					{#if needing[0].action}
						<Button href={needing[0].action.href}>{needing[0].action.label}</Button>
					{/if}
				{/snippet}
			</Banner>
		{/if}

		<Panel
			title="Marketplaces"
			description="Where this resource stands on each one, and what it is waiting on. A marketplace the resource was not created with can be added here."
		>
			{#snippet more()}
				<!-- Where the listing stands overall, and whether this page is being
				     told about changes as they happen. Two different facts, so the
				     second says "connected" rather than "live": side by side in one
				     tone they read as one fact stated twice. -->
				<span class="panel-badges">
					<StatusPill tone={PILL_TONE[status.tone]} label={status.label} />
					<StatusPill tone={live ? 'ok' : 'soon'} label={live ? 'connected' : 'reconnecting'} />
				</span>
			{/snippet}
			{#each mappings as mapping (mapping.id)}
				{@const chip = chips.get(mapping.inventory)}
				{@const verdict = readinessOf({
					inventory: mapping.inventory,
					intent: 'draft',
					mapping,
					connection: connectionFor(mapping.inventory, connections.data ?? []),
					status: (statuses.data ?? []).find((one) => one.inventory === mapping.inventory),
					vocabulary: vocabularies.get(mapping.inventory),
					payloadFiles,
					hasRights
				})}
				<div class="mk-row">
					<div class="mk-head">
						<span class="mk-name">{verdict.title}</span>
						{#if chip}
							<StatusPill tone={PILL_TONE[chip.tone]} label={chip.label} />
						{:else}
							<StatusPill tone={PILL_TONE[verdict.tone]} label={verdict.line} />
						{/if}
					</div>
					<p class="mk-say">
						{chip?.detail ?? `${mapping.binding_state} · ${mapping.lifecycle_state}`}
					</p>
					{#if chip?.paused}
						<p class="mk-say">Sending is paused here: {chip.paused}</p>
					{/if}
					<p class="mk-say">
						<!-- `verdict.line` ends in its own punctuation for some verdicts and
						     not others, so the sentence is closed only when it is open. -->
						Next send: {verdict.line}{fullStop(verdict.line)}
						{#if chip?.onDevice}
							Work for this marketplace runs on your own device.
						{/if}
					</p>
					<div class="mk-acts">
						{#if chip?.action}
							<!-- An anchor rather than the button tier, because a listed chip's
							     action leaves the console and has to carry `rel` with it. -->
							<a
								class="res-out"
								href={chip.action.href}
								target={chip.action.external ? '_blank' : undefined}
								rel={chip.action.external ? 'noopener noreferrer' : undefined}
							>{chip.action.label}</a>
						{/if}
						{#if mapping.binding_state === 'unbound'}
							<Button
								small
								disabled={attachSending}
								reason={attachSending ? 'A listing is being attached.' : undefined}
								onclick={() =>
									attaching === mapping.id ? (attaching = null) : openAttach(mapping.id)}
							>
								{attaching === mapping.id ? 'Cancel' : 'Mark as listed'}
							</Button>
						{/if}
					</div>

					{#if attaching === mapping.id}
						<div class="mk-attach">
							<Field
								label="The listing's address"
								id={`attach-${mapping.id}`}
								hint={`Nothing is sent to ${platformTitle(mapping.inventory)}; this records where the listing already is, so later edits reach it.`}
							>
								<input
									id={`attach-${mapping.id}`}
									type="url"
									placeholder="https://…"
									disabled={attachSending}
									bind:value={attachUrl}
								/>
							</Field>
							<Button
								tier="primary"
								disabled={attachSending || attachUrl.trim().length === 0}
								reason={attachUrl.trim().length === 0
									? 'Paste the listing address first.'
									: undefined}
								onclick={() => void attach(mapping.id)}
							>
								{attachSending ? 'Attaching…' : 'Attach'}
							</Button>
						</div>
						{#if attachRefusal !== null}
							<Banner tone="bad">{attachRefusal}</Banner>
						{/if}
					{/if}
				</div>
			{:else}
				<p class="res-note">This resource carries no marketplace mapping.</p>
			{/each}
			{#each unmapped as inventory (inventory)}
				<div class="mk-row">
					<div class="mk-head">
						<span class="mk-name">{platformTitle(inventory)}</span>
						<StatusPill label="Not listed" />
					</div>
					<p class="mk-say">Not listed here, and this resource has no mapping onto it.</p>
					<div class="mk-acts">
						<Button
							small
							disabled={adding !== null}
							reason={adding !== null ? 'A marketplace is being added.' : undefined}
							onclick={() => void crossListTo(inventory)}
						>
							{adding === inventory ? 'Adding…' : 'Cross-list here'}
						</Button>
					</div>
				</div>
			{/each}
			{#if addRefusal !== null}
				<Banner tone="bad">{addRefusal}</Banner>
			{/if}
			<p class="res-foot">
				Cross-listing here adds the marketplace to this resource and opens the send; the add writes
				your own catalogue and contacts nobody, and nothing reaches the marketplace until that
				send runs. Etsy is not listed here, because this console has no create path for it at
				all.
			</p>
		</Panel>

		{#if runs.length > 0}
			<Panel
				title="Sends"
				description="Every run in the recent window that carried this resource. Each opens its own timeline, with the steps, gates and events it recorded."
			>
				{#each runs as run (run.job)}
					<a class="res-line" href={`/sync/${run.job}`}>
						<span class="res-line-what">
							<span class="res-line-t">{platformTitle(run.inventory)}</span>
							<span class="res-line-id">{run.job.slice(0, 8)}…</span>
						</span>
						<span class="res-line-at">{run.state === null ? 'just started' : run.state}</span>
					</a>
				{/each}
				<p class="res-foot">
					Only the newest runs are read, so a send older than that window is not listed here.
					Sync holds every run.
				</p>
			</Panel>
		{/if}

		{#if blockedBy.length > 0}
			<Banner tone="warn" title="This resource is live and cannot be edited through us">
				{blockedBy.map(platformTitle).join(', ')} has a published listing, and neither editing a
				published listing nor taking one back to draft is a transition we have captured there. The
				fields below are shown as stored and the edit is held back rather than sent and refused.
			</Banner>
		{/if}

		{#if seed}
			{@const current = seed}
			<form class="res-form" onsubmit={save}>
				<Panel
					title="The resource"
					description="The canonical fields. Every marketplace this resource is on takes them from here."
				>
					<Field label="Title" id="edit-title" required>
						<input
							id="edit-title"
							type="text"
							required
							maxlength="500"
							disabled={blockedBy.length > 0}
							value={current.title}
							oninput={(event) => (seed = { ...current, title: event.currentTarget.value })}
						/>
					</Field>

					<Field label="Description" id="edit-body">
						{#if bodyCap}
							{@const used = measure(current.body, bodyCap.unit)}
							<span class="res-count" class:over={used > bodyCap.limit}>
								{used} of {bodyCap.limit}
							</span>
						{/if}
						<textarea
							id="edit-body"
							disabled={blockedBy.length > 0}
							value={current.body}
							oninput={(event) => (seed = { ...current, body: event.currentTarget.value })}
						></textarea>
					</Field>

					<fieldset class="res-choices">
						<legend>Written as</legend>
						{#each ['Markdown', 'Html'] as const as format (format)}
							<label>
								<input
									type="radio"
									name="edit-body-format"
									disabled={blockedBy.length > 0}
									checked={current.bodyFormat === format}
									onchange={() => (seed = { ...current, bodyFormat: format })}
								/>
								{format}
							</label>
						{/each}
					</fieldset>

					<fieldset class="res-choices">
						<legend>Price</legend>
						<label>
							<input
								type="radio"
								name="edit-price-branch"
								disabled={blockedBy.length > 0}
								checked={current.branch === 'free'}
								onchange={() => (seed = { ...current, branch: 'free' })}
							/>
							Free
						</label>
						<label>
							<input
								type="radio"
								name="edit-price-branch"
								disabled={blockedBy.length > 0}
								checked={current.branch === 'paid'}
								onchange={() => (seed = { ...current, branch: 'paid' })}
							/>
							Paid
						</label>
					</fieldset>

					{#if current.branch === 'paid'}
						<div class="res-row">
							<Field label="Amount" id="edit-amount" required>
								<input
									id="edit-amount"
									type="text"
									inputmode="decimal"
									placeholder="4.50"
									disabled={blockedBy.length > 0}
									value={current.amount}
									oninput={(event) => (seed = { ...current, amount: event.currentTarget.value })}
								/>
							</Field>
							<Field label="Currency" id="edit-currency" required>
								<select
									id="edit-currency"
									disabled={blockedBy.length > 0}
									value={current.currency}
									onchange={(event) =>
										(seed = { ...current, currency: event.currentTarget.value })}
								>
									{#each CURRENCY_OPTIONS as option (option.value)}
										<option value={option.value}>{option.code}</option>
									{/each}
								</select>
							</Field>
						</div>
					{/if}

					{#if licensing.length > 0}
						<Field
							label="Licence"
							id="edit-licence"
							hint="Gated on the price, so changing free or paid changes this list. A stated licence can be replaced here but not withdrawn."
						>
							<select
								id="edit-licence"
								disabled={blockedBy.length > 0}
								value={current.licence ?? ''}
								onchange={(event) =>
									(seed = {
										...current,
										licence: event.currentTarget.value === '' ? null : event.currentTarget.value
									})}
							>
								<option value="">Leave as stored</option>
								{#each licenceChoices as choice (choice.id)}
									<option value={choice.id}>{choice.label}</option>
								{/each}
							</select>
						</Field>
					{/if}

					{#if editRefusal !== null}
						<Banner tone="bad">{editRefusal}</Banner>
					{/if}

					<div class="res-acts">
						<Button
							tier="primary"
							type="submit"
							disabled={saving || blockedBy.length > 0}
							reason={blockedBy.length > 0
								? 'A published listing cannot be edited through us.'
								: saving
									? 'The change is being saved.'
									: undefined}
						>
							{saving ? 'Saving…' : 'Save changes'}
						</Button>
						<span class="res-note">
							Saving writes the catalogue. It reaches a marketplace on the next send.
						</span>
					</div>
				</Panel>
			</form>
		{/if}

		<Panel
			title="Files"
			description="What a buyer downloads, plus the thumbnail we draw from the first file."
		>
			{#each stored.files as file (file.id)}
				{@const may = removable(storedFiles, file.id, inventories.length > 0)}
				{@const swappable = replaceable(storedFiles, file.id)}
				{@const mine = fileAction.kind !== 'idle' && fileAction.file === file.id}
				<div class="res-line res-file">
					<span class="res-line-what">
						<StatusPill tone="flat" label={ROLE_WORD[file.role]} />
						<span class="res-line-t" class:res-file-anon={file.name === undefined}
							>{fileName(file)}</span
						>
						<span class="res-line-id">{file.kind} · {file.scan}</span>
					</span>
					<span class="res-line-at">{file.byte_len.toLocaleString('en-GB')} bytes</span>
					<span class="res-file-acts">
						<Button
							small
							disabled={busy(fileAction) || !swappable.ok}
							reason={busy(fileAction)
								? 'One file change runs at a time.'
								: swappable.ok
									? undefined
									: swappable.reason}
							onclick={() => choose('replace', file.id)}>Replace…</Button
						>
						<Button
							small
							danger
							disabled={busy(fileAction) || !may.ok}
							reason={busy(fileAction)
								? 'One file change runs at a time.'
								: may.ok
									? undefined
									: may.reason}
							onclick={() => fileEvent({ kind: 'ask', verb: 'remove', file: file.id, chosen: null })}
							>Remove…</Button
						>
					</span>
					{#if mine && fileAction.kind === 'uploading'}
						<p class="res-file-say" role="status">
							Replacing {fileWords(file)}… {Math.round(fileAction.fraction * 100)}% sent
						</p>
					{:else if mine && fileAction.kind === 'writing'}
						<p class="res-file-say" role="status">Saving…</p>
					{:else if mine && fileAction.kind === 'confirming'}
						<div class="res-file-ask">
							<p class="res-file-say">
								{#if fileAction.verb === 'remove'}
									Remove {fileWords(file)} from this resource?
									{#if retiresCover(storedFiles, file.id)}
										{LAST_FILE_LOSES_THUMBNAIL}
									{:else if stalesCover(storedFiles, file.id)}
										The thumbnail is drawn from this file, so it is redrawn from the next one.
									{/if}
								{:else}
									Replace {fileWords(file)} with {fileAction.chosen}?
									{#if redrawsCover(storedFiles, file.id)}
										The thumbnail is drawn from this file, so it is redrawn from the new one.
									{/if}
								{/if}
								{reachSentence(inventories)} Removing or replacing a file does not reclaim storage: the
								file itself stays in your storage either way.
							</p>
							<span class="res-file-acts">
								<Button small tier="primary" onclick={confirmFile}>
									{fileAction.verb === 'remove' ? 'Remove it' : 'Replace it'}
								</Button>
								<Button small onclick={() => fileEvent({ kind: 'cancel' })}>Keep it</Button>
							</span>
						</div>
					{:else if mine && fileAction.kind === 'refused'}
						<p class="res-file-say res-file-no" role="alert">{fileAction.sentence}</p>
					{/if}
				</div>
			{:else}
				<p class="res-note">No files recorded.</p>
			{/each}

			<div class="res-file-add">
				<Button
					small
					icon="plus"
					disabled={busy(fileAction)}
					reason={busy(fileAction) ? 'One file change runs at a time.' : undefined}
					onclick={() => choose('add', null)}>Add file</Button
				>
				{#if fileAction.kind === 'uploading' && fileAction.file === null}
					<span class="res-file-say" role="status"
						>Sending… {Math.round(fileAction.fraction * 100)}%</span
					>
				{:else if fileAction.kind === 'writing' && fileAction.file === null}
					<span class="res-file-say" role="status">Saving…</span>
				{:else if fileAction.kind === 'refused' && fileAction.file === null}
					<span class="res-file-say res-file-no" role="alert">{fileAction.sentence}</span>
				{/if}
			</div>

			<!-- Off-screen rather than hidden: a display:none input cannot be
			     clicked open in every browser, and this one is opened by script. -->
			<input
				class="res-file-pick"
				type="file"
				bind:this={picking}
				onchange={picked}
				tabindex="-1"
				aria-hidden="true"
			/>

			<p class="res-foot">
				<span class="block"
					>Changing a file here changes your Resources. {fileReach ??
						reachSentence(inventories)}</span
				>
				<span class="block"
					>The thumbnail is drawn from the first file. Replacing that file redraws it; replacing any
					other leaves it alone.</span
				>
			</p>
		</Panel>

		<PublishDialog
			open={publishing}
			{mappings}
			connections={connections.data ?? []}
			statuses={statuses.data ?? []}
			{vocabularies}
			{payloadFiles}
			{hasRights}
			onClose={() => (publishing = false)}
			onPublished={published}
		/>
		<DeleteDialog
			open={deleting}
			product={id}
			title={stored.title}
			{mappings}
			onClose={() => (deleting = false)}
			onDeleted={deleted}
		/>
	{/if}
</div>
