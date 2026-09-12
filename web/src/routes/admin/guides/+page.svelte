<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import {
		ApiFailure,
		api,
		type GuideHeadView,
		type GuideTaxon,
		type GuideTaxonKind
	} from '$lib/api';
	import Button from '$lib/Button.svelte';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import Field from '$lib/Field.svelte';
	import Menu from '$lib/Menu.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import Toggle from '$lib/Toggle.svelte';
	import { toast } from '$lib/toast';
	import { slugRefusal, slugify, titleRefusal } from '$lib/pages/guides/editor';
	import { taxonLabel } from '$lib/pages/guides/filters';
	import '$lib/pages/admin/admin.css';
	import '$lib/pages/guides/guides.css';

	const queryClient = useQueryClient();
	const now = Date.now();

	const guides = createQuery(() => ({
		queryKey: queryKeys.adminGuides,
		queryFn: () => api.adminGuides()
	}));

	/** Every topic and tag, retired ones included: this is the screen that
	 *  retires them, so a list that hid them would have no way back. */
	const taxonomy = createQuery(() => ({
		queryKey: queryKeys.adminGuideTaxonomy,
		queryFn: () => api.adminGuideTaxonomy()
	}));

	const rows = $derived(guides.data?.guides ?? []);
	const topics = $derived(taxonomy.data?.topics ?? []);
	const tags = $derived(taxonomy.data?.tags ?? []);
	/** The two kinds, in the order the panel reads them. One shape for both,
	 *  because a topic and a tag differ only in how many a guide may hold. */
	const groups = $derived([
		{ kind: 'topics' as GuideTaxonKind, label: 'Topics', taxa: topics },
		{ kind: 'tags' as GuideTaxonKind, label: 'Tags', taxa: tags }
	]);
	/** What a new guide may be filed under. A retired taxon is not offered to
	 *  something being written now. */
	const liveTopics = $derived(topics.filter((topic) => !topic.retired));
	const liveTags = $derived(tags.filter((tag) => !tag.retired));

	let title = $state('');
	let slug = $state('');
	/** Whether the operator has typed a slug of their own. Until they do, the
	 *  slug follows the title; after, it stops, because a slug that kept
	 *  following would overwrite what they just wrote. */
	let slugTyped = $state(false);
	const suggested = $derived(slugTyped ? slug : slugify(title));
	let topicId = $state<string | null>(null);
	let tagIds = $state<string[]>([]);
	let tagMenu = $state(false);

	const createRefusal = $derived(titleRefusal(title) ?? slugRefusal(suggested));

	/** Whether this page is still on screen.
	 *
	 * A mutation's callbacks are not bound to the component that started it: a
	 * create that answers after the operator has navigated away would go on to
	 * clear fields nobody is looking at and, worse, send the browser to the new
	 * guide's editor from whatever screen they had moved to. The cache is left
	 * to be invalidated either way — a stale list is a fact about the server,
	 * and refreshing it cannot land on the wrong screen. */
	let onScreen = true;
	$effect(() => {
		return () => {
			onScreen = false;
		};
	});

	const creating = createMutation(() => ({
		mutationFn: () =>
			// A new guide starts as an empty draft: the body is written in the
			// editor, which is the screen with the toolbar and the preview on
			// it, so creating here would be a second place to write one. It is
			// never created published — publication is its own request, made
			// once somebody has read the guide back.
			api.createGuide({
				slug: suggested,
				title: title.trim(),
				body: '',
				topic_id: topicId,
				tag_ids: tagIds
			}),
		onSuccess: async (guide) => {
			await queryClient.invalidateQueries({ queryKey: queryKeys.adminGuides });
			if (!onScreen) {
				return;
			}
			title = '';
			slug = '';
			slugTyped = false;
			topicId = null;
			tagIds = [];
			await goto(`/admin/guides/${guide.slug}`);
		},
		onError: (failure: Error) =>
			toast(
				'error',
				failure instanceof ApiFailure ? failure.message : 'The guide was not created.'
			)
	}));

	const deleting = createMutation(() => ({
		// The row carries the guide's own id, so a delete names the guide the
		// operator was shown rather than whatever answers to its address by the
		// time the request arrives. A slug and a revision would not: a guide
		// deleted and recreated there starts again at revision one.
		mutationFn: (guide: GuideHeadView) =>
			api.deleteGuide(guide.slug, {
				expected_id: guide.id,
				expected_revision: guide.revision
			}),
		onSuccess: async () => {
			await queryClient.invalidateQueries({ queryKey: queryKeys.adminGuides });
			// A deleted guide was possibly published, so the reader's lists and
			// the page itself are stale in every narrowing.
			await queryClient.invalidateQueries({ queryKey: queryKeys.guides });
			await queryClient.invalidateQueries({ queryKey: queryKeys.guideTaxonomy });
			toast('info', 'Guide deleted.');
		},
		onError: (failure: Error) =>
			toast(
				'error',
				failure instanceof ApiFailure
					? failure.status === 409
						? 'Somebody changed this guide while the list was open. Reload before deleting it.'
						: failure.message
					: 'The guide was not deleted.'
			)
	}));

	function remove(guide: GuideHeadView) {
		const sure = confirm(
			`Delete “${guide.title}”?\n\n` +
				(guide.status === 'published'
					? 'It is published, so every link to it in this console stops resolving. '
					: '') +
				'The body is gone with it, and any picture it used stays stored.'
		);
		if (sure) {
			deleting.mutate(guide);
		}
	}

	// --- topics and tags ---------------------------------------------------

	/** The name each taxon is being renamed to, by id. Absent means untouched,
	 *  which is what keeps a list refetch from overwriting a rename in
	 *  progress. */
	let renaming = $state<Record<string, string>>({});
	let newName = $state<Record<GuideTaxonKind, string>>({ topics: '', tags: '' });

	const adding = createMutation(() => ({
		mutationFn: (variables: { kind: GuideTaxonKind; name: string }) =>
			api.createGuideTaxon(variables.kind, {
				slug: slugify(variables.name),
				name: variables.name.trim()
			}),
		onSuccess: async (taxon, variables) => {
			newName[variables.kind] = '';
			await queryClient.invalidateQueries({ queryKey: queryKeys.adminGuideTaxonomy });
			toast('info', `${taxon.name} added.`);
		},
		onError: (failure: Error) =>
			toast('error', failure instanceof ApiFailure ? failure.message : 'It was not added.')
	}));

	const changing = createMutation(() => ({
		mutationFn: (variables: {
			kind: GuideTaxonKind;
			taxon: GuideTaxon;
			name: string;
			retired: boolean;
		}) =>
			api.updateGuideTaxon(variables.kind, variables.taxon.id, {
				name: variables.name.trim(),
				retired: variables.retired
			}),
		onSuccess: async (taxon) => {
			// The rename is the server's answer now, so the local draft goes:
			// keeping it would show a name nobody stored if the two differ.
			delete renaming[taxon.id];
			await queryClient.invalidateQueries({ queryKey: queryKeys.adminGuideTaxonomy });
			// A rename or a retirement changes what a reader's picker offers and
			// what every published guide carrying it is labelled.
			await queryClient.invalidateQueries({ queryKey: queryKeys.guides });
			await queryClient.invalidateQueries({ queryKey: queryKeys.guideTaxonomy });
		},
		onError: (failure: Error) =>
			toast('error', failure instanceof ApiFailure ? failure.message : 'It was not changed.')
	}));

	/** Why this name cannot be added, or null where it can. Both bounds at
	 *  once: the name is stored as typed and the address is derived from it, so
	 *  a name with no slug in it is refused here rather than at the table. */
	function addRefusal(name: string): string | null {
		return titleRefusal(name) ?? slugRefusal(slugify(name));
	}
</script>

<div class="page">
	<PageHead
		icon="book-open"
		title="Guides"
		description="The help pages every seller reads. Written here, rendered by the server, and published one at a time."
	/>

	<Panel title="Write a new guide" description="A title and an address. The body comes next.">
		<!-- One row of controls, aligned on the controls themselves rather than
		     on the bottom of each field: the address field carries a hint and
		     the others do not, and a flex row ending at `flex-end` put the
		     title's input a hint's height above the address's. The hint lives
		     under the whole row for the same reason. -->
		<div class="gd-head">
			<Field label="Title" id="guide-title" required>
				<input
					id="guide-title"
					type="text"
					bind:value={title}
					placeholder="Connecting a marketplace"
				/>
			</Field>
			<Field label="Address" id="guide-slug">
				<input
					id="guide-slug"
					type="text"
					value={suggested}
					oninput={(event) => {
						slugTyped = true;
						slug = event.currentTarget.value;
					}}
				/>
			</Field>
			<Field label="Topic" id="guide-new-topic">
				<select
					id="guide-new-topic"
					value={topicId ?? ''}
					disabled={taxonomy.isError}
					onchange={(event) =>
						(topicId = event.currentTarget.value.length === 0 ? null : event.currentTarget.value)}
				>
					<option value="">No topic</option>
					{#each liveTopics as topic (topic.id)}
						<option value={topic.id}>{topic.name}</option>
					{/each}
				</select>
			</Field>
			<div class="gd-head-tags">
				<span class="gd-group-label" id="guide-new-tags">Tags</span>
				<Menu bind:open={tagMenu} label="Tags for the new guide" align="start">
					{#snippet trigger()}
						<Button onclick={() => (tagMenu = !tagMenu)}>
							{tagIds.length === 0 ? 'No tags' : `${tagIds.length} chosen`}
						</Button>
					{/snippet}
					<div class="gd-tag-menu" role="group" aria-labelledby="guide-new-tags">
						{#each liveTags as tag (tag.id)}
							<label>
								<input
									type="checkbox"
									checked={tagIds.includes(tag.id)}
									onchange={(event) =>
										(tagIds = event.currentTarget.checked
											? [...tagIds, tag.id]
											: tagIds.filter((id) => id !== tag.id))}
								/>
								<span class="gd-tax">{tag.name}</span>
							</label>
						{:else}
							<p class="none">No tag has been made yet.</p>
						{/each}
					</div>
				</Menu>
			</div>
			<div class="gd-head-act">
				<Button
					tier="primary"
					icon="plus"
					disabled={createRefusal !== null || creating.isPending}
					reason={createRefusal ?? (creating.isPending ? 'The guide is being created.' : undefined)}
					onclick={() => creating.mutate()}
				>
					{creating.isPending ? 'Creating…' : 'Create draft'}
				</Button>
			</div>
		</div>
		<p class="gd-head-hint">
			Sellers reach it at /guides/{suggested.length === 0 ? '…' : suggested}. The address is fixed
			once the guide exists, because every link to it is that address.
		</p>
	</Panel>

	<Panel
		title="Topics and tags"
		description="One topic per guide and any number of tags. Retiring one keeps it on the guides that carry it and takes it out of the pickers."
	>
		{#if taxonomy.isPending}
			<p class="quiet">Reading the topics and tags…</p>
		{:else if taxonomy.isError}
			<p class="quiet">
				The topics and tags could not be read, so nothing here can be chosen or changed.
			</p>
		{:else}
			<div class="gd-tax-cols">
				{#each groups as group (group.kind)}
					<div class="gd-tax-col">
						<h3>{group.label}</h3>
						{#each group.taxa as taxon (taxon.id)}
							{@const draft = renaming[taxon.id] ?? taxon.name}
							<div class="gd-tax-row" class:retired={taxon.retired}>
								<label class="sr-only" for={`taxon-${taxon.id}`}>
									Name of {taxon.name}
								</label>
								<input
									id={`taxon-${taxon.id}`}
									type="text"
									value={draft}
									oninput={(event) => (renaming[taxon.id] = event.currentTarget.value)}
								/>
								<Toggle
									label="Retired"
									checked={taxon.retired}
									disabled={changing.isPending}
									onchange={(retired) =>
										changing.mutate({
											kind: group.kind,
											taxon,
											// The stored name, not the half-typed rename beside it:
											// retiring is one decision and renaming is another.
											name: taxon.name,
											retired
										})}
								/>
								<Button
									small
									disabled={draft.trim() === taxon.name ||
										titleRefusal(draft) !== null ||
										changing.isPending}
									reason={titleRefusal(draft) ??
										(draft.trim() === taxon.name ? 'The name is unchanged.' : undefined)}
									onclick={() =>
										changing.mutate({
											kind: group.kind,
											taxon,
											name: draft,
											retired: taxon.retired
										})}
								>
									Rename
								</Button>
								<span class="gd-tax-slug">/{taxon.slug}</span>
							</div>
						{:else}
							<p class="quiet">None yet.</p>
						{/each}
						<div class="gd-tax-add">
							<label class="sr-only" for={`taxon-new-${group.kind}`}>
								New {group.label.toLowerCase()} name
							</label>
							<input
								id={`taxon-new-${group.kind}`}
								type="text"
								placeholder={group.kind === 'topics' ? 'Getting started' : 'pricing'}
								bind:value={newName[group.kind]}
							/>
							<Button
								tier="additive"
								small
								icon="plus"
								disabled={addRefusal(newName[group.kind]) !== null || adding.isPending}
								reason={addRefusal(newName[group.kind]) ??
									(adding.isPending ? 'One is being added.' : undefined)}
								onclick={() => adding.mutate({ kind: group.kind, name: newName[group.kind] })}
							>
								Add
							</Button>
						</div>
					</div>
				{/each}
			</div>
		{/if}
	</Panel>

	<Panel>
		{#if guides.isPending}
			<p class="quiet">Reading the guides…</p>
		{:else if guides.isError}
			<Placeholder
				icon="book-open"
				headline="The guides could not be read"
				body="The request did not come back with an answer we can act on, so this page cannot
					say whether any guide exists. Reloading is the only thing worth trying from here."
			/>
		{:else if rows.length === 0}
			<Placeholder
				icon="book-open"
				headline="No guide has been written yet"
				body="Sellers see an empty Help and guides page until the first one is published. A
					draft is invisible to them, so there is no harm in starting one."
			/>
		{:else}
			<div class="op-table">
				<table>
					<thead>
						<tr>
							<th>Guide</th>
							<th>Filed under</th>
							<th>State</th>
							<th class="num">Updated</th>
							<th class="num">Actions</th>
						</tr>
					</thead>
					<tbody>
						{#each rows as guide (guide.slug)}
							<tr>
								<td class="op-cell" data-label="Guide">
									<a class="t" href={`/admin/guides/${guide.slug}`} title={guide.title}>
										{guide.title}
									</a>
									<span class="s">/guides/{guide.slug}</span>
								</td>
								<td data-label="Filed under">
									<span class="gd-row-tax">
										{#if guide.topic !== null}
											<span class="gd-tax gd-tax-topic">{taxonLabel(guide.topic)}</span>
										{/if}
										{#each guide.tags as tag (tag.id)}
											<span class="gd-tax">{taxonLabel(tag)}</span>
										{/each}
										{#if guide.topic === null && guide.tags.length === 0}
											<span class="quiet">—</span>
										{/if}
									</span>
								</td>
								<td data-label="State">
									{#if guide.status === 'published'}
										<StatusPill tone="ok" label="published" />
									{:else}
										<StatusPill tone="soon" label="draft" />
									{/if}
								</td>
								<td class="num" data-label="Updated" title={utcInstant(guide.updated_at)}>
									{agoLabel(guide.updated_at, now)}
								</td>
								<td data-label="Actions">
									<div class="op-acts">
										<Button tier="outline" small href={`/admin/guides/${guide.slug}`}>Edit</Button>
										{#if guide.status === 'published'}
											<Button tier="outline" small href={`/guides/${guide.slug}`}>View</Button>
										{/if}
										<Button
											tier="outline"
											small
											danger
											disabled={deleting.isPending}
											reason={deleting.isPending ? 'A delete is in flight.' : undefined}
											onclick={() => remove(guide)}
										>
											Delete
										</Button>
									</div>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
			<p class="foot-note">
				A draft is a 404 on the reader's side rather than an unpublished page: nobody but an
				operator can read one, and no link to it resolves until it is published.
			</p>
		{/if}
	</Panel>
</div>
