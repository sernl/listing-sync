<script lang="ts">
	// The seller's collections as cards: the name in the resource violet, how
	// many it holds, and the marketplaces its members are on. "New collection"
	// opens the form at the top of the list; the fine print about what a
	// collection is and why it costs sits behind the header's Explain.

	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, collectionsApi, type CollectionBody } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { agoLabel } from '$lib/elapsed';
	import { entitlementRead, limitOf } from '$lib/entitlement-read';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import FlowActionBar from '$lib/FlowActionBar.svelte';
	import Icon from '$lib/Icon.svelte';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import Menu from '$lib/Menu.svelte';
	import MenuItem from '$lib/MenuItem.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
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
	import '$lib/flow.css';
	import './collections.css';

	const queryClient = useQueryClient();

	const collections = createQuery(() => ({
		queryKey: COLLECTIONS_KEY,
		queryFn: () => collectionsApi.list().then((view) => view.collections)
	}));

	const plan = createQuery(() => entitlementRead);
	const capped = $derived(limitOf(plan.data, 'collections'));

	let search = $state('');
	let creating = $state(false);
	let name = $state('');
	let description = $state('');
	/** The server's refusal and the name it was raised against, held together
	 *  so the sentence leaves the moment the seller edits the field. */
	let refusal = $state<{ message: string; name: string } | null>(null);
	let confirming = $state<string | null>(null);
	let nameField = $state<HTMLInputElement | null>(null);
	let menus = $state<Record<string, boolean>>({});

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
			toast('info', `Created ${stored.name}.`);
		},
		onError: (failure: Error) => {
			if (failure instanceof ApiFailure && failure.status === 422) {
				refusal = { message: failure.message, name };
				return;
			}
			toast('error', 'We couldn’t create the collection.');
		}
	}));

	const remove = createMutation(() => ({
		mutationFn: (id: string) => collectionsApi.remove(id),
		onSuccess: async (_answer: void, id: string) => {
			const gone = all.find((collection) => collection.id === id)?.name ?? 'the collection';
			confirming = null;
			await queryClient.invalidateQueries({ queryKey: COLLECTIONS_KEY });
			toast('info', `Deleted ${gone}.`);
		},
		onError: () => {
			toast('error', 'We couldn’t delete the collection.');
		}
	}));

	/** What is wrong with the name in the form, whether this page decided it or
	 *  the server did. An empty field says nothing: a form the seller has not
	 *  filled yet has nothing wrong with it. */
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
			return 'Creating the collection.';
		}
		if (!verdict.accepted) {
			return verdict.message;
		}
		return problem ?? undefined;
	});

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

	function openNew() {
		creating = true;
		nameField?.focus();
	}
</script>

<div class="page flow-page has-bar collections-page">
	<PageHead
		icon="layers"
		title="Collections"
		description="Group resources in the order you choose."
		guide="labels-and-collections"
	>
		{#snippet aside()}
			<Explain title="What a collection is" label="">
				<p>{WHAT_A_COLLECTION_IS}</p>
				<p>
					Publish a collection to a marketplace, apply a template or labels to all of it, or
					export it as a spreadsheet. Each follows the collection’s order.
				</p>
				<p>{WHY_COLLECTIONS_COST}</p>
			</Explain>
			<Button
				tier="primary"
				icon="plus"
				disabled={capped !== null}
				reason={capped ?? undefined}
				onclick={openNew}
			>
				New collection
			</Button>
		{/snippet}
	</PageHead>

	<div class="flow">
		{#if capped !== null}
			<!-- The figure and the way forward, because a refusal with neither
			     reads as a control we took away. -->
			<Banner tone="warn" title="Your plan doesn’t include collections">
				{capped}
				{#snippet action()}
					<Button tier="primary" small href="/settings/subscription">See plans</Button>
				{/snippet}
			</Banner>
		{/if}

		{#if creating && capped === null}
			<section class="flow-card coll-new" aria-labelledby="coll-new-title">
				<h2 id="coll-new-title" class="coll-new-title">New collection</h2>
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
					<Field label="Note to yourself" id="new-collection-description">
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
				<div class="flow-actions">
					<Button tier="primary" disabled={blocked !== undefined} reason={blocked} onclick={run}>
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
			</section>
		{/if}

		{#if collections.isPending}
			<p class="quiet">Loading…</p>
		{:else if collections.isError}
			<Banner tone="bad" title="We couldn’t load your collections">
				Nothing has changed.
				{#snippet action()}
					<Button onclick={() => collections.refetch()}>Try again</Button>
				{/snippet}
			</Banner>
		{:else if all.length === 0}
			<Placeholder icon="layers" headline="No collections yet." body="Make one, then add resources to it.">
				{#snippet actions()}
					<Button href="/resources" icon="layout-list">Go to Resources</Button>
				{/snippet}
			</Placeholder>
		{:else}
			<section class="flow-section" aria-labelledby="coll-yours-title">
				<div class="flow-section-head">
					<h2 id="coll-yours-title">Your collections</h2>
					<span class="coll-total">
						{all.length === 1 ? '1 collection' : `${all.length} collections`}
					</span>
				</div>

				{#if all.length > 6}
					<span class="coll-search">
						<Icon name="search" size={16} />
						<input
							id="collections-search"
							type="search"
							aria-label="Search collections"
							placeholder="Search collections"
							bind:value={search}
						/>
					</span>
				{/if}

				{#if shown.length === 0}
					<p class="quiet">Nothing matches that search.</p>
				{:else}
					<ul class="coll-cards">
						{#each shown as collection (collection.id)}
							<li class="coll-card">
								<a class="coll-card-main" href={`/collections/${collection.id}`}>
									<span class="res-name coll-card-name">{collection.name}</span>
									<span class="coll-card-count">{countLine(collection.count)}</span>
									{#if collection.description !== null}
										<span class="coll-card-said">{collection.description}</span>
									{/if}
								</a>
								<div class="coll-card-foot">
									<span class="coll-marks" aria-label="On these marketplaces">
										{#each collection.inventories as inventory (inventory)}
											<MarketplaceMark {inventory} size={20} />
										{:else}
											<span class="coll-card-none">Not on a marketplace yet</span>
										{/each}
									</span>
									<span class="coll-card-when">{agoLabel(collection.updated_at, Date.now())}</span>
									<Menu label="Actions for {collection.name}" bind:open={() => menus[collection.id] ?? false, (value) => (menus[collection.id] = value)}>
										{#snippet trigger()}
											<button
												class="coll-kebab"
												type="button"
												aria-haspopup="menu"
												aria-expanded={menus[collection.id] ?? false}
												aria-label="Actions for {collection.name}"
												onclick={() => (menus[collection.id] = !menus[collection.id])}
											>
												<Icon name="ellipsis-vertical" size={16} />
											</button>
										{/snippet}
										<a class="coll-menu-link menu-item" href={`/collections/${collection.id}`}>
											<Icon name="eye" size={14} />Open
										</a>
										<MenuItem
											icon="trash-2"
											danger
											onclick={() => {
												menus[collection.id] = false;
												confirming = collection.id;
											}}
										>
											Delete
										</MenuItem>
									</Menu>
								</div>
							</li>
						{/each}
					</ul>
				{/if}

				{#if removing !== null}
					<div class="flow-card coll-confirm" role="alertdialog" tabindex="-1" aria-label={`Delete ${removing.name}`}>
						<p>
							Delete <span class="res-name">{removing.name}</span>? The {countLine(removing.count)}
							in it stay in Resources and on their marketplaces.
						</p>
						<div class="flow-actions">
							<Button
								danger
								disabled={remove.isPending}
								reason={remove.isPending ? 'Deleting the collection.' : undefined}
								onclick={() => remove.mutate(removing.id)}
							>
								{remove.isPending ? 'Deleting…' : 'Delete it'}
							</Button>
							<Button tier="quiet" onclick={() => (confirming = null)}>Keep it</Button>
						</div>
					</div>
				{/if}
			</section>
		{/if}
	</div>
</div>

<FlowActionBar>
	<Button
		tier="primary"
		icon="plus"
		disabled={capped !== null}
		reason={capped ?? undefined}
		onclick={openNew}
	>
		New collection
	</Button>
</FlowActionBar>
