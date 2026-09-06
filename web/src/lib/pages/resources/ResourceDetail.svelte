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
	import { editBlockedBy } from '$lib/authoring';
	import { AUTHORABLE_PLATFORMS, platformTitle } from '$lib/platforms';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import DeleteDialog from '$lib/DeleteDialog.svelte';
	import { agoLabel } from '$lib/elapsed';
	import { external } from '$lib/external';
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
	import ResourceForm from './ResourceForm.svelte';
	import PublishDialog from '$lib/PublishDialog.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { connectionFor, readinessOf } from '$lib/publish-readiness';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import type { InventoryId } from '$lib/generated/vocab';
	import { PILL_TONE, fullStop } from './list';
	import { sentenceFor } from './refusal';
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

	async function deleted() {
		deleting = false;
		await queryClient.invalidateQueries({ queryKey: queryKeys.products });
		await queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
		toast('info', 'Listing deleted.');
		await goto('/resources');
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
				<Button href="/resources" icon="layout-list">Back to Resources</Button>
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
			headline="Which marketplaces carry this resource could not be read"
			body="Nothing below would be true without it, so nothing below is shown. Reload to try again."
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
								use:external
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

		<!-- The same form the create route renders, in its second mode. Not a
		     second form: the edit used to collect six fields of a model with
		     twenty-four, so seventeen sidecar fields were set once at create and
		     could never be changed. The live-listing banner, the files panel and
		     the marketplace ticks are all inside it, because each belongs beside
		     the fields it constrains. -->
		<ResourceForm
			mode={{ kind: 'edit', product: stored, mapped: inventories, blockedBy }}
		/>

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
