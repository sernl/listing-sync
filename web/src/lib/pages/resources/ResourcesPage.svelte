<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { allPages, api, type MappingHead } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import BulkDeleteDialog from '$lib/BulkDeleteDialog.svelte';
	import { BULK_ACTIONS, unavailable, type BulkVerb } from '$lib/bulk-verbs';
	import Button from '$lib/Button.svelte';
	import CrossListDialog from '$lib/CrossListDialog.svelte';
	import DeleteDialog from '$lib/DeleteDialog.svelte';
	import Field from '$lib/Field.svelte';
	import { entitlementRead, featureOf, limitOf } from '$lib/entitlement-read';
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
	import MarkListedDialog from '$lib/MarkListedDialog.svelte';
	import Menu from '$lib/Menu.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import { palette } from '$lib/palette.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { platformTitle } from '$lib/platforms';
	import { queryKeys } from '$lib/query';
	import RowCard from '$lib/RowCard.svelte';
	import TabBar from '$lib/TabBar.svelte';
	import { toast } from '$lib/toast';
	import type { InventoryId } from '$lib/generated/vocab';
	import {
		MARKETPLACE_TILES,
		PAGE_STEP,
		DEFAULT_TAB,
		RESOURCE_TABS,
		SORTS,
		STANDING_OPTIONS,
		VERB_PHRASE,
		countsFor,
		filterSearch,
		filtersActive,
		labelsFromUrl,
		matchesResource,
		metaLine,
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
	const DUPLICATE_MISSING =
		'Copying a resource needs each file’s hash, which the product view does not serve.';

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
	// The board opens on the segment that hides nothing. Choosing the open tab
	// again returns here, so the bar always has a selected tab and the state the
	// board is in is always named.
	let tab = $state<TabId>(DEFAULT_TAB);
	let lastTab: TabId = DEFAULT_TAB;
	let drawn = $state(PAGE_STEP);

	let bulkMenu = $state(false);
	let scopeMenu = $state(false);
	let labelMenu = $state(false);
	let mode = $state<BulkVerb | null>(null);
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
		catalogue.isError ? 'your Resources' : null,
		mappings.isError ? 'which marketplaces carry each resource' : null,
		connections.isError ? 'which marketplaces you are signed in to' : null,
		halts.isError ? 'whether sending is paused anywhere' : null
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
	const verb = $derived(BULK_ACTIONS.find((action) => action.verb === mode) ?? null);

	function clearFilters() {
		marketplaces = [];
		standing = 'all';
		tab = DEFAULT_TAB;
		lastTab = DEFAULT_TAB;
		box = '';
		narrow('', []);
	}

	function chooseTab(id: string) {
		if (lastTab === id) {
			tab = DEFAULT_TAB;
			lastTab = DEFAULT_TAB;
			return;
		}
		lastTab = id as TabId;
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
		mode = null;
		selected = new Set();
	}

	let crossListing = $state(false);
	let markingListed = $state(false);
	let labelling = $state(false);
	let deleting = $state(false);

	/** Only the four built verbs open anything; Edit is disabled at the control
	 *  with its recorded reason, so this is exhaustive over what can be run. */
	function run(chosenVerb: BulkVerb) {
		if (chosenVerb === 'cross_list') {
			crossListing = true;
		} else if (chosenVerb === 'mark_listed') {
			markingListed = true;
		} else if (chosenVerb === 'labels') {
			labelling = true;
		} else if (chosenVerb === 'delete') {
			deleting = true;
		}
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
			toast('info', `Send started on ${platformTitle(runs[0].inventory)}.`);
			void goto(`/sync/${runs[0].job}`);
			return;
		}
		toast('info', `Send started on ${runs.length} marketplaces.`);
	}

	async function labelled(count: number) {
		labelling = false;
		cancelBulk();
		toast('info', `${count} ${count === 1 ? 'resource' : 'resources'} relabelled.`);
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

	async function deletedOne() {
		removing = null;
		toast('info', 'Resource deleted.');
		await Promise.all([
			queryClient.invalidateQueries({ queryKey: queryKeys.products }),
			queryClient.invalidateQueries({ queryKey: queryKeys.mappings })
		]);
	}
</script>

<div class="page resources-page" class:bulk-open={mode !== null}>
	<PageHead
		icon="layout-list"
		title="Resources"
		description="Every resource you have, and where each one is listed."
		search={() => palette.show()}
	>
		{#snippet aside()}
			<Menu bind:open={bulkMenu} label="Bulk actions">
				{#snippet trigger()}
					<Button tier="primary" icon="ellipsis-vertical" onclick={() => (bulkMenu = !bulkMenu)}>
						Bulk actions
					</Button>
				{/snippet}
				{#each BULK_ACTIONS as action (action.verb)}
					<button
						type="button"
						disabled={action.missing !== null}
						title={action.missing ?? undefined}
						onclick={() => {
							bulkMenu = false;
							mode = action.verb;
							selected = new Set();
						}}
					>
						{action.label}…
					</button>
				{/each}
			</Menu>
		{/snippet}
	</PageHead>

	{#if read && (counts.attention ?? 0) > 0 && tab !== 'attention' && standing !== 'attention'}
		<Banner
			tone="warn"
			title={`${counts.attention} ${counts.attention === 1 ? 'resource needs' : 'resources need'} you`}
		>
			A marketplace is waiting on a sign-in, holding a send, or reporting a failure. Work for Tes
			and TPT runs on your own device, so nothing moves while that device is off.
			{#snippet action()}
				<Button
					onclick={() => {
						tab = 'attention';
						lastTab = 'attention';
					}}
				>
					Show only those
				</Button>
			{/snippet}
		</Banner>
	{/if}

	<Panel>
		<div class="res-filters">
			<Field label="Search" id="resource-search">
				<span class="res-search">
					<Icon name="search" size={16} />
					<input
						id="resource-search"
						type="search"
						placeholder="Search by title"
						value={box}
						oninput={(event) => typed(event.currentTarget.value)}
					/>
				</span>
			</Field>

			<div class="res-group">
				<span class="res-group-label" id="resource-marketplaces">Marketplaces</span>
				<div class="mk-tiles" role="group" aria-labelledby="resource-marketplaces">
					{#each MARKETPLACE_TILES as tile (tile.inventory)}
						<button
							class="mk-tile"
							type="button"
							disabled={tile.disabled}
							aria-pressed={marketplaces.includes(tile.inventory)}
							title={tile.reason ?? platformTitle(tile.inventory)}
							onclick={() => toggleMarketplace(tile.inventory)}
						>
							{tile.label}
						</button>
					{/each}
				</div>
			</div>

			<div class="res-group">
				<span class="res-group-label" id="resource-labels">Labels</span>
				<Menu bind:open={labelMenu} label="Filter by label">
					{#snippet trigger()}
						<Button onclick={() => (labelMenu = !labelMenu)}>
							{chosenLabels.length === 0 ? 'Any label' : `${chosenLabels.length} chosen`}
						</Button>
					{/snippet}
					<!-- Three arms, because a read in flight and a read that failed are
					     not the same answer as a seller who has made no labels. -->
					<div class="label-menu">
						{#if labels.isPending}
							<p class="none">Reading your labels…</p>
						{:else if labels.isError}
							<p class="none">Your labels could not be read, so none can be chosen here.</p>
						{:else}
							{#each labels.data ?? [] as one (one.name)}
								<!-- Every label the organisation holds, the marks an import
								     wrote among them: a seller looking for what they just
								     imported filters by the shop's own chip. -->
								<label>
									<input
										type="checkbox"
										checked={chosenLabels.includes(one.name)}
										onchange={() => toggleLabel(one.name)}
									/>
									<LabelChip name={one.name} colour={one.colour} system={one.system} />
								</label>
							{:else}
								<p class="none">No resource carries a label yet.</p>
							{/each}
						{/if}
					</div>
				</Menu>
			</div>

			<Field label="Standing" id="resource-standing">
				<select
					id="resource-standing"
					value={standing}
					onchange={(event) => (standing = event.currentTarget.value as StandingFilter)}
				>
					{#each STANDING_OPTIONS as option (option.value)}
						<option value={option.value}>{option.label}</option>
					{/each}
				</select>
			</Field>
		</div>

		{#if anythingSet}
			<div class="res-clear">
				<Button tier="quiet" small onclick={clearFilters}>Clear filters</Button>
			</div>
		{/if}
	</Panel>

	<TabBar {tabs} bind:current={tab} onselect={chooseTab} />

	<div class="res-toolbar">
		<!-- No figure until the reads are in: "viewing 0 resources" is what a
		     seller with an empty catalogue sees, so standing it in for a read that
		     failed states something false in a number nobody would doubt. -->
		<span class="eyebrow">
			{#if read}
				Viewing {rows.length}
				{rows.length === 1 ? 'resource' : 'resources'}
			{:else if reading}
				Reading your Resources
			{:else}
				Nothing counted
			{/if}
		</span>
		<span>
			<label class="sr-only" for="resource-sort">Order</label>
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

	{#if mode !== null && verb !== null}
		<div class="select-all">
			<input
				type="checkbox"
				aria-label="Select every resource shown"
				checked={allShownSelected}
				onchange={toggleAllShown}
			/>
			<Menu bind:open={scopeMenu} label="Selection scope" align="start">
				{#snippet trigger()}
					<Button
						tier="quiet"
						small
						icon="ellipsis-vertical"
						onclick={() => (scopeMenu = !scopeMenu)}
					>
						Scope
					</Button>
				{/snippet}
				<button
					type="button"
					onclick={() => {
						scopeMenu = false;
						selectAllShown();
					}}
				>
					Select the {shown.length} shown
				</button>
				<button
					type="button"
					onclick={() => {
						scopeMenu = false;
						selected = new Set(rows.map((row) => row.product.id));
					}}
				>
					Select all {rows.length} matching
				</button>
				<button
					type="button"
					onclick={() => {
						scopeMenu = false;
						selected = new Set();
					}}
				>
					Select none
				</button>
			</Menu>
			<span>Select the resources to {VERB_PHRASE[verb.verb]}</span>
		</div>
	{/if}

	{#if reading}
		<p class="res-note">Loading the catalogue…</p>
	{:else if unread.length > 0}
		<!-- Named rather than summarised, and the rows withheld rather than
		     drawn from what did arrive: a row built without the connections says
		     every marketplace needs a sign-in, and one built without the statuses
		     says a paused marketplace is running. -->
		<Banner tone="bad" title="Some of this page could not be loaded">
			What failed: {unread.join(', ')}. Your resources are not shown, because what we could
			draw would be wrong. Reload to try again.
		</Banner>
	{:else if allRows.length === 0 && !anythingSet}
		<Placeholder
			icon="layout-list"
			headline="Nothing in your Resources yet."
			body="Import brings your existing shop across as drafts you review. Creating one starts from a blank form."
		>
			{#snippet actions()}
				<Button
					tier="primary"
					href="/import"
					icon="download"
					disabled={importRefusal !== null}
					reason={importRefusal ?? undefined}
				>
					Import from a marketplace
				</Button>
				<span class="res-note">or</span>
				<Button
					href="/resources/new"
					icon="plus"
					disabled={createRefusal !== null}
					reason={createRefusal ?? undefined}
				>
					Create a resource
				</Button>
			{/snippet}
		</Placeholder>
	{:else if rows.length === 0}
		<Placeholder
			icon="search"
			headline="Nothing matches these filters."
			body="Your Resources are not empty; these filters are what is hiding them."
		>
			{#snippet actions()}
				<Button tier="quiet" onclick={clearFilters}>Clear filters</Button>
			{/snippet}
		</Placeholder>
	{:else}
		<div class="res-rows">
			{#each shown as row (row.product.id)}
				<RowCard
					href={`/resources/${row.product.id}`}
					title={row.product.title}
					meta={metaLine(row.product, row, now)}
					cover={row.product.cover ?? null}
					selectable={mode !== null}
					bind:selected={
						() => selected.has(row.product.id), (value) => setSelected(row.product.id, value)
					}
				>
					{#snippet strip()}
						<MarketplaceChips chips={row.chips} />
					{/snippet}
					{#snippet menu(close: () => void)}
						<a class="res-menu-link" href={`/resources/${row.product.id}`}>Open</a>
						<button type="button" disabled title={DUPLICATE_MISSING}>Duplicate</button>
						<button
							class="res-danger"
							type="button"
							onclick={() => {
								close();
								removing = row;
							}}
						>
							Delete
						</button>
					{/snippet}
				</RowCard>
			{/each}
		</div>

		{#if rows.length > shown.length}
			<div class="res-more">
				<Button onclick={() => (drawn += PAGE_STEP)}>
					Show {Math.min(PAGE_STEP, rows.length - shown.length)} more
				</Button>
			</div>
		{/if}

		<p class="res-foot">
			{#each unavailable() as action (action.verb)}
				<span class="block">{action.label} in bulk is not built. {action.missing}</span>
			{/each}
			{#if work.isError}
				<span class="block">
					What each marketplace is doing right now could not be read, so the chips show the last
					state recorded rather than a live one.
				</span>
			{/if}
		</p>
	{/if}

	{#if mode !== null && verb !== null}
		<div class="bulk-bar">
			<!-- `chosen.length` rather than `selected.size`: a selection survives a
			     filter change, and the label filter narrows the server's own query,
			     so a resource can be selected and no longer among the rows this
			     page holds. The dialog acts on `chosen`, so the figure beside the
			     verb is the one it will act on. -->
			<Button
				tier="additive"
				disabled={chosen.length === 0}
				reason={chosen.length === 0
					? 'Select at least one resource that is in view.'
					: undefined}
				onclick={() => run(verb.verb)}
			>
				{verb.label} ({chosen.length})
			</Button>
			<Button tier="quiet" onclick={() => (selected = new Set())}>Unselect all</Button>
			<Button onclick={cancelBulk}>Cancel</Button>
		</div>
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
/>

<DeleteDialog
	open={removing !== null}
	product={removing?.product.id ?? ''}
	title={removing?.product.title ?? ''}
	mappings={removing === null ? [] : [...removing.mapped.values()]}
	onClose={() => (removing = null)}
	onDeleted={deletedOne}
/>
