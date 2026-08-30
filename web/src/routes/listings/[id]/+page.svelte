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
	import { platformTitle } from '$lib/platforms';
	import DeleteDialog from '$lib/DeleteDialog.svelte';
	import { agoLabel } from '$lib/elapsed';
	import { createLedger, type Ledger } from '$lib/ledger';
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
	let runs = $state<{ inventory: InventoryId; job: string }[]>([]);

	// Seeded once rather than mirrored: a refetch arriving while the seller is
	// typing must not overwrite what they typed.
	$effect(() => {
		const stored = product.data;
		if (stored !== undefined && seed === null) {
			seed = editSeedOf(stored);
		}
	});

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
		runs = [...started, ...runs];
		void queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
		if (started.length === 1) {
			toast('info', 'Send started.');
			void goto(`/sync/${started[0].job}`);
			return;
		}
		toast('info', `Send started on ${started.length} marketplaces.`);
	}

	async function deleted() {
		deleting = false;
		await queryClient.invalidateQueries({ queryKey: queryKeys.products });
		await queryClient.invalidateQueries({ queryKey: queryKeys.mappings });
		toast('info', 'Listing deleted.');
		await goto('/listings');
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

		<Panel
			title="Marketplaces"
			description="Chosen when the draft was created. There is no way to add one afterwards, so this set is fixed."
		>
			{#each mappings as mapping (mapping.id)}
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
						<span class="s">{mapping.binding_state} · {mapping.lifecycle_state}</span>
					</span>
					<span class="grow"></span>
					<span class="pill {verdict.tone}">{verdict.line}</span>
				</div>
			{:else}
				<p class="quiet">This listing carries no marketplace mapping.</p>
			{/each}
		</Panel>

		{#if runs.length > 0}
			<Panel title="Sends started here" description="Each run carries its own outcome and steps.">
				{#each runs as run (run.job)}
					<a class="job" href={`/sync/${run.job}`}>
						<span class="what">
							<span class="t">{platformTitle(run.inventory)}</span>
							<span class="w mono">{run.job.slice(0, 8)}…</span>
						</span>
						<span class="when">open the run</span>
					</a>
				{/each}
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
