<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api, type LabelView, type ProductHead } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import FlowActionBar from '$lib/FlowActionBar.svelte';
	import { entitlementRead, limitOf } from '$lib/entitlement-read';
	import Icon from '$lib/Icon.svelte';
	import { formatPrice } from '$lib/listings-view';
	import Menu from '$lib/Menu.svelte';
	import MenuItem from '$lib/MenuItem.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Pagination from '$lib/Pagination.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import { LABEL_COUNTS_KEY, countAll, deleteLabel, labelResources, renameLabel } from './api';
	import {
		LABEL_MAX_CHARS,
		checkRename,
		countLine,
		deleteWarning,
		filterHref,
		matching,
		rows,
		swatchClass,
		totalLine,
		unchanged
	} from './labels-view';
	import '$lib/flow.css';
	import './labels.css';

	const queryClient = useQueryClient();

	const labels = createQuery(() => ({
		queryKey: queryKeys.labels,
		queryFn: () => api.labels().then((view) => view.labels)
	}));

	// The label cap, from the plan the shell already read. A marketplace's own
	// automatic labels are excluded from the count on the server's side, so
	// this figure is the seller's own vocabulary and not what a connection
	// added to it.
	const plan = createQuery(() => entitlementRead);
	const capped = $derived(limitOf(plan.data, 'labels'));

	/** The vocabulary the counts are taken over, read once so the key and the
	 *  walk cannot be built from two different readings of it. */
	const names = $derived((labels.data ?? []).map((label) => label.name));

	// The counts are a second read on purpose: the names render as soon as the
	// vocabulary lands, so a catalogue that is slow costs each chip its figure
	// rather than costing the page its list.
	//
	// The vocabulary is part of the key rather than only of the answer. A rename
	// invalidates both reads at once, and a counts refetch that ran before the
	// renamed list arrived would count the names that are gone and then hold
	// that answer under a key nothing else invalidates.
	//
	// `staleTime` is what stops a refocus repeating the whole pass: each figure
	// costs a walk of the catalogue narrowed to that label.
	const COUNTS_FRESH_MS = 5 * 60 * 1000;
	const counts = createQuery(() => ({
		// Joined on a newline, which no label can contain.
		queryKey: [...LABEL_COUNTS_KEY, names.join('\n')],
		enabled: labels.isSuccess,
		staleTime: COUNTS_FRESH_MS,
		queryFn: () => countAll(names)
	}));

	let search = $state('');
	/** Closed for the visit alone: every label is still listed, it is the
	 *  counts beside them that are missing. */
	let uncountedShown = $state(true);
	let editing = $state<string | null>(null);
	let draft = $state('');
	/** The server's refusal, and the draft it was raised against, so the
	 *  sentence leaves the moment the seller edits the name. */
	let refusal = $state<{ message: string; draft: string } | null>(null);
	let confirming = $state<string | null>(null);
	/** Which chip's menu is open, by label name. */
	let menus = $state<Record<string, boolean>>({});
	let nameField = $state<HTMLInputElement | null>(null);
	let confirmPanel = $state<HTMLElement | null>(null);

	$effect(() => {
		nameField?.focus();
	});

	// A confirmation that opens below the fold is one the seller never saw:
	// focus moves into it, and the scroll brings it clear of the phone's
	// fixed tab bar, which `scroll-margin-block` in the sheet reserves.
	$effect(() => {
		const panel = confirmPanel;
		if (panel !== null) {
			panel.scrollIntoView({ block: 'nearest' });
			panel.focus();
		}
	});

	const counting = $derived(counts.isPending || counts.isFetching);
	const all = $derived(rows(labels.data ?? [], counts.data?.counts ?? new Map()));
	const shown = $derived(matching(all, search));
	const uncounted = $derived(counts.isError || (counts.data?.failed.length ?? 0) > 0);
	const verdict = $derived(checkRename(draft, editing ?? '', names));
	const stale = $derived(editing !== null && unchanged(draft, editing));

	/** Both reads move on a write. So do the catalogue's label-narrowed entries,
	 *  which a rename respells and a delete empties — but not the unnarrowed
	 *  catalogue, whose rows carry no label and so cannot have changed. */
	function refresh(): Promise<unknown> {
		return Promise.all([
			queryClient.invalidateQueries({ queryKey: queryKeys.labels }),
			queryClient.invalidateQueries({ queryKey: LABEL_COUNTS_KEY }),
			queryClient.invalidateQueries({
				queryKey: queryKeys.products,
				predicate: (query) => query.queryKey.length > queryKeys.products.length
			})
		]);
	}

	const renaming = createMutation(() => ({
		mutationFn: (change: { from: string; to: string }) => renameLabel(change.from, change.to),
		onSuccess: async (stored: LabelView) => {
			editing = null;
			refusal = null;
			await refresh();
			toast('info', `Renamed to ${stored.name}.`);
		},
		onError: (failure: Error) => {
			// A 422 is about the name in the field, so it is answered beside the
			// field. Anything else is about the request and goes to the toasts.
			if (failure instanceof ApiFailure && failure.status === 422) {
				refusal = { message: failure.message, draft };
				return;
			}
			toast('error', 'We couldn’t rename the label.');
		}
	}));

	const deleting = createMutation(() => ({
		mutationFn: (name: string) => deleteLabel(name),
		onSuccess: async (_answer: void, name: string) => {
			confirming = null;
			await refresh();
			toast('info', `Deleted ${name}.`);
		},
		onError: () => {
			toast('error', 'We couldn’t delete the label.');
		}
	}));

	function startRename(name: string) {
		confirming = null;
		refusal = null;
		draft = name;
		editing = name;
	}

	function startDelete(name: string) {
		editing = null;
		confirming = name;
	}

	function submitRename(event: SubmitEvent) {
		event.preventDefault();
		if (editing === null || !verdict.accepted || stale) {
			return;
		}
		renaming.mutate({ from: editing, to: verdict.name });
	}

	/** What is wrong with the name in the rename field, whether this page
	 *  decided it or the server did. */
	const problem = $derived.by(() => {
		if (refusal !== null && refusal.draft === draft) {
			return refusal.message;
		}
		return verdict.accepted ? null : verdict.message;
	});

	/** Why Save is greyed when nothing is wrong: it is the name the label
	 *  already has. Muted rather than red. */
	const note = $derived(problem === null && stale ? 'The label already has this name.' : null);

	const blocked = $derived.by(() => {
		if (renaming.isPending) {
			return 'Saving the new name.';
		}
		if (note !== null) {
			return note;
		}
		return problem ?? undefined;
	});

	// --- creating one ---------------------------------------------------------

	// There is no empty label to create: a label exists exactly while a
	// resource carries it, which is what `GET /v1/labels` answers. So a new
	// label is a name and the resources that carry it, written in one press.
	const PICK_PER_PAGE = 25;
	let newName = $state('');
	let newField = $state<HTMLInputElement | null>(null);
	let picked = $state<Set<string>>(new Set());
	let products = $state<ProductHead[]>([]);
	let pickPage = $state(1);
	let pickCursors = $state<(string | null)[]>([null]);
	let pickNext = $state<string | null>(null);
	let pickBusy = $state(false);
	let pickLoaded = $state(false);
	let pickUnread = $state(false);
	let creating = $state(false);
	let createRefusal = $state<string | null>(null);

	const newVerdict = $derived(checkRename(newName, '', names));
	const naming = $derived(newName.trim().length > 0);
	const createBlocked = $derived.by(() => {
		if (capped !== null) {
			return capped;
		}
		if (creating) {
			return 'Adding the label.';
		}
		if (!newVerdict.accepted) {
			return newVerdict.message;
		}
		if (picked.size === 0) {
			return 'Tick at least one resource.';
		}
		return null;
	});

	async function readProducts(cursor: string | null, page: number) {
		pickBusy = true;
		try {
			const view = await api.products(cursor, null, PICK_PER_PAGE);
			products = view.products;
			pickNext = view.next_cursor;
			pickPage = page;
			pickCursors = [...pickCursors.slice(0, page), view.next_cursor];
			pickUnread = false;
		} catch {
			pickUnread = true;
		} finally {
			pickBusy = false;
			pickLoaded = true;
		}
	}

	// The tick list is read once a name is typed, not on arrival: most visits
	// here are to look at the labels, not to make one.
	$effect(() => {
		if (naming && !pickLoaded && !pickBusy) {
			void readProducts(null, 1);
		}
	});

	function pick(product: string, on: boolean) {
		const next = new Set(picked);
		if (on) {
			next.add(product);
		} else {
			next.delete(product);
		}
		picked = next;
	}

	async function createLabel() {
		if (createBlocked !== null || !newVerdict.accepted) {
			return;
		}
		const name = newVerdict.name;
		creating = true;
		createRefusal = null;
		try {
			const done = await labelResources([...picked], name, {
				read: (product) => api.productLabels(product),
				write: (product, held) => api.setProductLabels(product, held)
			});
			if (done.count > 0) {
				await refresh();
			}
			if (done.refusal === null) {
				toast('info', `Added ${name} to ${countLine(done.count, false)}.`);
				newName = '';
				picked = new Set();
			} else {
				createRefusal =
					done.count === 0
						? done.refusal
						: `Labelled ${done.count} of ${picked.size}, then it stopped: ${done.refusal}`;
			}
		} finally {
			creating = false;
		}
	}

	function startNew() {
		if (naming && createBlocked === null) {
			void createLabel();
			return;
		}
		newField?.scrollIntoView({ block: 'center', behavior: 'smooth' });
		newField?.focus();
	}
</script>

<div class="page flow-page has-bar labels-page">
	<PageHead
		icon="tag"
		title="Labels"
		description="Sort your resources with your own words."
		guide="labels-and-collections"
	>
		{#snippet aside()}
			<Explain title="How labels drive schedules" label="How labels work">
				<p>
					A label is a word you put on resources, like a term, a bundle or a sale. Only you see
					it. Each resource can carry up to twenty.
				</p>
				<p>
					A schedule can send “everything labelled Autumn term”. It looks up the label each
					time it runs, so a resource you label later goes out on the next run, and one you
					take the label off stays behind.
				</p>
				<p>
					Labels also narrow the Resources list and Analytics. Labels an import adds, like
					TPT or Tes, are kept for you and can’t be renamed or deleted.
				</p>
				<p><a href="/automations/sharing">Open Schedules</a></p>
			</Explain>
		{/snippet}
	</PageHead>

	<div class="flow">
		<section class="flow-card lb-new" aria-labelledby="lb-new-title">
			<div class="flow-card-head">
				<h2 id="lb-new-title" class="lb-new-title">
					<Icon name="plus" size={18} />
					New label
				</h2>
			</div>
			<div class="lb-new-row">
				<Field label="Name" id="lb-new-name" hint={`Up to ${LABEL_MAX_CHARS} characters.`}>
					<input
						id="lb-new-name"
						type="text"
						placeholder="Autumn term"
						autocomplete="off"
						disabled={capped !== null || creating}
						bind:this={newField}
						bind:value={newName}
					/>
				</Field>
			</div>
			{#if capped !== null}
				<p class="flow-warn">{capped}</p>
			{:else if naming && !newVerdict.accepted}
				<p class="row-refusal">{newVerdict.message}</p>
			{/if}

			{#if naming && capped === null}
				<div class="flow-pick">
					<p class="flow-label">Put it on</p>
					{#if !pickLoaded}
						<p class="quiet">Loading your resources…</p>
					{:else if pickUnread && products.length === 0}
						<Banner tone="bad">
							Your resources could not be loaded.
							{#snippet action()}
								<Button small onclick={() => void readProducts(pickCursors[pickPage - 1] ?? null, pickPage)}>
									Try again
								</Button>
							{/snippet}
						</Banner>
					{:else if products.length === 0 && pickPage === 1}
						<p class="quiet">Import your shop first, then label resources here.</p>
					{:else}
						<div class="pick-list">
							{#each products as product (product.id)}
								<label class="pick-row">
									<input
										type="checkbox"
										checked={picked.has(product.id)}
										disabled={creating}
										onchange={(event) => pick(product.id, event.currentTarget.checked)}
									/>
									<span class="pick-title">{product.title}</span>
									<span class="pick-price">{formatPrice(product.price)}</span>
								</label>
							{/each}
						</div>
						<Pagination
							page={pickPage}
							hasNext={pickNext !== null}
							busy={pickBusy}
							label="Your resources"
							summary={`${picked.size} ticked`}
							onprevious={() => void readProducts(pickCursors[pickPage - 2] ?? null, pickPage - 1)}
							onnext={() => void readProducts(pickNext, pickPage + 1)}
						/>
					{/if}
				</div>
			{/if}

			{#if createRefusal !== null}
				<Banner tone="bad">{createRefusal}</Banner>
			{/if}

			<div class="flow-actions lb-new-act">
				<Button
					tier="primary"
					icon="plus"
					disabled={createBlocked !== null}
					reason={createBlocked ?? undefined}
					onclick={() => void createLabel()}
				>
					{creating
						? 'Adding…'
						: picked.size === 0
							? 'Create label'
							: `Create label on ${countLine(picked.size, false)}`}
				</Button>
			</div>
		</section>

		<section class="flow-section" aria-labelledby="lb-yours-title">
			<div class="flow-section-head">
				<h2 id="lb-yours-title">Your labels</h2>
				{#if all.length > 0}
					<span class="labels-total">{totalLine(all.length)}</span>
				{/if}
			</div>

			{#if labels.isPending}
				<p class="quiet">Loading…</p>
			{:else if labels.isError}
				<Banner tone="bad" title="We couldn’t load your labels">
					Nothing has changed.
					{#snippet action()}
						<Button onclick={() => labels.refetch()}>Try again</Button>
					{/snippet}
				</Banner>
			{:else if all.length === 0}
				<Placeholder icon="tag" headline="No labels yet." body="Name one above and tick where it goes." />
			{:else}
				{#if all.length > 8}
					<span class="labels-search">
						<Icon name="search" size={16} />
						<input
							id="labels-search"
							type="search"
							aria-label="Search labels"
							placeholder="Search labels"
							bind:value={search}
						/>
					</span>
				{/if}

				{#if uncounted && uncountedShown}
					<Banner
						tone="warn"
						title="We couldn’t count every label"
						onDismiss={() => (uncountedShown = false)}
					>
						All your labels are still listed.
						{#snippet action()}
							<Button onclick={() => counts.refetch()}>Try again</Button>
						{/snippet}
					</Banner>
				{/if}

				{#if shown.length === 0}
					<p class="quiet labels-nomatch">
						Nothing matches that search.
						<Button tier="quiet" small onclick={() => (search = '')}>Clear search</Button>
					</p>
				{:else}
					<ul class="lb-chips">
						{#each shown as row (row.name)}
							<li
								class="lb-chip {swatchClass(row.colour)}"
								class:open={editing === row.name || confirming === row.name}
							>
								<a class="lb-chip-main" href={filterHref(row.name)} title="Show resources labelled {row.name}">
									<span class="lb-swatch" aria-hidden="true"></span>
									<span class="lb-name">{row.name}</span>
									<span class="lb-count" class:pending={row.count === null}>
										{row.count ?? (counting ? '…' : '?')}
										<span class="sr-only">{countLine(row.count, counting)}</span>
									</span>
								</a>
								<!-- A system label is an import's own mark: the seller
								     neither made it nor can rename or delete it, and both
								     routes answer 404 for one. No menu rather than an empty
								     one. -->
								{#if !row.system}
									<Menu label="Actions for {row.name}" bind:open={menus[row.name]} align="end">
										{#snippet trigger()}
											<button
												class="lb-kebab"
												type="button"
												aria-haspopup="menu"
												aria-expanded={menus[row.name] ?? false}
												aria-label="Actions for {row.name}"
												onclick={() => (menus[row.name] = !menus[row.name])}
											>
												<Icon name="ellipsis-vertical" size={15} />
											</button>
										{/snippet}
										<MenuItem
											icon="pencil"
											onclick={() => {
												menus[row.name] = false;
												startRename(row.name);
											}}
										>
											Rename
										</MenuItem>
										<MenuItem
											icon="trash-2"
											danger
											onclick={() => {
												menus[row.name] = false;
												startDelete(row.name);
											}}
										>
											Delete
										</MenuItem>
									</Menu>
								{:else}
									<span class="lb-lock" title="Added by an import. Kept for you.">
										<Icon name="lock" size={13} />
									</span>
								{/if}
							</li>
						{/each}
					</ul>

					{#if editing !== null}
						<form class="row-action flow-card" onsubmit={submitRename}>
							<Field
								label="Rename {editing}"
								id="label-rename"
								hint={`Up to ${LABEL_MAX_CHARS} characters.`}
							>
								<!-- No `maxlength`: it counts UTF-16 code units, and the
								     rule that refuses counts code points, as the server
								     does. -->
								<input
									id="label-rename"
									type="text"
									bind:value={draft}
									bind:this={nameField}
									onkeydown={(event) => {
										if (event.key === 'Escape') {
											editing = null;
										}
									}}
								/>
							</Field>
							{#if problem !== null}
								<p class="row-refusal">{problem}</p>
							{:else if note !== null}
								<p class="row-note">{note}</p>
							{/if}
							<div class="row-buttons">
								<Button tier="primary" type="submit" disabled={blocked !== undefined} reason={blocked}>
									{renaming.isPending ? 'Saving…' : 'Save'}
								</Button>
								<Button tier="quiet" onclick={() => (editing = null)}>Cancel</Button>
							</div>
						</form>
					{/if}

					{#if confirming !== null}
						{@const row = all.find((one) => one.name === confirming)}
						{#if row !== undefined}
							<!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
							<div
								class="row-action flow-card"
								role="alertdialog"
								aria-label={`Delete ${row.name}`}
								tabindex="-1"
								bind:this={confirmPanel}
								onkeydown={(event) => {
									if (event.key === 'Escape') {
										confirming = null;
									}
								}}
							>
								<p class="row-warning">{deleteWarning(row.name, row.count)}</p>
								<div class="row-buttons">
									<Button
										danger
										disabled={deleting.isPending}
										reason={deleting.isPending ? 'Deleting the label.' : undefined}
										onclick={() => deleting.mutate(row.name)}
									>
										{deleting.isPending ? 'Deleting…' : 'Delete label'}
									</Button>
									<Button tier="quiet" onclick={() => (confirming = null)}>Cancel</Button>
								</div>
							</div>
						{/if}
					{/if}
				{/if}
			{/if}
		</section>
	</div>
</div>

<FlowActionBar>
	<Button
		tier="primary"
		icon="plus"
		disabled={capped !== null || creating}
		reason={capped ?? (creating ? 'Adding the label.' : undefined)}
		onclick={startNew}
	>
		{naming && createBlocked === null ? `Create label on ${countLine(picked.size, false)}` : 'New label'}
	</Button>
</FlowActionBar>
