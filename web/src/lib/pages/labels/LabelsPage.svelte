<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api, type LabelView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import { entitlementRead, limitOf } from '$lib/entitlement-read';
	import Icon from '$lib/Icon.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import RowCard from '$lib/RowCard.svelte';
	import { queryKeys } from '$lib/query';
	import { remember, remembered } from '$lib/dismissal';
	import { toast } from '$lib/toast';
	import { LABEL_COUNTS_KEY, countAll, deleteLabel, renameLabel } from './api';
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
	// vocabulary lands, so a catalogue that is slow costs each row its figure
	// rather than costing the page its list.
	//
	// The vocabulary is part of the key rather than only of the answer. A rename
	// invalidates both reads at once, and a counts refetch that ran before the
	// renamed list arrived would count the names that are gone and then hold
	// that answer under a key nothing else invalidates, leaving the renamed row
	// with no figure for good.
	//
	// `staleTime` is what stops a refocus repeating the whole pass. There is no
	// count on the labels route, so each figure costs a walk of the catalogue
	// narrowed to that label; five minutes is long enough that alt-tabbing is
	// free and short enough that a relabel elsewhere shows up on the next visit.
	const COUNTS_FRESH_MS = 5 * 60 * 1000;
	const counts = createQuery(() => ({
		// Joined on a newline, which no label can contain: the server refuses a
		// control character in a name, so two different vocabularies cannot
		// spell the same key.
		queryKey: [...LABEL_COUNTS_KEY, names.join('\n')],
		enabled: labels.isSuccess,
		staleTime: COUNTS_FRESH_MS,
		queryFn: () => countAll(names)
	}));

	let search = $state('');
	/** Read at setup rather than in an effect: this app does not render on the
	 *  server, so storage is readable here, and an effect would draw the banner
	 *  and then take it away in front of a seller who closed it last week. */
	let bannerShown = $state(!remembered('labels.what-are-labels'));
	/** Closed for the visit alone. Every label is still listed under this
	 *  sentence -- it is the counts beside them that are missing -- so closing
	 *  it leaves a page that still explains itself, and the counts may well be
	 *  readable on the next navigation. */
	let uncountedShown = $state(true);
	let newAsked = $state(false);
	let editing = $state<string | null>(null);
	let draft = $state('');
	/** The server's refusal, and the draft it was raised against. Held together
	 *  so the sentence leaves the moment the seller edits the name: a refusal
	 *  about a name no longer in the field is worse than none, because Save is
	 *  live underneath it. */
	let refusal = $state<{ message: string; draft: string } | null>(null);
	let confirming = $state<string | null>(null);
	let nameField = $state<HTMLInputElement | null>(null);
	let confirmPanel = $state<HTMLElement | null>(null);

	$effect(() => {
		nameField?.focus();
	});

	// A confirmation that opens below the fold is a confirmation the seller never
	// saw: the kebab is at the row, and the panel that answers it may be
	// anywhere. Focus moves into it so a keyboard and a screen reader both land
	// on the sentence, and the scroll brings it to somewhere the phone's fixed
	// tab bar is not, which `scroll-margin-block` in the sheet reserves.
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
	 *  catalogue, whose rows carry no label and so cannot have changed. Matching
	 *  on the prefix alone would empty that one on every other screen too. */
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
			toast('error', 'The label was not renamed.');
		}
	}));

	const deleting = createMutation(() => ({
		mutationFn: (name: string) => deleteLabel(name),
		onSuccess: async (_answer: void, name: string) => {
			confirming = null;
			await refresh();
			toast('info', `${name} is no longer one of your labels.`);
		},
		onError: () => {
			toast('error', 'The label was not deleted.');
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

	/** What is wrong with the name in the field, whether this page decided it or
	 *  the server did. Shown rather than left in the disabled control's tooltip:
	 *  a greyed Save with a hidden reason is a page that will not say what it
	 *  wants. The server's sentence stands only while its own draft does. */
	const problem = $derived.by(() => {
		if (refusal !== null && refusal.draft === draft) {
			return refusal.message;
		}
		return verdict.accepted ? null : verdict.message;
	});

	/** Why Save is greyed when nothing is wrong with the name: it is the name
	 *  the label already has. Muted rather than red, because nothing has failed
	 *  and there is nothing to correct. */
	const note = $derived(problem === null && stale ? 'That is the name it already has.' : null);

	/** Why Save cannot run, which the tier requires of any disabled control. */
	const blocked = $derived.by(() => {
		if (renaming.isPending) {
			return 'The rename is being saved.';
		}
		if (note !== null) {
			return note;
		}
		return problem ?? undefined;
	});
</script>

<div class="page labels-page">
	<PageHead
		icon="tag"
		title="Labels"
		description="Your own words for grouping resources, up to twenty on each one."
	>
		{#snippet aside()}
			<Button
				tier="additive"
				icon="plus"
				disabled={capped !== null}
				reason={capped ?? undefined}
				onclick={() => (newAsked = !newAsked)}
			>
				New label
			</Button>
		{/snippet}
	</PageHead>

	{#if bannerShown}
		<Banner
			title="What are labels?"
			onDismiss={() => {
				bannerShown = false;
				remember('labels.what-are-labels');
			}}
		>
			Use labels to group and filter your resources. Each label's colour comes from its
			name.
		</Banner>
	{/if}

	<!-- There is no empty label to create: a label exists exactly while a
	     resource carries it, which is what `GET /v1/labels` answers and what
	     deleting the last carrier undoes. So the additive action explains the
	     one way to make one rather than opening a form the API cannot accept.
	     The region announces itself, because the control that opens it is a
	     shared button with no room for `aria-expanded`. -->
	<div role="status" aria-live="polite">
		{#if newAsked}
			<Banner title="A label starts on a resource">
				You make a label by putting it on a resource: open one, or pick several on the
				Resources list, and add it there.
				{#snippet action()}
					<Button href="/resources" icon="layout-list">Go to Resources</Button>
				{/snippet}
			</Banner>
		{/if}
	</div>

	{#if labels.isPending}
		<p class="quiet">Loading…</p>
	{:else if labels.isError}
		<Banner tone="bad" title="We could not read your labels">
			Nothing has changed. We could not load the list just now.
			{#snippet action()}
				<Button onclick={() => labels.refetch()}>Try again</Button>
			{/snippet}
		</Banner>
	{:else if all.length === 0}
		<Placeholder
			icon="tag"
			headline="Create your first label to get started."
			body="A label is your own word for a group of resources; add one on a resource and it appears here."
		>
			{#snippet actions()}
				<Button tier="additive" href="/resources" icon="layout-list">Go to Resources</Button>
			{/snippet}
		</Placeholder>
	{:else}
		<div class="labels-toolbar">
			<Field label="Search" id="labels-search">
				<span class="labels-search">
					<Icon name="search" size={16} />
					<input
						id="labels-search"
						type="search"
						placeholder="Search labels"
						bind:value={search}
					/>
				</span>
			</Field>
			<span class="labels-total">{totalLine(all.length)}</span>
		</div>

		{#if uncounted && uncountedShown}
			<Banner
				tone="warn"
				title="Some counts are missing"
				onDismiss={() => (uncountedShown = false)}
			>
				Every label is listed below. We could not read all of the counts.
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
			<ul class="labels-list">
				{#each shown as row (row.name)}
					<li
						class="label-row {swatchClass(row.colour)}"
						class:open={editing === row.name || confirming === row.name}
					>
						<RowCard
							href={filterHref(row.name)}
							title={row.name}
							meta={countLine(row.count, counting)}
						>
							{#snippet menu(close)}
								<!-- A system label is an import's own mark: the seller neither
								     made it nor can rename or delete it, and both routes answer
								     404 for one — the same answer a name nobody holds gets.
								     Hidden rather than disabled, because there is no reason to
								     offer a control whose refusal would say the label is not
								     there. -->
								{#if !row.system}
									<button
										type="button"
										role="menuitem"
										onclick={() => {
											close();
											startRename(row.name);
										}}>Rename</button
									>
									<button
										type="button"
										role="menuitem"
										onclick={() => {
											close();
											startDelete(row.name);
										}}>Delete</button
									>
								{/if}
							{/snippet}
						</RowCard>

						{#if editing === row.name}
							<form class="row-action" onsubmit={submitRename}>
								<Field
									label="New name"
									id="label-rename"
									hint={`At most ${LABEL_MAX_CHARS} characters; every resource that has it keeps it.`}
								>
									<!-- No `maxlength`: it counts UTF-16 code units, so a name
									     written in astral characters would be cut at thirty while
									     the hint beside it promised sixty. The rule that refuses
									     counts code points, as the server does. -->
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
									<Button
										tier="additive"
										type="submit"
										disabled={blocked !== undefined}
										reason={blocked}
									>
										{renaming.isPending ? 'Saving…' : 'Save'}
									</Button>
									<Button onclick={() => (editing = null)}>Cancel</Button>
								</div>
							</form>
						{/if}

						{#if confirming === row.name}
							<!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
							<div
								class="row-action"
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
										reason={deleting.isPending
											? 'The label is being deleted.'
											: undefined}
										onclick={() => deleting.mutate(row.name)}
									>
										{deleting.isPending ? 'Deleting…' : 'Delete label'}
									</Button>
									<Button tier="quiet" onclick={() => (confirming = null)}>Cancel</Button>
								</div>
							</div>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	{/if}
</div>
