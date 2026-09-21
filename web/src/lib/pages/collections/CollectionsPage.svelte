<script lang="ts">
	// Every collection the seller has, and the one form that makes another.
	//
	// The New collection control is gated on `collections_max` rather than on
	// the create failing: the free plan includes none, so a seller on it would
	// otherwise fill a name and a description before being refused. The reason
	// is stated on the control and again beside it, because a disabled control
	// with no stated reason reads as a fault.

	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, collectionsApi, type CollectionBody } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { agoLabel } from '$lib/elapsed';
	import { entitlementRead, limitOf } from '$lib/entitlement-read';
	import Field from '$lib/Field.svelte';
	import Icon from '$lib/Icon.svelte';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import MenuItem from '$lib/MenuItem.svelte';
	import Note from '$lib/Note.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import RowCard from '$lib/RowCard.svelte';
	import { toast } from '$lib/toast';
	import {
		COLLECTIONS_KEY,
		DESCRIPTION_MAX_CHARS,
		NAME_MAX_CHARS,
		WHAT_A_COLLECTION_IS,
		WHY_COLLECTIONS_COST,
		checkName,
		countLine,
		matching
	} from './collections';
	import './collections.css';

	const queryClient = useQueryClient();

	const collections = createQuery(() => ({
		queryKey: COLLECTIONS_KEY,
		queryFn: () => collectionsApi.list().then((view) => view.collections)
	}));

	// The cap, from the plan the shell already read. A pending or failed read
	// refuses nothing: `limitOf` answers null for both, so an outage never
	// disables a paying seller's own control.
	const plan = createQuery(() => entitlementRead);
	const capped = $derived(limitOf(plan.data, 'collections'));

	let search = $state('');
	let creating = $state(false);
	let name = $state('');
	let description = $state('');
	/** The server's refusal and the name it was raised against, held together
	 *  so the sentence leaves the moment the seller edits the field: a refusal
	 *  about a name no longer in the form is worse than none, because Create is
	 *  live underneath it. */
	let refusal = $state<{ message: string; name: string } | null>(null);
	let confirming = $state<string | null>(null);
	let nameField = $state<HTMLInputElement | null>(null);

	$effect(() => {
		if (creating) {
			nameField?.focus();
		}
	});

	const all = $derived(collections.data ?? []);
	const shown = $derived(matching(all, search));
	const taken = $derived(all.map((collection) => collection.name));
	const verdict = $derived(checkName(name, taken));
	const removing = $derived(all.find((collection) => collection.id === confirming) ?? null);

	const create = createMutation(() => ({
		mutationFn: (body: CollectionBody) => collectionsApi.create(body),
		onSuccess: async (stored) => {
			creating = false;
			name = '';
			description = '';
			refusal = null;
			await queryClient.invalidateQueries({ queryKey: COLLECTIONS_KEY });
			toast('info', `${stored.name} is one of your collections.`);
		},
		onError: (failure: Error) => {
			// A 422 is about the form, so it is answered beside it. Anything
			// else is about the request and goes to the toasts.
			if (failure instanceof ApiFailure && failure.status === 422) {
				refusal = { message: failure.message, name };
				return;
			}
			toast('error', 'The collection was not created.');
		}
	}));

	const remove = createMutation(() => ({
		mutationFn: (id: string) => collectionsApi.remove(id),
		onSuccess: async (_answer: void, id: string) => {
			const gone = all.find((collection) => collection.id === id)?.name ?? 'The collection';
			confirming = null;
			await queryClient.invalidateQueries({ queryKey: COLLECTIONS_KEY });
			toast('info', `${gone} is no longer one of your collections.`);
		},
		onError: () => {
			toast('error', 'The collection was not deleted.');
		}
	}));

	/** What is wrong with the name in the form, whether this page decided it or
	 *  the server did. Shown rather than left in the disabled control's
	 *  tooltip: a greyed Create with a hidden reason is a page that will not
	 *  say what it wants. An empty field says nothing at all -- a form the
	 *  seller has not filled yet has nothing wrong with it. */
	const problem = $derived.by(() => {
		if (refusal !== null && refusal.name === name) {
			return refusal.message;
		}
		return name.trim().length === 0 || verdict.accepted ? null : verdict.message;
	});

	/** Why Create cannot run, which the button tier requires of any disabled
	 *  control. */
	const blocked = $derived.by(() => {
		if (create.isPending) {
			return 'The collection is being created.';
		}
		if (!verdict.accepted) {
			return verdict.message;
		}
		return problem ?? undefined;
	});

	/** The create, from either door: the form's own submit, and the button in
	 *  the footer beside Cancel, which is outside the form. */
	function run() {
		if (!verdict.accepted || create.isPending) {
			return;
		}
		create.mutate({
			name: verdict.name,
			description: description.trim().length === 0 ? null : description.trim()
		});
	}

	function submit(event: SubmitEvent) {
		event.preventDefault();
		run();
	}
</script>

<div class="page collections-page">
	<PageHead
		icon="layers"
		title="Collections"
		description="Ordered sets of your resources."
		guide="labels-and-collections"
	>
		{#snippet aside()}
			<Button
				tier="additive"
				icon="plus"
				disabled={capped !== null}
				reason={capped ?? undefined}
				onclick={() => (creating = !creating)}
			>
				New collection
			</Button>
		{/snippet}
	</PageHead>

	{#if capped !== null}
		<!-- The figure and the way forward, because a refusal with neither reads
		     as a control we took away. The free plan includes none, so this is
		     the state most sellers meet this page in. -->
		<Banner tone="warn" title="Collections are not on your plan">
			{capped}
			{WHY_COLLECTIONS_COST}
			{#snippet action()}
				<Button tier="primary" small href="/settings/subscription">See plans</Button>
			{/snippet}
		</Banner>
	{/if}

	{#if creating && capped === null}
		<Panel title="A new collection">
			<form class="coll-form" onsubmit={submit}>
				<Field label="Name" id="new-collection-name" required>
					<input
						id="new-collection-name"
						type="text"
						maxlength={NAME_MAX_CHARS}
						placeholder="Autumn term"
						bind:this={nameField}
						bind:value={name}
						disabled={create.isPending}
					/>
				</Field>
				<Field
					label="Description"
					id="new-collection-description"
					hint="Yours alone; it reaches no marketplace."
				>
					<textarea
						id="new-collection-description"
						rows="2"
						maxlength={DESCRIPTION_MAX_CHARS}
						bind:value={description}
						disabled={create.isPending}
					></textarea>
				</Field>
			</form>
			{#if problem !== null}
				<p class="refusal">{problem}</p>
			{/if}
			<div class="coll-form-foot">
				<Button tier="additive" disabled={blocked !== undefined} reason={blocked} onclick={run}>
					{create.isPending ? 'Creating…' : 'Create'}
				</Button>
				<Button
					tier="quiet"
					onclick={() => {
						creating = false;
						refusal = null;
					}}
				>
					Cancel
				</Button>
			</div>
		</Panel>
	{/if}

	{#if collections.isPending}
		<p class="quiet">Loading…</p>
	{:else if collections.isError}
		<Banner tone="bad" title="We could not read your collections">
			Nothing has changed.
			{#snippet action()}
				<Button onclick={() => collections.refetch()}>Try again</Button>
			{/snippet}
		</Banner>
	{:else if all.length === 0}
		<Placeholder icon="layers" headline="No collections yet." body={WHAT_A_COLLECTION_IS}>
			{#snippet actions()}
				<Button
					tier="additive"
					icon="plus"
					disabled={capped !== null}
					reason={capped ?? undefined}
					onclick={() => (creating = true)}
				>
					New collection
				</Button>
				<Button href="/resources" icon="layout-list">Go to Resources</Button>
			{/snippet}
		</Placeholder>
	{:else}
		<div class="coll-toolbar">
			<Field label="Search" id="collections-search">
				<span class="coll-search">
					<Icon name="search" size={16} />
					<input
						id="collections-search"
						type="search"
						placeholder="Search collections"
						bind:value={search}
					/>
				</span>
			</Field>
			<span class="coll-total">
				{all.length === 1 ? '1 collection' : `${all.length} collections`}
			</span>
		</div>

		{#if shown.length === 0}
			<p class="quiet">Nothing matches that search.</p>
		{:else}
			{#each shown as collection (collection.id)}
				<RowCard
					href={`/collections/${collection.id}`}
					title={collection.name}
					meta={collection.description ??
						`updated ${agoLabel(collection.updated_at, Date.now())}`}
				>
					{#snippet strip()}
						<span class="coll-marks">
							<span class="coll-total">{countLine(collection.count)}</span>
							{#each collection.inventories as inventory (inventory)}
								<MarketplaceMark {inventory} size={16} />
							{/each}
						</span>
					{/snippet}
					{#snippet menu(close: () => void)}
						<a class="coll-menu-link menu-item" href={`/collections/${collection.id}`}>
							<Icon name="eye" size={14} />Open
						</a>
						<MenuItem
							icon="trash-2"
							danger
							onclick={() => {
								close();
								confirming = collection.id;
							}}
						>
							Delete
						</MenuItem>
					{/snippet}
				</RowCard>
			{/each}
		{/if}

		{#if removing !== null}
			<Panel title={`Delete ${removing.name}?`}>
				<p>
					The {countLine(removing.count)} in it stay in your Resources, on every marketplace
					they are already on.
				</p>
				<div class="coll-form-foot">
					<Button
						danger
						disabled={remove.isPending}
						reason={remove.isPending ? 'The collection is being deleted.' : undefined}
						onclick={() => remove.mutate(removing.id)}
					>
						{remove.isPending ? 'Deleting…' : 'Delete it'}
					</Button>
					<Button tier="quiet" onclick={() => (confirming = null)}>Keep it</Button>
				</div>
			</Panel>
		{/if}

		<Note>{WHAT_A_COLLECTION_IS}</Note>
	{/if}
</div>
