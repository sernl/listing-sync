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
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import PublishDialog from '$lib/PublishDialog.svelte';
	import { connectionFor, readinessOf } from '$lib/publish-readiness';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import type { InventoryId } from '$lib/generated/vocab';

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
	const inventories = $derived(mappings.map((mapping: MappingHead) => mapping.inventory));

	const vocabularies = createQueries(() => ({
		queries: inventories.map((inventory: InventoryId) => ({
			queryKey: queryKeys.vocabulary(inventory),
			queryFn: () => api.vocabulary(inventory),
			staleTime: Infinity
		})),
		combine: (results: { data?: VocabularyView }[]) =>
			new Map(
				results.flatMap((result) =>
					result.data === undefined
						? []
						: [[result.data.inventory, result.data] as [InventoryId, VocabularyView]]
				)
			)
	}));

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
	// Runs started from this page in this session, kept only until the bounded
	// read below carries them: a run the server has not listed yet would
	// otherwise vanish between starting it and the refetch landing.
	let justStarted = $state<{ inventory: InventoryId; job: string }[]>([]);

	// Seeded once rather than mirrored: a refetch arriving while the seller is
	// typing must not overwrite what they typed.
	$effect(() => {
		const stored = product.data;
		if (stored !== undefined && seed === null) {
			seed = editSeedOf(stored);
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

	// The marketplaces this console authors for that this item is not mapped
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
		return failure.message;
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

	/** Adds a marketplace this item does not reach, then opens the send it is
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
					? failure.message
					: `${platformTitle(inventory)} was not added.`;
		} finally {
			adding = null;
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

<div class="page">
	{#if product.isPending}
		<p class="quiet">Loading the listing…</p>
	{:else if product.isError || product.data === undefined}
		<PageHead icon="▤" title="Listing" description="This listing could not be read." />
		<Panel>
			<div class="placeholder">
				<span class="big" aria-hidden="true">⌕</span>
				<b>No such listing</b>
				<p>It may have been deleted. The catalogue still has everything else.</p>
			</div>
		</Panel>
	{:else}
		{@const stored = product.data}
		{@const status = rowStatus(mappings)}
		<PageHead
			icon="▤"
			title={stored.title}
			description={`${formatPrice(stored.price)} · updated ${agoLabel(stored.updated_at, Date.now())}`}
		>
			{#snippet aside()}
				<span class="pill {status.tone}">{status.label}</span>
				<span class="tag-note">{live ? 'live' : 'reconnecting…'}</span>
				<button class="cta" type="button" onclick={() => (publishing = true)}>Publish…</button>
				<button class="btn danger" type="button" onclick={() => (deleting = true)}>Delete…</button>
			{/snippet}
		</PageHead>

		{#if needing.length > 0}
			<div class="attn">
				<div class="t">
					{needing.length}
					{needing.length === 1 ? 'marketplace needs' : 'marketplaces need'} you
				</div>
				<p>
					{needing.map((chip) => platformTitle(chip.inventory)).join(', ')}. Nothing is sent to
					{needing.length === 1 ? 'it' : 'them'} until this is cleared, and the listing already
					there stands where it stood.
				</p>
				{#if needing[0].action}
					<a class="act" href={needing[0].action.href}>{needing[0].action.label}</a>
				{/if}
			</div>
		{/if}

		<Panel
			title="Marketplaces"
			description="Where this item stands on each one, and what it is waiting on. A marketplace the item was not created with can be added here."
		>
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
				<div class="row">
					<span class="what">
						<span class="t">{verdict.title}</span>
						<span class="s">{chip?.detail ?? `${mapping.binding_state} · ${mapping.lifecycle_state}`}</span>
						{#if chip?.paused}
							<span class="s">Sending is paused here: {chip.paused}</span>
						{/if}
						<span class="s">
							Next send: {verdict.line}.
							{#if chip?.onDevice}
								Work for this marketplace runs on your own device.
							{/if}
						</span>
					</span>
					<span class="grow"></span>
					{#if chip}
						<span class="pill {chip.tone}">{chip.label}</span>
						{#if chip.action}
							<a
								class="btn small"
								href={chip.action.href}
								target={chip.action.external ? '_blank' : undefined}
								rel={chip.action.external ? 'noopener noreferrer' : undefined}
							>{chip.action.label}</a>
						{/if}
					{:else}
						<span class="pill {verdict.tone}">{verdict.line}</span>
					{/if}
				</div>
			{:else}
				<p class="quiet">This item carries no marketplace mapping.</p>
			{/each}
			{#each unmapped as inventory (inventory)}
				<div class="row">
					<span class="what">
						<span class="t">{platformTitle(inventory)}</span>
						<span class="s">Not listed here, and this item has no mapping onto it.</span>
					</span>
					<span class="grow"></span>
					<span class="pill mut">Not listed</span>
					<button
						class="btn small"
						type="button"
						disabled={adding !== null}
						onclick={() => void crossListTo(inventory)}
					>
						{adding === inventory ? 'Adding…' : 'Cross-list here'}
					</button>
				</div>
			{/each}
			{#if addRefusal !== null}
				<p class="refusal">{addRefusal}</p>
			{/if}
			<p class="foot-note">
				Cross-listing here adds the marketplace to this item and opens the send; the add writes
				your own catalogue and contacts nobody, and nothing reaches the marketplace until that
				send runs. Etsy is not listed here, because this console has no create path for it at
				all.
			</p>
		</Panel>

		{#if runs.length > 0}
			<Panel
				title="Sends"
				description="Every run in the recent window that carried this item. Each opens its own timeline, with the steps, gates and events it recorded."
			>
				{#each runs as run (run.job)}
					<a class="job" href={`/sync/${run.job}`}>
						<span class="what">
							<span class="t">{platformTitle(run.inventory)}</span>
							<span class="w mono">{run.job.slice(0, 8)}…</span>
						</span>
						<span class="when">{run.state === null ? 'just started' : run.state}</span>
					</a>
				{/each}
				<p class="foot-note">
					Only the newest runs are read, so a send older than that window is not listed here.
					Sync holds every run.
				</p>
			</Panel>
		{/if}

		{#if blockedBy.length > 0}
			<div class="attn">
				<div class="t">This listing is live and cannot be edited through us</div>
				<p>
					{blockedBy.map(platformTitle).join(', ')} has a published listing, and neither editing a
					published listing nor taking one back to draft is a transition we have captured there.
					The fields below are shown as stored and the edit is held back rather than sent and
					refused.
				</p>
			</div>
		{/if}

		{#if seed}
			{@const current = seed}
			<form class="form wide" onsubmit={save}>
				<Panel
					title="The listing"
					description="The canonical fields. Every marketplace this listing is on takes them from here."
				>
					<label class="field">
						Title
						<input
							type="text"
							required
							maxlength="500"
							disabled={blockedBy.length > 0}
							value={current.title}
							oninput={(event) => (seed = { ...current, title: event.currentTarget.value })}
						/>
					</label>

					<label class="field" style="margin-top: 12px">
						Description
						{#if bodyCap}
							{@const used = measure(current.body, bodyCap.unit)}
							<span class="counter {used > bodyCap.limit ? 'over' : ''}">
								{used} of {bodyCap.limit}
							</span>
						{/if}
						<textarea
							disabled={blockedBy.length > 0}
							value={current.body}
							oninput={(event) => (seed = { ...current, body: event.currentTarget.value })}
						></textarea>
					</label>

					<div class="inline-choices" style="margin-top: 10px">
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
					</div>

					<div class="inline-choices" style="margin-top: 14px">
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
					</div>

					{#if current.branch === 'paid'}
						<div class="field-row" style="margin-top: 12px">
							<label class="field">
								Price
								<input
									type="text"
									inputmode="decimal"
									placeholder="4.50"
									disabled={blockedBy.length > 0}
									value={current.amount}
									oninput={(event) => (seed = { ...current, amount: event.currentTarget.value })}
								/>
							</label>
							<label class="field">
								Currency
								<select
									disabled={blockedBy.length > 0}
									value={current.currency}
									onchange={(event) =>
										(seed = { ...current, currency: event.currentTarget.value })}
								>
									{#each CURRENCY_OPTIONS as option (option.value)}
										<option value={option.value}>{option.code}</option>
									{/each}
								</select>
							</label>
						</div>
					{/if}

					{#if licensing.length > 0}
						<label class="field" style="margin-top: 12px">
							Licence
							<span class="hint">
								Gated on the price, so changing free or paid changes this list. A stated licence
								can be replaced here but not withdrawn.
							</span>
							<select
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
						</label>
					{/if}

					{#if editRefusal !== null}
						<p class="refusal">{editRefusal}</p>
					{/if}

					<div class="actions">
						<button class="cta" type="submit" disabled={saving || blockedBy.length > 0}>
							{saving ? 'Saving…' : 'Save changes'}
						</button>
						<span class="s">
							Saving writes the catalogue. It reaches a marketplace on the next send.
						</span>
					</div>
				</Panel>
			</form>
		{/if}

		<Panel title="Files" description="What a buyer downloads, plus the cover we generated.">
			{#each stored.files as file (file.id)}
				<div class="file-row">
					<span class="badge">{file.role}</span>
					<span class="name">{file.kind}</span>
					<span class="s">{file.scan}</span>
					<span class="when">{file.byte_len.toLocaleString('en-GB')} bytes</span>
				</div>
			{:else}
				<p class="quiet">No files recorded.</p>
			{/each}
			<p class="foot-note">
				Files are set when the draft is created. Replacing them is not something this console does
				yet.
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
