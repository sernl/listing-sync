<script lang="ts">
	import { createQueries, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import {
		ApiFailure,
		api,
		collectionsApi,
		type MappingHead,
		type VocabularyView
	} from '$lib/api';
	import { editBlockedBy } from '$lib/authoring';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
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
		stripFor,
		type MarketplaceChip,
		type RunRow,
		type WorkItem
	} from '$lib/inventory';
	import { formatPrice, rowStatus } from '$lib/listings-view';
	import LabelChip from '$lib/LabelChip.svelte';
	import Field from '$lib/Field.svelte';
	import Icon from '$lib/Icon.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import MarketplaceTile from './MarketplaceTile.svelte';
	import { tileFor } from './marketplace-tile';
	import ResourceForm from './ResourceForm.svelte';
	import PublishDialog from '$lib/PublishDialog.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { connectionFor, readinessOf } from '$lib/publish-readiness';
	import { queryKeys } from '$lib/query';
	import { COLLECTIONS_KEY, countLine } from '$lib/pages/collections/collections';
	import { toast } from '$lib/toast';
	import type { InventoryId } from '$lib/generated/vocab';
	import { PILL_TONE } from './list';
	import { sentenceFor } from './refusal';
	import './resources.css';

	const queryClient = useQueryClient();
	const id = $derived(page.params.id ?? '');

	const product = createQuery(() => ({
		queryKey: queryKeys.product(id),
		queryFn: () => api.product(id),
		enabled: id.length > 0
	}));
	// What this resource is filed under, including the mark an import wrote on
	// it. Its own read rather than a field of the product: the labels route is
	// what carries `system`, and the aggregate does not.
	const labels = createQuery(() => ({
		queryKey: queryKeys.productLabels(id),
		queryFn: () => api.productLabels(id).then((view) => view.labels),
		enabled: id.length > 0
	}));
	// Which collections pick this resource. Keyed by the product, and nested
	// under the collections prefix so a membership written on the Collections
	// page moves this panel on the next read.
	const collections = createQuery(() => ({
		queryKey: [...COLLECTIONS_KEY, 'of-product', id],
		queryFn: () => collectionsApi.forProduct(id).then((view) => view.collections),
		enabled: id.length > 0
	}));
	const allMappings = createQuery(() => ({
		queryKey: queryKeys.mappings,
		queryFn: () => api.mappings().then((view) => view.mappings)
	}));
	const connections = createQuery(() => ({
		queryKey: queryKeys.connections,
		queryFn: () => api.connections()
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
	// cached for the session and shared with the create form, and the tiles for
	// marketplaces this resource does not reach offer to cross-list onto
	// exactly these.
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
		let revision = 0;
		const unsubscribe = ledger.subscribe((state) => {
			live = state.connected;
			if (state.revision === revision) return;
			revision = state.revision;
			if ([...state.kinds].some((kind) => kind === 'resync' || kind.startsWith('Job') || kind.startsWith('Item') || kind === 'ImportCompleted')) {
				void queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
			}
		});
		return () => {
			unsubscribe();
			ledger?.close();
		};
	});

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

	// One strip rather than two loops. `stripFor` answers every marketplace this
	// console authors for plus any this resource is already mapped onto, which
	// is exactly the union the page used to build from a loop over the mappings
	// and a second over what was left; a marketplace with no mapping reaches
	// `chipFor` as an absent one and gets its own "not listed" chip there.
	const chips = $derived.by(() => {
		const mapped = new Map(mappings.map((mapping: MappingHead) => [mapping.inventory, mapping]));
		return stripFor(mappings).map((inventory: InventoryId) => {
			const mapping = mapped.get(inventory);
			return chipFor({
				product: id,
				inventory,
				mapping,
				work: mapping === undefined ? undefined : (work.data ?? new Map()).get(mapping.id),
				connection: connectionFor(inventory, connections.data ?? []),
				status: (statuses.data ?? []).find((one) => one.inventory === inventory)
			});
		});
	});

	// The tiles the panel draws. The readiness verdict is passed only for a
	// marketplace this resource actually reaches: for one it does not, the
	// verdict is the sentence saying so, which the tile already says in its own
	// words.
	const tiles = $derived.by(() => {
		const mapped = new Map(mappings.map((mapping: MappingHead) => [mapping.inventory, mapping]));
		const now = Date.now();
		return chips.map((chip: MarketplaceChip) => {
			const mapping = mapped.get(chip.inventory);
			return tileFor({
				chip,
				mapping,
				product: id,
				now,
				readiness:
					mapping === undefined
						? undefined
						: readinessOf({
								inventory: chip.inventory,
								intent: 'draft',
								mapping,
								connection: connectionFor(chip.inventory, connections.data ?? []),
								status: (statuses.data ?? []).find((one) => one.inventory === chip.inventory),
								vocabulary: vocabularies.get(chip.inventory),
								payloadFiles,
								hasRights
							})
			});
		});
	});

	/** The mapping whose attach form is open and the marketplace it reaches, so
	 *  the one form the page draws names what it writes against. */
	const attachingTo = $derived.by(() => {
		const open = attaching;
		if (open === null) {
			return undefined;
		}
		const tile = tiles.find((one) => one.mapping === open);
		return tile === undefined ? undefined : { mapping: open, inventory: tile.inventory };
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

	const needing = $derived(
		chips.filter((chip: MarketplaceChip) => chip.action !== null && chip.tone === 'bad')
	);

	const blockedBy = $derived(editBlockedBy(mappings));
	const payloadFiles = $derived(
		(product.data?.files ?? []).filter((file) => file.role === 'payload').length
	);
	const hasRights = $derived(product.data?.rights !== undefined);

	function published(started: { inventory: InventoryId; job: string }[]) {
		publishing = false;
		justStarted = [...started, ...justStarted];
		void queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
		void queryClient.invalidateQueries({ queryKey: queryKeys.inventoryWork });
		if (started.length === 1) {
			toast('info', 'Sending started.');
			void goto(`/sync/${started[0].job}`);
			return;
		}
		toast('info', `Started sending to ${started.length} marketplaces.`);
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
			toast('info', `${platformTitle(inventory)} added. Now choose how to send it.`);
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

	/** Opens the attach form on this mapping, or closes the one already open on
	 *  it. Only a tile carrying a mapping offers the control at all, so the
	 *  absent case is unreachable from the panel and refuses rather than
	 *  guessing which mapping was meant. */
	function toggleAttach(mapping: string | null) {
		if (mapping === null) {
			return;
		}
		if (attaching === mapping) {
			attaching = null;
			return;
		}
		openAttach(mapping);
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
			toast('info', 'Listing attached. Teachouse checks it on its next update.');
		} catch (failure) {
			attachRefusal =
				failure instanceof ApiFailure
					? sentenceFor(failure, 'That listing could not be attached.')
					: 'That listing could not be attached.';
		} finally {
			attachSending = false;
		}
	}

	async function deleted() {
		deleting = false;
		await queryClient.invalidateQueries({ queryKey: queryKeys.products });
		await queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
		toast('info', 'Resource deleted.');
		await goto('/resources');
	}
</script>

<div class="page resources-page flow-page">
	{#if product.isPending || allMappings.isPending}
		<p class="res-note">Loading this resource…</p>
	{:else if gone}
		<PageHead icon="layout-list" title="Resource" description="We can’t find this resource." />
		<Placeholder
			icon="search"
			headline="Resource not found"
			body="It may have been deleted."
		>
			{#snippet actions()}
				<Button href="/resources" icon="layout-list">Back to Resources</Button>
			{/snippet}
		</Placeholder>
	{:else if product.isError || product.data === undefined}
		<!-- A fault is not a deletion. Only a 404 says the resource is gone; a
		     500, a dropped connection or the retry backoff all reach here, and
		     telling a seller their resource was deleted over a bad minute is the
		     worse of the two wrong answers. -->
		<PageHead icon="layout-list" title="Resource" description="This resource didn’t load." />
		<Placeholder
			icon="triangle-alert"
			headline="This resource didn’t load"
			body="Reload to try again."
		>
			{#snippet actions()}
				<Button href="/resources" icon="layout-list">Back to Resources</Button>
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
			headline="We couldn’t load where this resource is listed"
			body="Reload to try again."
		>
			{#snippet actions()}
				<Button href="/resources" icon="layout-list">Back to Resources</Button>
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
				<!-- Publish is beside Save in the form's sticky bar, where the hand
				     is after an edit; only the destructive action stays up here. -->
				<Button danger icon="trash-2" onclick={() => (deleting = true)}>Delete…</Button>
			{/snippet}
		</PageHead>

		{#if needing.length > 0}
			<Banner
				tone="warn"
				title={`${needing.length} ${needing.length === 1 ? 'marketplace needs' : 'marketplaces need'} you`}
			>
				{needing.map((chip) => platformTitle(chip.inventory)).join(', ')}: nothing is sent until you
				fix this.
				{#snippet action()}
					{#if needing[0].action}
						<Button href={needing[0].action.href}>{needing[0].action.label}</Button>
					{/if}
				{/snippet}
			</Banner>
		{/if}

		<!-- The same form the create route renders, in its second mode, with
		     this page's own pieces handed into its steps: the marketplace tiles
		     into step 3, Publish beside Save, and the history under the steps. -->
		<ResourceForm mode={{ kind: 'edit', product: stored, mapped: inventories, blockedBy }}>
			{#snippet listed()}
				<div class="flow-section">
					<div class="flow-section-head">
						<h3>Listed now</h3>
						<!-- Where the listing stands overall, and whether this page is
						     being told about changes as they happen: two facts, so the
						     second says "connected" rather than "live". -->
						<span class="panel-badges">
							<StatusPill tone={PILL_TONE[status.tone]} label={status.label} />
							<StatusPill tone={live ? 'ok' : 'soon'} label={live ? 'connected' : 'reconnecting'} />
						</span>
					</div>
					<div class="res-mk-grid">
						{#each tiles as tile (tile.inventory)}
							<MarketplaceTile
								{tile}
								adding={adding !== null}
								addingHere={adding === tile.inventory}
								attachOpen={tile.mapping !== null && attaching === tile.mapping}
								{attachSending}
								oncrosslist={() => void crossListTo(tile.inventory)}
								onattach={() => toggleAttach(tile.mapping)}
							/>
						{/each}
					</div>

					<!-- One attach form below the grid rather than one inside each
					     tile, naming the marketplace it writes against. -->
					{#if attachingTo !== undefined}
						<div class="mk-attach">
							<Field
								label="Listing link"
								id={`attach-${attachingTo.mapping}`}
								hint="Paste the link to your existing listing."
							>
								<input
									id={`attach-${attachingTo.mapping}`}
									type="url"
									placeholder="https://…"
									disabled={attachSending}
									bind:value={attachUrl}
								/>
							</Field>
							<Button
								tier="primary"
								disabled={attachSending || attachUrl.trim().length === 0}
								reason={attachUrl.trim().length === 0 ? 'Paste the listing link first.' : undefined}
								onclick={() => void attach(attachingTo.mapping)}
							>
								{attachSending ? 'Attaching…' : 'Attach'}
							</Button>
						</div>
						{#if attachRefusal !== null}
							<Banner tone="bad">{attachRefusal}</Banner>
						{/if}
					{/if}
					{#if addRefusal !== null}
						<Banner tone="bad">{addRefusal}</Banner>
					{/if}
				</div>
			{/snippet}

			{#snippet publish()}
				<Button tier="primary" icon="share-2" onclick={() => (publishing = true)}>Publish…</Button>
			{/snippet}

			{#snippet after()}
				{#if runs.length > 0}
					<section class="flow-section">
						<div class="flow-section-head"><h2>Recent sends</h2></div>
						<div class="res-lines">
							{#each runs as run (run.job)}
								<a class="res-line" href={`/sync/${run.job}`}>
									<span class="res-line-what">
										<span class="res-line-t"><MarketplaceMark inventory={run.inventory} size={18} /></span>
										<span class="res-line-id">{run.job.slice(0, 8)}…</span>
									</span>
									<span class="res-line-at">{run.state === null ? 'just started' : run.state}</span>
								</a>
							{/each}
						</div>
					</section>
				{/if}

				{#if (labels.data ?? []).length > 0 || (collections.data ?? []).length > 0}
					<!-- Drawn only where there is one: an empty section on every
					     resource page would be a heading standing in for a fact. -->
					<section class="flow-section">
						<div class="flow-section-head"><h2>Filed under</h2></div>
						{#if (labels.data ?? []).length > 0}
							<div class="res-chips">
								{#each labels.data ?? [] as label (label.name)}
									<LabelChip name={label.name} colour={label.colour} system={label.system} />
								{/each}
							</div>
						{/if}
						{#if (collections.data ?? []).length > 0}
							<div class="res-lines">
								{#each collections.data ?? [] as collection (collection.id)}
									<a class="res-line" href={`/collections/${collection.id}`}>
										<span class="res-line-what">
											<Icon name="layers" size={16} />
											<span class="res-line-t">{collection.name}</span>
										</span>
										<span class="res-line-at">{countLine(collection.count)}</span>
									</a>
								{/each}
							</div>
						{/if}
					</section>
				{/if}
			{/snippet}
		</ResourceForm>

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
