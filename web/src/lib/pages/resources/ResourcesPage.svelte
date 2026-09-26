<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { allPages, api, type MappingHead } from '$lib/api';
	import AddToCollectionDialog from '$lib/AddToCollectionDialog.svelte';
	import ApplyTemplateDialog from '$lib/ApplyTemplateDialog.svelte';
	import Banner from '$lib/Banner.svelte';
	import BulkDeleteDialog from '$lib/BulkDeleteDialog.svelte';
	import { BULK_ACTIONS, type BulkVerb } from '$lib/bulk-verbs';
	import Button from '$lib/Button.svelte';
	import CrossListDialog from '$lib/CrossListDialog.svelte';
	import DeleteDialog from '$lib/DeleteDialog.svelte';
	import { entitlementRead, featureOf, limitOf } from '$lib/entitlement-read';
	import Explain from '$lib/Explain.svelte';
	import FlowActionBar from '$lib/FlowActionBar.svelte';
	import FlowDiagram from '$lib/FlowDiagram.svelte';
	import Icon from '$lib/Icon.svelte';
	import {
		WORK_RUNS,
		newestWork,
		rowFor,
		type InventoryRow,
		type StandingFilter,
		type WorkItem
	} from '$lib/inventory';
	import LabelChip from '$lib/LabelChip.svelte';
	import LabelsDialog from '$lib/LabelsDialog.svelte';
	import { normaliseQuery } from '$lib/listings-view';
	import MarketplaceChips from '$lib/MarketplaceChips.svelte';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import MarkListedDialog from '$lib/MarkListedDialog.svelte';
	import Menu from '$lib/Menu.svelte';
	import MenuItem from '$lib/MenuItem.svelte';
	import Note from '$lib/Note.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import { palette } from '$lib/palette.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { platformTitle } from '$lib/platforms';
	import { COLLECTIONS_KEY } from '$lib/pages/collections/collections';
	import { queryKeys } from '$lib/query';
	import { MIGRATION_HREF } from '$lib/sync-request';
	import { MAPPINGS_HREF, PRICING_HREF } from '$lib/pages/automations/seller-rules';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import type { InventoryId } from '$lib/generated/vocab';
	import { FILES_HREF } from './files-browser';
	import {
		BULK_ICON,
		MARKETPLACE_TILES,
		PAGE_STEP,
		DEFAULT_TAB,
		RESOURCE_TABS,
		SORTS,
		STANDING_OPTIONS,
		countsFor,
		filterSearch,
		filtersActive,
		labelsFromUrl,
		matchesResource,
		rowFacts,
		sortRows,
		unionById,
		type ResourceFilters,
		type SortId,
		type TabId
	} from './list';
	import './resources.css';

	/** Nothing here can copy a resource: `POST /v1/products` wants a
	 *  `FileHandle` per file and `GET /v1/products/{id}` serves a `FileView`,
	 *  which carries the identifier and not the hash, so a copy made from what
	 *  this client can read would be a resource with no payload. */
	const DUPLICATE_MISSING = 'You can’t copy a resource yet.';

	const queryClient = useQueryClient();

	// The plan, from the cache the shell filled. Both controls in the empty
	// board start something the server will refuse at the cap, so the refusal
	// is stated on the control rather than after the form.
	const plan = createQuery(() => entitlementRead);
	const createRefusal = $derived(limitOf(plan.data, 'resources'));
	const importRefusal = $derived(featureOf(plan.data, 'import_marketplace'));

	// The two filters the server answers ride the URL: the Labels page links
	// straight to `/resources?label=<name>`, and a narrowed board is a place
	// worth bookmarking, so both have to be readable from the address rather
	// than held in this component. The rest of the filter card narrows rows this
	// page already holds and stays local.
	const chosenLabels = $derived(labelsFromUrl(page.url.searchParams));
	// One cache entry per selection rather than per label, because the answer is
	// the union of several reads and not any one of them.
	const labelKey = $derived(
		chosenLabels.length === 0 ? null : JSON.stringify([...chosenLabels].sort())
	);

	// The label narrows what the server sends rather than what this page shows,
	// because it is a clause in the catalogue's own page query. Several labels
	// are several walks of the catalogue: the endpoint answers one at a time.
	const catalogue = createQuery(() => ({
		queryKey: queryKeys.catalogue(labelKey),
		queryFn: async () => {
			if (chosenLabels.length === 0) {
				return allPages(
					(cursor) => api.products(cursor, null),
					(view) => view.products
				);
			}
			const walks = await Promise.all(
				chosenLabels.map((label) =>
					allPages(
						(cursor) => api.products(cursor, label),
						(view) => view.products
					)
				)
			);
			return unionById(walks);
		}
	}));
	const mappings = createQuery(() => ({
		queryKey: queryKeys.mappings,
		queryFn: () => api.mappings().then((view) => view.mappings)
	}));
	const connections = createQuery(() => ({
		queryKey: queryKeys.connections,
		queryFn: () => api.connections()
	}));
	const halts = createQuery(() => ({
		queryKey: queryKeys.status,
		queryFn: () => api.status().then((view) => view.inventories)
	}));
	// Every label the organisation uses. A label nothing carries any more is
	// absent, so the filter never offers one that can only answer nothing.
	const labels = createQuery(() => ({
		queryKey: queryKeys.labels,
		queryFn: () => api.labels().then((view) => view.labels)
	}));
	// What is happening to each mapping right now, which no endpoint answers
	// directly: an item is reachable only through the run that holds it, so the
	// newest runs are read and the newest item per mapping is kept.
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

	let marketplaces = $state<InventoryId[]>([]);
	let standing = $state<StandingFilter>('all');
	let sort = $state<SortId>('updated_desc');
	// The board opens on the chip that hides nothing. Pressing the chosen chip
	// again returns here, so one chip is always pressed and the state the
	// board is in is always named.
	let tab = $state<TabId>(DEFAULT_TAB);
	let drawn = $state(PAGE_STEP);

	let bulkMenu = $state(false);
	let moreMenu = $state(false);
	// The row whose kebab is open: one at a time, like every menu here.
	let rowMenu = $state<string | null>(null);
	let selected = $state<Set<string>>(new Set());
	let removing = $state<InventoryRow | null>(null);

	// The URL carries the search, so a narrowed board can be linked to and
	// returned to. The field below is seeded from the address and writes it back
	// as it is typed in, and the two must not disagree.
	const query = $derived(normaliseQuery(page.url.searchParams.get('q')));
	let box = $state(normaliseQuery(page.url.searchParams.get('q')));
	$effect(() => {
		if (query !== box.trim()) {
			box = query;
		}
	});

	/** Writes both server-side filters at once, because either alone would drop
	 *  the other from the address. Replaces rather than pushes: a filter is not
	 *  a place, and a back button that walked every keystroke would be. */
	function narrow(nextQuery: string, nextLabels: readonly string[]) {
		const search = filterSearch(nextQuery, nextLabels);
		void goto(search.length === 0 ? page.url.pathname : `${page.url.pathname}?${search}`, {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	}

	function typed(next: string) {
		box = next;
		narrow(next, chosenLabels);
	}

	const filters = $derived<ResourceFilters>({ query, marketplaces, standing, tab });

	/** The four reads a row is built from. A chip is not a chip without them:
	 *  `chipFor` reads an absent connection as "no account is linked" and an
	 *  absent status as "not paused", so a row drawn while any of these is
	 *  outstanding states things nobody was told. */
	const reading = $derived(
		catalogue.isPending || mappings.isPending || connections.isPending || halts.isPending
	);
	const unread = $derived([
		catalogue.isError ? 'your resources' : null,
		mappings.isError ? 'where each resource is listed' : null,
		connections.isError ? 'which marketplaces you are signed in to' : null,
		halts.isError ? 'whether any marketplace is paused' : null
	].filter((what): what is string => what !== null));
	const read = $derived(!reading && unread.length === 0);

	const products = $derived(catalogue.data ?? []);

	const byProduct = $derived.by(() => {
		const index = new Map<string, MappingHead[]>();
		for (const mapping of mappings.data ?? []) {
			index.set(mapping.product, [...(index.get(mapping.product) ?? []), mapping]);
		}
		return index;
	});

	const allRows = $derived(
		products.map((product) =>
			rowFor({
				product,
				mappings: byProduct.get(product.id) ?? [],
				work: work.data ?? new Map(),
				connections: connections.data ?? [],
				statuses: halts.data ?? []
			})
		)
	);

	const counts = $derived(countsFor(allRows, read));
	const tabs = $derived(
		RESOURCE_TABS.map((entry) => ({
			id: entry.id,
			label: entry.label,
			count: counts[entry.id],
			hint: entry.hint
		}))
	);
	/** How many of the filters behind "More" are set, so the chip says so
	 *  while they are out of sight. */
	const moreSet = $derived(chosenLabels.length + (standing === 'all' ? 0 : 1));

	const rows = $derived(
		sortRows(
			allRows.filter((row) => matchesResource(row, filters)),
			sort
		)
	);
	const shown = $derived(rows.slice(0, drawn));
	// Re-read whenever the catalogue is, rather than once at mount: the ages in
	// the meta lines are written against this, and a board left open overnight
	// would otherwise still say "updated 2 hours ago".
	const now = $derived.by(() => {
		void catalogue.dataUpdatedAt;
		return Date.now();
	});

	// A narrowed list is a shorter list, so the stack starts from the top again
	// rather than keeping a depth the seller reached under other filters.
	$effect(() => {
		void filters;
		void sort;
		void labelKey;
		drawn = PAGE_STEP;
	});

	const anythingSet = $derived(filtersActive(filters, chosenLabels));
	const chosen = $derived(allRows.filter((row) => selected.has(row.product.id)));
	const allShownSelected = $derived(
		shown.length > 0 && shown.every((row) => selected.has(row.product.id))
	);
	/** The verb the bar offers first, the one a selection is most often made
	 *  for; the rest sit in its menu. */
	const firstVerb = BULK_ACTIONS[0];
	const otherVerbs = BULK_ACTIONS.slice(1);
	// Covers that failed to load, drawn as the placeholder rather than a
	// broken image.
	let brokenCovers = $state<Set<string>>(new Set());

	function clearFilters() {
		marketplaces = [];
		standing = 'all';
		tab = DEFAULT_TAB;
		box = '';
		narrow('', []);
	}

	function chooseTab(id: TabId) {
		tab = tab === id ? DEFAULT_TAB : id;
	}

	function toggleMarketplace(inventory: InventoryId) {
		marketplaces = marketplaces.includes(inventory)
			? marketplaces.filter((entry) => entry !== inventory)
			: [...marketplaces, inventory];
	}

	function toggleLabel(name: string) {
		narrow(
			box,
			chosenLabels.includes(name)
				? chosenLabels.filter((entry) => entry !== name)
				: [...chosenLabels, name]
		);
	}

	function setSelected(product: string, value: boolean) {
		const next = new Set(selected);
		if (value) {
			next.add(product);
		} else {
			next.delete(product);
		}
		selected = next;
	}

	function selectAllShown() {
		selected = new Set(shown.map((row) => row.product.id));
	}

	function toggleAllShown() {
		if (allShownSelected) {
			selected = new Set();
			return;
		}
		selectAllShown();
	}

	function cancelBulk() {
		bulkMenu = false;
		selected = new Set();
	}

	let crossListing = $state(false);
	let markingListed = $state(false);
	let labelling = $state(false);
	let deleting = $state(false);
	let collecting = $state(false);
	let templating = $state(false);

	/** Every built verb does something; Edit alone is disabled at the control
	 *  with its recorded reason, so this is exhaustive over what can be run.
	 *
	 *  Copy-or-move leaves this page rather than opening a dialog: the seller
	 *  chooses a pair of marketplaces, a disposition and reads a per-resource
	 *  preview before anything is queued, which is a screen and not a
	 *  confirmation. The selection travels in the address so the page it lands
	 *  on can be reopened with the same tick list. */
	function run(chosenVerb: BulkVerb) {
		if (chosenVerb === 'cross_list') {
			crossListing = true;
		} else if (chosenVerb === 'mark_listed') {
			markingListed = true;
		} else if (chosenVerb === 'labels') {
			labelling = true;
		} else if (chosenVerb === 'add_to_collection') {
			collecting = true;
		} else if (chosenVerb === 'apply_template') {
			templating = true;
		} else if (chosenVerb === 'delete') {
			deleting = true;
		} else if (chosenVerb === 'move' || chosenVerb === 'price' || chosenVerb === 'map_terms') {
			// Each identifier escaped and the commas left as commas: the separator
			// is part of the address's own grammar, and escaping it whole turned a
			// readable link into `products=p4%2Cp5`.
			const products = chosen.map((row) => encodeURIComponent(row.product.id)).join(',');
			// Pricing and Mappings are the same kind of hand-off as Migrations:
			// what a resource should cost or land under on the target is decided
			// beside a per-resource preview the server computed, so the selection
			// travels in the address and the decision is taken there.
			const where =
				chosenVerb === 'move'
					? MIGRATION_HREF
					: chosenVerb === 'price'
						? PRICING_HREF
						: MAPPINGS_HREF;
			void goto(`${where}?products=${products}`);
		}
	}

	/** The collection the selection went into, named rather than counted: a
	 *  seller who has four collections needs to know which one grew. */
	async function collected(into: string, count: number) {
		collecting = false;
		cancelBulk();
		toast(
			'info',
			count === 0
				? `Those resources are already in ${into}.`
				: `${count} ${count === 1 ? 'resource' : 'resources'} added to ${into}.`
		);
		await queryClient.invalidateQueries({ queryKey: COLLECTIONS_KEY });
	}

	/** A template landed on the selection. The dialog says what it changed, so
	 *  this only refreshes what the writes touched and stands the board down. */
	async function templated() {
		templating = false;
		cancelBulk();
		await Promise.all([
			queryClient.invalidateQueries({ queryKey: queryKeys.products }),
			queryClient.invalidateQueries({ queryKey: queryKeys.mappings })
		]);
	}

	function started(runs: { inventory: InventoryId; job: string }[]) {
		crossListing = false;
		cancelBulk();
		void queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
		void queryClient.invalidateQueries({ queryKey: queryKeys.inventoryWork });
		if (runs.length === 0) {
			return;
		}
		if (runs.length === 1) {
			toast('info', `Started sending to ${platformTitle(runs[0].inventory)}.`);
			void goto(`/sync/${runs[0].job}`);
			return;
		}
		toast('info', `Started sending to ${runs.length} marketplaces.`);
	}

	async function labelled(count: number) {
		labelling = false;
		cancelBulk();
		toast('info', `Labels updated on ${count} ${count === 1 ? 'resource' : 'resources'}.`);
		await Promise.all([
			queryClient.invalidateQueries({ queryKey: queryKeys.products }),
			queryClient.invalidateQueries({ queryKey: queryKeys.labels })
		]);
	}

	/** Some resources were relabelled before one failed. The writes that landed
	 *  are real, so the board refreshes; the dialog stays open with the refusal
	 *  on it, and nothing here claims the selection succeeded. */
	async function partlyLabelled(_count: number) {
		await Promise.all([
			queryClient.invalidateQueries({ queryKey: queryKeys.products }),
			queryClient.invalidateQueries({ queryKey: queryKeys.labels })
		]);
	}

	async function markedListed(bound: number) {
		markingListed = false;
		cancelBulk();
		toast('info', `${bound} ${bound === 1 ? 'listing' : 'listings'} attached.`);
		await queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
	}

	async function deletedInBulk(count: number) {
		deleting = false;
		cancelBulk();
		toast('info', `${count} ${count === 1 ? 'resource' : 'resources'} deleted.`);
		await Promise.all([
			queryClient.invalidateQueries({ queryKey: queryKeys.products }),
			queryClient.invalidateQueries({ queryKey: queryKeys.mappings })
		]);
	}

	async function partlyDeleted(_count: number) {
		await Promise.all([
			queryClient.invalidateQueries({ queryKey: queryKeys.products }),
			queryClient.invalidateQueries({ queryKey: queryKeys.mappings })
		]);
	}

	async function deletedOne() {
		removing = null;
		toast('info', 'Resource deleted.');
		await Promise.all([
			queryClient.invalidateQueries({ queryKey: queryKeys.products }),
			queryClient.invalidateQueries({ queryKey: queryKeys.mappings })
		]);
	}
</script>

<div class="page resources-page" class:bulk-open={selected.size > 0}>
	<PageHead icon="layout-list" title="Resources" guide="labels-and-collections" search={() => palette.show()}>
		{#snippet aside()}
			<Explain title="How resources reach a marketplace" label="How it works">
				<FlowDiagram
					from={{ icon: 'package', label: 'Your resource' }}
					to={[{ icon: 'store', label: 'Marketplaces' }]}
					rule="Publish"
					label="Your resource is published to your marketplaces"
				/>
				<p>1. Add the file and the details once.</p>
				<p>2. Choose the marketplaces it goes to.</p>
				<p>3. Publish. Nothing changes on a marketplace until you do.</p>
				<p>
					TES and TPT work runs in the Teachouse app on your own computer, so nothing is sent
					while it is off.
				</p>
			</Explain>
			<!-- The way to the seller's own files, which are a property of the
			     catalogue rather than of the marketplaces they came from. New
			     resource is the shell's own primary at the top right, and the
			     action bar's on a phone, so the head does not draw a third. -->
			<Button tier="outline" icon="files" href={FILES_HREF}>Files</Button>
		{/snippet}
	</PageHead>

	{#if read && (counts.attention ?? 0) > 0 && tab !== 'attention' && standing !== 'attention'}
		<Banner
			tone="warn"
			title={`${counts.attention} ${counts.attention === 1 ? 'resource needs' : 'resources need'} you`}
		>
			Check the Teachouse app is running.
			{#snippet action()}
				<Button onclick={() => (tab = 'attention')}>Show them</Button>
			{/snippet}
		</Banner>
	{/if}

	<!-- Search and every filter in one row: the state chips carry their
	     counts, the marketplace chips their marks, and the two filters a
	     seller rarely needs sit behind More. -->
	<div class="res-bar">
		<label class="res-search">
			<Icon name="search" size={16} />
			<span class="sr-only">Search</span>
			<input
				id="resource-search"
				type="search"
				placeholder="Search by title"
				value={box}
				oninput={(event) => typed(event.currentTarget.value)}
			/>
		</label>

		<div class="res-fchips" role="group" aria-label="Filter resources">
			{#each tabs as one (one.id)}
				<button
					type="button"
					class="res-fchip"
					aria-pressed={tab === one.id}
					title={one.hint}
					onclick={() => chooseTab(one.id)}
				>
					{one.label}
					{#if one.count !== null && one.count !== undefined}
						<span class="res-fchip-n">{one.count}</span>
					{/if}
				</button>
			{/each}
			<span class="res-fchip-sep" aria-hidden="true"></span>
			{#each MARKETPLACE_TILES as tile (tile.inventory)}
				<!-- The mark alone: a teacher knows the logo, and the words would
				     push the row past one line. The title and the name still say it. -->
				<button
					type="button"
					class="res-fchip res-fchip-mark"
					disabled={tile.disabled}
					aria-pressed={marketplaces.includes(tile.inventory)}
					aria-label={tile.reason ?? platformTitle(tile.inventory)}
					title={tile.reason ?? platformTitle(tile.inventory)}
					onclick={() => toggleMarketplace(tile.inventory)}
				>
					<MarketplaceMark inventory={tile.inventory} size={18} />
				</button>
			{/each}
		</div>
		<Menu bind:open={moreMenu} label="More filters" align="end">
			{#snippet trigger()}
				<button
					type="button"
					class="res-fchip"
					aria-pressed={moreSet > 0}
					aria-expanded={moreMenu}
					onclick={() => (moreMenu = !moreMenu)}
				>
					<Icon name="sliders-horizontal" size={14} />
					More
					{#if moreSet > 0}<span class="res-fchip-n">{moreSet}</span>{/if}
				</button>
			{/snippet}
			<div class="label-menu">
				<label class="res-more-status">
					<span>Status on a marketplace</span>
					<select
						id="resource-standing"
						value={standing}
						onchange={(event) => (standing = event.currentTarget.value as StandingFilter)}
					>
						{#each STANDING_OPTIONS as option (option.value)}
							<option value={option.value}>{option.label}</option>
						{/each}
					</select>
				</label>
				<p class="none">Labels</p>
				<!-- Three arms, because a read in flight and a read that failed are
				     not the same answer as a seller who has made no labels. -->
				{#if labels.isPending}
					<p class="none">Loading your labels…</p>
				{:else if labels.isError}
					<p class="none">We couldn’t load your labels. Reload to try again.</p>
				{:else}
					{#each labels.data ?? [] as one (one.name)}
						<label>
							<input
								type="checkbox"
								checked={chosenLabels.includes(one.name)}
								onchange={() => toggleLabel(one.name)}
							/>
							<LabelChip name={one.name} colour={one.colour} system={one.system} />
						</label>
					{:else}
						<p class="none">None of your resources has a label yet.</p>
					{/each}
				{/if}
			</div>
		</Menu>
	</div>

	<div class="res-toolbar">
		<!-- No figure until the reads are in: "0 resources" is what a seller
		     with an empty catalogue sees, so standing it in for a read that
		     failed states something false in a number nobody would doubt. -->
		<span class="eyebrow">
			{#if read}
				{rows.length}
				{rows.length === 1 ? 'resource' : 'resources'}
			{:else if reading}
				Loading your resources
			{:else}
				Couldn’t count your resources
			{/if}
		</span>
		{#if anythingSet}
			<Button tier="quiet" small icon="x" onclick={clearFilters}>Clear filters</Button>
		{/if}
		<span class="res-sort-at">
			<label class="sr-only" for="resource-sort">Sort by</label>
			<select
				class="res-sort"
				id="resource-sort"
				value={sort}
				onchange={(event) => (sort = event.currentTarget.value as SortId)}
			>
				{#each SORTS as option (option.id)}
					<option value={option.id}>{option.label}</option>
				{/each}
			</select>
		</span>
	</div>

	{#if reading}
		<p class="res-note">Loading your resources…</p>
	{:else if unread.length > 0}
		<!-- Named rather than summarised, and the rows withheld rather than
		     drawn from what did arrive: a row built without the connections says
		     every marketplace needs a sign-in, and one built without the statuses
		     says a paused marketplace is running. -->
		<Banner tone="bad" title={`Part of this page didn’t load: ${unread.join(', ')}`}>
			Reload to try again.
		</Banner>
	{:else if allRows.length === 0 && !anythingSet}
		<Placeholder
			icon="layout-list"
			headline="You have no resources yet."
			body="Make your first one, or bring your shop in."
		>
			{#snippet actions()}
				<Button
					tier="primary"
					href="/resources/new"
					icon="circle-plus"
					disabled={createRefusal !== null}
					reason={createRefusal ?? undefined}
				>
					New resource
				</Button>
				<Button
					href="/import"
					icon="download"
					disabled={importRefusal !== null}
					reason={importRefusal ?? undefined}
				>
					Import from a marketplace
				</Button>
			{/snippet}
		</Placeholder>
	{:else if rows.length === 0}
		<Placeholder icon="search" headline="Nothing matches these filters.">
			{#snippet actions()}
				<Button tier="quiet" onclick={clearFilters}>Clear filters</Button>
			{/snippet}
		</Placeholder>
	{:else}
		<div class="res-list">
			<div class="res-list-head">
				<input
					type="checkbox"
					class="res-item-tick"
					aria-label="Select every resource shown"
					checked={allShownSelected}
					onchange={toggleAllShown}
				/>
				<span>Resource</span>
				<span>Price</span>
				<span>Marketplaces</span>
				<span>Status</span>
				<span></span>
			</div>
			{#each shown as row (row.product.id)}
				{@const facts = rowFacts(row.product, row, now)}
				{@const cover = row.product.cover ?? null}
				<div class="res-item" class:picked={selected.has(row.product.id)}>
					<input
						type="checkbox"
						class="res-item-tick"
						aria-label={`Select ${row.product.title}`}
						checked={selected.has(row.product.id)}
						onchange={(event) => setSelected(row.product.id, event.currentTarget.checked)}
					/>
					<a class="res-item-main" href={`/resources/${row.product.id}`}>
						<!-- The alt is empty on purpose: the title beside it names the
						     resource. Lazily loaded, because a catalogue draws sixty. -->
						<span class="res-item-cover">
							{#if cover !== null && !brokenCovers.has(cover)}
								<img
									src={cover}
									alt=""
									loading="lazy"
									decoding="async"
									onerror={() => (brokenCovers = new Set([...brokenCovers, cover]))}
								/>
							{:else}
								<Icon name="image" size={18} />
							{/if}
						</span>
						<span class="res-item-text">
							<span class="res-name">{row.product.title}</span>
							<span class="res-item-meta">{facts.updated}</span>
						</span>
					</a>
					<span class="price res-item-price">{facts.price}</span>
					<span class="res-item-marks"><MarketplaceChips chips={row.chips} /></span>
					<span class="res-item-status">
						<StatusPill tone={facts.status.tone} label={facts.status.label} />
					</span>
					<span class="res-item-menu">
						<Menu
							bind:open={
								() => rowMenu === row.product.id,
								(value) => (rowMenu = value ? row.product.id : null)
							}
							label={`Actions for ${row.product.title}`}
						>
							{#snippet trigger()}
								<button
									type="button"
									class="res-kebab"
									aria-label={`Actions for ${row.product.title}`}
									onclick={() =>
										(rowMenu = rowMenu === row.product.id ? null : row.product.id)}
								>
									<Icon name="ellipsis-vertical" size={16} />
								</button>
							{/snippet}
							<a class="res-menu-link menu-item" href={`/resources/${row.product.id}`}>
								<Icon name="eye" size={14} />Open
							</a>
							<MenuItem icon="copy" disabled reason={DUPLICATE_MISSING}>Duplicate</MenuItem>
							<MenuItem
								icon="trash-2"
								danger
								onclick={() => {
									rowMenu = null;
									removing = row;
								}}
							>
								Delete
							</MenuItem>
						</Menu>
					</span>
				</div>
			{/each}
		</div>

		{#if rows.length > shown.length}
			<div class="res-more">
				<Button onclick={() => (drawn += PAGE_STEP)}>
					Show {Math.min(PAGE_STEP, rows.length - shown.length)} more
				</Button>
			</div>
		{/if}

		{#if work.isError}
			<Note icon="triangle-alert">
				The marketplace status on each resource may be out of date.
			</Note>
		{/if}
	{/if}

	{#if selected.size > 0}
		<!-- `chosen.length` rather than `selected.size`: a selection survives a
		     filter change, and the label filter narrows the server's own query,
		     so a resource can be selected and no longer among the rows this
		     page holds. The dialogs act on `chosen`, so the figure is the one
		     they will act on. -->
		<div class="bulk-bar" role="region" aria-label="Selected resources">
			<span class="bulk-count"><strong>{chosen.length}</strong> selected</span>
			{#if chosen.length < rows.length}
				<Button
					tier="quiet"
					small
					onclick={() => (selected = new Set(rows.map((row) => row.product.id)))}
				>
					Select all {rows.length}
				</Button>
			{/if}
			<Button
				tier="primary"
				icon={BULK_ICON[firstVerb.verb]}
				disabled={chosen.length === 0}
				reason={chosen.length === 0 ? 'Select at least one resource that is in view.' : undefined}
				onclick={() => run(firstVerb.verb)}
			>
				{firstVerb.label}
			</Button>
			<Menu bind:open={bulkMenu} label="More actions">
				{#snippet trigger()}
					<Button icon="ellipsis-vertical" onclick={() => (bulkMenu = !bulkMenu)}>More</Button>
				{/snippet}
				{#each otherVerbs as action (action.verb)}
					<MenuItem
						icon={BULK_ICON[action.verb]}
						danger={action.verb === 'delete'}
						disabled={action.missing !== null || chosen.length === 0}
						reason={action.missing ??
							(chosen.length === 0 ? 'Select at least one resource that is in view.' : undefined)}
						onclick={() => {
							bulkMenu = false;
							run(action.verb);
						}}
					>
						{action.label}…
					</MenuItem>
				{/each}
			</Menu>
			<Button tier="quiet" icon="x" onclick={cancelBulk}>Clear</Button>
		</div>
	{:else}
		<FlowActionBar>
			<Button
				tier="primary"
				icon="circle-plus"
				href="/resources/new"
				disabled={createRefusal !== null}
				reason={createRefusal ?? undefined}
			>
				New resource
			</Button>
		</FlowActionBar>
	{/if}
</div>

<CrossListDialog
	open={crossListing}
	rows={chosen}
	onClose={() => (crossListing = false)}
	onStarted={started}
/>

<LabelsDialog
	open={labelling}
	rows={chosen}
	known={labels.data ?? []}
	onClose={() => (labelling = false)}
	onLabelled={labelled}
	onPartial={partlyLabelled}
/>

<MarkListedDialog
	open={markingListed}
	rows={chosen}
	onClose={() => (markingListed = false)}
	onBound={markedListed}
/>

<BulkDeleteDialog
	open={deleting}
	rows={chosen}
	onClose={() => (deleting = false)}
	onDeleted={deletedInBulk}
	onPartial={partlyDeleted}
/>

<DeleteDialog
	open={removing !== null}
	product={removing?.product.id ?? ''}
	title={removing?.product.title ?? ''}
	mappings={removing === null ? [] : [...removing.mapped.values()]}
	onClose={() => (removing = null)}
	onDeleted={deletedOne}
/>

<AddToCollectionDialog
	open={collecting}
	rows={chosen}
	onClose={() => (collecting = false)}
	onAdded={collected}
/>

<!-- The template's own dialog, shared with the Collections page and the
     Template Manager: it reads the template list itself, so the selection is
     all this board has to hand it. -->
<ApplyTemplateDialog
	open={templating}
	products={chosen.map((row) => row.product.id)}
	onClose={() => (templating = false)}
	onDone={templated}
/>
