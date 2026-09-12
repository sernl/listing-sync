<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { ApiFailure, api, type GuideHeadView } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import Field from '$lib/Field.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import { slugRefusal, slugify, titleRefusal } from '$lib/pages/guides/editor';
	import '$lib/pages/admin/admin.css';
	import '$lib/pages/guides/guides.css';

	const queryClient = useQueryClient();
	const now = Date.now();

	const guides = createQuery(() => ({
		queryKey: queryKeys.adminGuides,
		queryFn: () => api.adminGuides()
	}));

	const rows = $derived(guides.data?.guides ?? []);

	let title = $state('');
	let slug = $state('');
	/** Whether the operator has typed a slug of their own. Until they do, the
	 *  slug follows the title; after, it stops, because a slug that kept
	 *  following would overwrite what they just wrote. */
	let slugTyped = $state(false);
	const suggested = $derived(slugTyped ? slug : slugify(title));

	const createRefusal = $derived(titleRefusal(title) ?? slugRefusal(suggested));

	const creating = createMutation(() => ({
		mutationFn: () =>
			// A new guide starts as an empty draft: the body is written in the
			// editor, which is the screen with the toolbar and the preview on
			// it, so creating here would be a second place to write one.
			api.createGuide({ slug: suggested, title: title.trim(), body: '', status: 'draft' }),
		onSuccess: async (guide) => {
			await queryClient.invalidateQueries({ queryKey: queryKeys.adminGuides });
			title = '';
			slug = '';
			slugTyped = false;
			await goto(`/admin/guides/${guide.slug}`);
		},
		onError: (failure: Error) =>
			toast(
				'error',
				failure instanceof ApiFailure ? failure.message : 'The guide was not created.'
			)
	}));

	const deleting = createMutation(() => ({
		mutationFn: (guide: string) => api.deleteGuide(guide),
		onSuccess: async () => {
			await queryClient.invalidateQueries({ queryKey: queryKeys.adminGuides });
			toast('info', 'Guide deleted.');
		},
		onError: (failure: Error) =>
			toast(
				'error',
				failure instanceof ApiFailure ? failure.message : 'The guide was not deleted.'
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
			deleting.mutate(guide.slug);
		}
	}
</script>

<div class="page">
	<PageHead
		icon="book-open"
		title="Guides"
		description="The help pages every seller reads. Written here, rendered by the server, and published one at a time."
	/>

	<Panel title="Write a new guide" description="A title and an address. The body comes next.">
		<div class="gd-head">
			<div class="grow">
				<Field label="Title" id="guide-title" required>
					<input
						id="guide-title"
						type="text"
						bind:value={title}
						placeholder="Connecting a marketplace"
					/>
				</Field>
			</div>
			<div class="grow">
				<Field
					label="Address"
					id="guide-slug"
					hint="Where sellers reach it: /guides/{suggested.length === 0
						? '…'
						: suggested}"
				>
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
			</div>
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
