<script lang="ts">
	// One collection: what it is called, what is in it and in what order, and
	// the three verbs that act on the whole set.
	//
	// Two columns, as the migration and scheduling screens are: what is being
	// acted on on the left, the verbs and their previews on the right. The
	// members are the left column because they are what every verb on the right
	// takes, and a seller reordering a set has to see the order the publish will
	// use while they do it.
	//
	// Every member write is `PUT /v1/collections/{id}/members` with the whole
	// ordered set, so a reorder, a removal and an addition are one call and the
	// page never holds two ways of saying what order the collection is in. The
	// arithmetic is `./collections`, which is tested; this component holds the
	// reads, the sends and the words.

	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import {
		ApiFailure,
		allPages,
		api,
		collectionsApi,
		type MappingHead,
		type ProductHead
	} from '$lib/api';
	import ApplyTemplateDialog from '$lib/ApplyTemplateDialog.svelte';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import type { InventoryId } from '$lib/generated/vocab';
	import { INVENTORY_ORDER, normaliseQuery } from '$lib/listings-view';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import Note from '$lib/Note.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { exportPath } from '$lib/pages/export/api';
	import {
		failureMessage,
		filenameFrom,
		saveDocument,
		type CsvDocument
	} from '$lib/pages/export/download';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import CollectionPublishDialog from './CollectionPublishDialog.svelte';
	import {
		COLLECTIONS_KEY,
		DESCRIPTION_MAX_CHARS,
		NAME_MAX_CHARS,
		added,
		checkName,
		collectionKey,
		countLine,
		marksOf,
		moved
	} from './collections';
	import './collections.css';

	const queryClient = useQueryClient();
	const id = $derived(page.params.id ?? '');

	const collection = createQuery(() => ({
		queryKey: collectionKey(id),
		queryFn: () => collectionsApi.get(id),
		enabled: id.length > 0
	}));

	// The whole catalogue and every mapping, for the Add resources checklist:
	// the same two reads the migration page's own tick list takes, and the same
	// cache entries, so opening this page after that one costs nothing.
	const catalogue = createQuery(() => ({
		queryKey: queryKeys.catalogue(null),
		queryFn: () =>
			allPages(
				(cursor) => api.products(cursor, null),
				(view) => view.products
			)
	}));
	const mappings = createQuery(() => ({
		queryKey: queryKeys.mappings,
		queryFn: () => api.mappings().then((view) => view.mappings)
	}));

	/** Whether the collection is actually absent rather than unreadable. Only a
	 *  404 says so; every other failure is a fault on our side or in between,
	 *  and telling a seller their collection is gone over a bad minute is the
	 *  worse of the two wrong answers. */
	const gone = $derived(
		collection.error instanceof ApiFailure && collection.error.status === 404
	);
	const members = $derived(collection.data?.members ?? []);
	const order = $derived(members.map((member) => member.product));
	const held = $derived(new Set(order));

	let renaming = $state(false);
	let name = $state('');
	let description = $state('');
	let saving = $state(false);
	let headerRefusal = $state<string | null>(null);

	let adding = $state(false);
	let box = $state('');
	let ticked = $state<Set<string>>(new Set());
	let memberRefusal = $state<string | null>(null);
	let writing = $state(false);

	let publishing = $state(false);
	let applying = $state(false);
	let labels = $state('');
	let labelling = $state(false);
	let labelRefusal = $state<string | null>(null);

	let exporting = $state(false);
	let exportFailure = $state<string | null>(null);

	const products = $derived<ProductHead[]>(catalogue.data ?? []);
	const marksOfProduct = $derived.by(() => {
		const index = new Map<string, InventoryId[]>();
		for (const mapping of (mappings.data ?? []) as MappingHead[]) {
			index.set(mapping.product, [...(index.get(mapping.product) ?? []), mapping.inventory]);
		}
		return index;
	});

	const query = $derived(normaliseQuery(box).toLocaleLowerCase());
	// Only what the collection does not already hold: a checklist offering a
	// member back would be a tick that does nothing, and `added` keeps such a
	// product where it is rather than moving it to the end.
	const offerable = $derived(
		products
			.filter((product) => !held.has(product.id))
			.filter(
				(product) =>
					query.length === 0 || product.title.toLocaleLowerCase().includes(query)
			)
	);
	const allShownTicked = $derived(
		offerable.length > 0 && offerable.every((product) => ticked.has(product.id))
	);

	const verdict = $derived(checkName(name));
	const headerBlocked = $derived.by(() => {
		if (saving) {
			return 'Saving the collection.';
		}
		return verdict.accepted ? undefined : verdict.message;
	});

	function startRename() {
		const stored = collection.data;
		if (stored === undefined) {
			return;
		}
		name = stored.name;
		description = stored.description ?? '';
		headerRefusal = null;
		renaming = true;
	}

	async function refresh(): Promise<unknown> {
		return Promise.all([
			queryClient.invalidateQueries({ queryKey: collectionKey(id) }),
			queryClient.invalidateQueries({ queryKey: COLLECTIONS_KEY })
		]);
	}

	async function saveHeader() {
		if (!verdict.accepted) {
			return;
		}
		saving = true;
		headerRefusal = null;
		try {
			await collectionsApi.update(id, {
				name: verdict.name,
				description: description.trim().length === 0 ? null : description.trim()
			});
			renaming = false;
			await refresh();
			toast('info', 'Saved.');
		} catch (failure) {
			headerRefusal =
				failure instanceof ApiFailure
					? failure.message
					: 'We couldn’t save the collection.';
		} finally {
			saving = false;
		}
	}

	/** Writes the whole ordered set. Every member control goes through here, so
	 *  a reorder and a removal cannot come to be written two different ways. */
	async function writeOrder(next: readonly string[], said: string) {
		writing = true;
		memberRefusal = null;
		try {
			await collectionsApi.setMembers(id, next);
			await refresh();
			toast('info', said);
		} catch (failure) {
			memberRefusal =
				failure instanceof ApiFailure
					? failure.message
					: 'We couldn’t change the collection.';
		} finally {
			writing = false;
		}
	}

	function tick(product: string, value: boolean) {
		const next = new Set(ticked);
		if (value) {
			next.add(product);
		} else {
			next.delete(product);
		}
		ticked = next;
	}

	async function addTicked() {
		const chosen = offerable
			.filter((product) => ticked.has(product.id))
			.map((product) => product.id);
		if (chosen.length === 0) {
			return;
		}
		await writeOrder(
			added(order, chosen),
			`${chosen.length} ${chosen.length === 1 ? 'resource' : 'resources'} added.`
		);
		ticked = new Set();
	}

	async function addLabels() {
		// Split on commas, because the field takes several at once and a
		// collection is where a seller files a whole set under the same words.
		const asked = labels
			.split(',')
			.map((one) => one.trim())
			.filter((one) => one.length > 0);
		if (asked.length === 0) {
			return;
		}
		labelling = true;
		labelRefusal = null;
		try {
			const ack = await collectionsApi.addLabels(id, asked);
			labels = '';
			await Promise.all([
				queryClient.invalidateQueries({ queryKey: queryKeys.labels }),
				queryClient.invalidateQueries({ queryKey: queryKeys.products })
			]);
			toast(
				'info',
				`${ack.added.join(', ')} added to ${ack.members} ${ack.members === 1 ? 'resource' : 'resources'}.`
			);
		} catch (failure) {
			labelRefusal =
				failure instanceof ApiFailure
					? failure.message
					: 'We couldn’t add the labels.';
		} finally {
			labelling = false;
		}
	}

	/** The same handover the Export page makes, narrowed to this collection.
	 *  An anchor carrying `download` rather than a navigation, because in the
	 *  desktop application a navigation replaces the whole window; that is
	 *  `saveDocument`'s own reasoning and this page borrows it rather than
	 *  writing a second one. */
	async function exportCollection() {
		exporting = true;
		exportFailure = null;
		try {
			const response = await fetch(exportPath({ collection: id }), {
				headers: { accept: 'text/csv' }
			});
			if (!response.ok) {
				let body = null;
				try {
					body = await response.json();
				} catch {
					body = null;
				}
				throw new ApiFailure(response.status, body);
			}
			const csv: CsvDocument = {
				blob: await response.blob(),
				filename: filenameFrom(response.headers.get('content-disposition'))
			};
			saveDocument(csv);
			toast('info', `Saved as ${csv.filename}.`);
		} catch (failure) {
			exportFailure = failureMessage(failure);
		} finally {
			exporting = false;
		}
	}

	async function published(queued: number, job: string | null) {
		publishing = false;
		await Promise.all([
			refresh(),
			queryClient.invalidateQueries({ queryKey: queryKeys.mappings }),
			queryClient.invalidateQueries({ queryKey: queryKeys.inventoryWork })
		]);
		if (queued === 0 || job === null) {
			toast(
				'info',
				'Nothing to publish: every resource is already there or can’t be published yet.'
			);
			return;
		}
		toast('info', `Publishing ${queued} ${queued === 1 ? 'resource' : 'resources'}.`);
	}

	async function applied() {
		applying = false;
		await Promise.all([
			refresh(),
			queryClient.invalidateQueries({ queryKey: queryKeys.products })
		]);
	}
</script>

<div class="page collections-page">
	{#if collection.isPending}
		<p class="quiet">Loading the collection…</p>
	{:else if gone}
		<PageHead
			icon="layers"
			title="Collection"
			description="We can’t find this collection."
			back={{ href: '/collections', label: 'Back to Collections' }}
		/>
		<Placeholder
			icon="search"
			headline="Collection not found"
			body="It may have been deleted."
		>
			{#snippet actions()}
				<Button href="/collections" icon="layers">Back to Collections</Button>
			{/snippet}
		</Placeholder>
	{:else if collection.isError || collection.data === undefined}
		<PageHead
			icon="layers"
			title="Collection"
			description="We couldn’t load this collection."
			back={{ href: '/collections', label: 'Back to Collections' }}
		/>
		<Placeholder
			icon="triangle-alert"
			headline="We couldn’t load this collection"
			body="Reload the page to try again."
		>
			{#snippet actions()}
				<Button href="/collections" icon="layers">Back to Collections</Button>
			{/snippet}
		</Placeholder>
	{:else}
		{@const stored = collection.data}
		<PageHead
			icon="layers"
			title={stored.name}
			description={stored.description ?? countLine(stored.count)}
			back={{ href: '/collections', label: 'Back to Collections' }}
		>
			{#snippet aside()}
				<Button onclick={startRename}>Rename…</Button>
			{/snippet}
		</PageHead>

		{#if renaming}
			<Panel title="Name and description">
				<div class="coll-form">
					<Field label="Name" id="collection-name" required>
						<input
							id="collection-name"
							type="text"
							maxlength={NAME_MAX_CHARS}
							disabled={saving}
							bind:value={name}
						/>
					</Field>
					<Field
						label="Description"
						id="collection-description"
						hint="Only you see this; it never goes to a marketplace."
					>
						<textarea
							id="collection-description"
							rows="2"
							maxlength={DESCRIPTION_MAX_CHARS}
							disabled={saving}
							bind:value={description}
						></textarea>
					</Field>
				</div>
				{#if headerRefusal !== null}
					<Banner tone="bad">{headerRefusal}</Banner>
				{/if}
				<div class="coll-form-foot">
					<Button
						tier="primary"
						disabled={headerBlocked !== undefined}
						reason={headerBlocked}
						onclick={() => void saveHeader()}
					>
						{saving ? 'Saving…' : 'Save'}
					</Button>
					<Button tier="quiet" onclick={() => (renaming = false)}>Cancel</Button>
				</div>
			</Panel>
		{/if}

		<div class="coll-body">
			<div>
				<Panel
					title="Resources in this collection"
					description="Publishing, templates, labels and export all follow this order."
				>
					{#snippet more()}
						<!-- Which marketplaces this set reaches, as one strip rather than
						     a count per member: it is the fact a seller checks before
						     publishing it somewhere new. -->
						<span class="coll-marks">
							{#each marksOf(members, INVENTORY_ORDER) as inventory (inventory)}
								<MarketplaceMark {inventory} size={18} />
							{/each}
						</span>
					{/snippet}
					{#if members.length === 0}
						<p class="quiet">This collection is empty.</p>
					{:else}
						{#each members as member, index (member.product)}
							<div class="coll-member">
								<span class="coll-pos">{index + 1}</span>
								<span class="coll-member-title">
									<a href={`/resources/${member.product}`}>{member.title}</a>
									<span class="coll-member-marks">
										{#each member.inventories as inventory (inventory)}
											<MarketplaceMark {inventory} size={16} />
										{/each}
									</span>
								</span>
								<span class="coll-member-acts">
									<Button
										tier="quiet"
										small
										disabled={index === 0 || writing}
										reason={index === 0
											? 'This resource is already first.'
											: writing
												? 'Saving the order.'
												: undefined}
										onclick={() =>
											void writeOrder(moved(order, member.product, 'up'), 'Order saved.')}
									>
										Move up
									</Button>
									<Button
										tier="quiet"
										small
										disabled={index === members.length - 1 || writing}
										reason={index === members.length - 1
											? 'This resource is already last.'
											: writing
												? 'Saving the order.'
												: undefined}
										onclick={() =>
											void writeOrder(moved(order, member.product, 'down'), 'Order saved.')}
									>
										Move down
									</Button>
									<Button
										tier="quiet"
										small
										danger
										disabled={writing}
										reason={writing ? 'Saving the collection.' : undefined}
										onclick={() =>
											void writeOrder(
												order.filter((product) => product !== member.product),
												`Removed ${member.title} from this collection.`
											)}
									>
										Remove
									</Button>
								</span>
							</div>
						{/each}
					{/if}

					{#if memberRefusal !== null}
						<Banner tone="bad">{memberRefusal}</Banner>
					{/if}

					<Note>Removing a resource only takes it out of this collection.</Note>
				</Panel>

				<Panel title="Add resources">
					{#if !adding}
						<Button tier="additive" icon="plus" onclick={() => (adding = true)}>
							Add resources
						</Button>
					{:else if catalogue.isPending || mappings.isPending}
						<p class="quiet">Loading your resources…</p>
					{:else if catalogue.isError || mappings.isError}
						<p class="quiet">We couldn’t load your resources. Reload the page to try again.</p>
					{:else if products.length === 0}
						<p class="quiet">Import your shop first, then add resources here.</p>
					{:else}
						<div class="pick-head">
							<input
								type="search"
								aria-label="Search your resources"
								placeholder="Search resources"
								bind:value={box}
							/>
							<label class="pick-all">
								<input
									type="checkbox"
									checked={allShownTicked}
									onchange={() =>
										(ticked = allShownTicked
											? new Set()
											: new Set(offerable.map((product) => product.id)))}
								/>
								Select all {offerable.length} shown
							</label>
							<span class="pick-count">{ticked.size} selected</span>
						</div>

						{#if offerable.length === 0}
							<p class="quiet">
								{query.length === 0
									? 'Every resource you have is already in this collection.'
									: 'Nothing matches that search.'}
							</p>
						{:else}
							<div class="pick-list">
								{#each offerable as product (product.id)}
									<label class="pick-row">
										<input
											type="checkbox"
											checked={ticked.has(product.id)}
											onchange={(event) => tick(product.id, event.currentTarget.checked)}
										/>
										<span class="pick-title">{product.title}</span>
										<span class="coll-member-marks">
											{#each marksOfProduct.get(product.id) ?? [] as inventory (inventory)}
												<MarketplaceMark {inventory} size={16} />
											{/each}
										</span>
									</label>
								{/each}
							</div>
						{/if}

						<div class="coll-verb-foot">
							<Button
								tier="additive"
								disabled={ticked.size === 0 || writing}
								reason={ticked.size === 0
									? 'Select at least one resource.'
									: writing
										? 'Saving the collection.'
										: undefined}
								onclick={() => void addTicked()}
							>
								{writing ? 'Adding…' : `Add ${ticked.size}`}
							</Button>
							<Button tier="quiet" onclick={() => (adding = false)}>Done</Button>
						</div>
					{/if}
				</Panel>
			</div>

			<div class="coll-right">
				<Panel
					title="Publish to a marketplace"
					description="Publishes every resource not already on that marketplace, in this order."
				>
					<Button
						tier="primary"
						disabled={stored.count === 0}
						reason={stored.count === 0
							? 'Add resources to this collection first.'
							: undefined}
						onclick={() => (publishing = true)}
					>
						Publish…
					</Button>
					<Note>Resources already on that marketplace are skipped, not published twice.</Note>
				</Panel>

				<Panel
					title="Apply a template and labels"
					description="Fill every resource here from one template."
				>
					<Button
						disabled={stored.count === 0}
						reason={stored.count === 0
							? 'Add resources to this collection first.'
							: undefined}
						onclick={() => (applying = true)}
					>
						Apply a template…
					</Button>

					<Field
						label="Add labels"
						id="collection-labels"
						hint="Separate labels with commas."
					>
						<input
							id="collection-labels"
							type="text"
							placeholder="Autumn term, Bundle"
							disabled={labelling || stored.count === 0}
							bind:value={labels}
						/>
					</Field>
					<div class="coll-verb-foot">
						<Button
							tier="additive"
							disabled={labels.trim().length === 0 || labelling || stored.count === 0}
							reason={stored.count === 0
								? 'Add resources to this collection first.'
								: labels.trim().length === 0
									? 'Type a label first.'
									: labelling
										? 'Adding the labels.'
										: undefined}
							onclick={() => void addLabels()}
						>
							{labelling ? 'Adding…' : `Add to ${countLine(stored.count)}`}
						</Button>
					</div>
					{#if labelRefusal !== null}
						<Banner tone="bad">{labelRefusal}</Banner>
					{/if}
				</Panel>

				<Panel
					title="Export"
					description="Download a spreadsheet of just this collection."
				>
					<Button
						icon="file-down"
						disabled={stored.count === 0 || exporting}
						reason={stored.count === 0
							? 'Add resources to this collection first.'
							: exporting
								? 'Preparing the spreadsheet.'
								: undefined}
						onclick={() => void exportCollection()}
					>
						{exporting ? 'Preparing…' : 'Export CSV'}
					</Button>
					{#if exportFailure !== null}
						<Banner tone="bad">{exportFailure}</Banner>
					{/if}
					<Note>Includes resource details only, not your files or marketplace logins.</Note>
				</Panel>
			</div>
		</div>

		<CollectionPublishDialog
			open={publishing}
			collection={id}
			name={stored.name}
			count={stored.count}
			onClose={() => (publishing = false)}
			onPublished={published}
		/>

		<ApplyTemplateDialog
			open={applying}
			products={order}
			onClose={() => (applying = false)}
			onDone={applied}
		/>
	{/if}
</div>
