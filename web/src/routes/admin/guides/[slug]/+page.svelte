<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { ApiFailure, api, type GuideStatus } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import { utcInstant } from '$lib/elapsed';
	import Field from '$lib/Field.svelte';
	import Icon from '$lib/Icon.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import Toggle from '$lib/Toggle.svelte';
	import { toast } from '$lib/toast';
	import { markUp, type MarkKind } from '$lib/tpt-form';
	import { dirty, draftRefusal, imageMarkdown, insertAt } from '$lib/pages/guides/editor';
	import '$lib/pages/guides/guides.css';

	const slug = $derived(page.params.slug ?? '');
	const queryClient = useQueryClient();

	const guide = createQuery(() => ({
		queryKey: queryKeys.adminGuide(slug),
		queryFn: () => api.adminGuide(slug),
		enabled: slug.length > 0
	}));

	const saved = $derived(guide.data);

	let title = $state('');
	let body = $state('');
	let published = $state(false);
	/** The wire's own word for the toggle's position. Kept derived rather than
	 *  held twice: two states for one fact drift. */
	const status = $derived<GuideStatus>(published ? 'published' : 'draft');
	/** Which guide the three fields above were seeded from, so a second read
	 *  of the same guide — a refetch after a save — does not overwrite what is
	 *  being typed, and navigating to another guide does. */
	let seeded = $state<string | null>(null);
	let box = $state<HTMLTextAreaElement | null>(null);
	let uploading = $state(false);

	$effect(() => {
		const view = saved;
		if (view !== undefined && seeded !== view.slug) {
			title = view.title;
			body = view.body;
			published = view.status === 'published';
			seeded = view.slug;
		}
	});

	const draft = $derived({ title, body, status });
	const unsaved = $derived(
		saved === undefined
			? false
			: dirty({ title: saved.title, body: saved.body, status: saved.status }, draft)
	);
	const refusal = $derived(draftRefusal(draft));

	const saving = createMutation(() => ({
		mutationFn: () => api.saveGuide(slug, { title: title.trim(), body, status }),
		onSuccess: (view) => {
			// The server's own answer, not the form's idea of it: the rendered
			// html the preview draws can only come from the body the server
			// stored.
			queryClient.setQueryData(queryKeys.adminGuide(slug), view);
			void queryClient.invalidateQueries({ queryKey: queryKeys.adminGuides });
			void queryClient.invalidateQueries({ queryKey: queryKeys.guides });
			void queryClient.invalidateQueries({ queryKey: queryKeys.guide(slug) });
			title = view.title;
			body = view.body;
			published = view.status === 'published';
			toast('info', view.status === 'published' ? 'Saved and published.' : 'Saved as a draft.');
		},
		onError: (failure: Error) =>
			toast('error', failure instanceof ApiFailure ? failure.message : 'The guide was not saved.')
	}));

	const deleting = createMutation(() => ({
		mutationFn: () => api.deleteGuide(slug),
		onSuccess: async () => {
			await queryClient.invalidateQueries({ queryKey: queryKeys.adminGuides });
			await goto('/admin/guides');
		},
		onError: (failure: Error) =>
			toast(
				'error',
				failure instanceof ApiFailure ? failure.message : 'The guide was not deleted.'
			)
	}));

	/** One toolbar button: Markdown written around what the operator selected,
	 *  and the caret put back where they would type next. The same `markUp` the
	 *  resource description uses, so the two toolbars cannot come to write
	 *  different Markdown. */
	function format(kind: MarkKind) {
		if (box === null) {
			return;
		}
		const written = box;
		const marked = markUp(written.value, written.selectionStart, written.selectionEnd, kind);
		body = marked.text;
		// After the render that carries the new value, or the browser would
		// restore the caret into the old one.
		queueMicrotask(() => {
			written.focus();
			written.setSelectionRange(marked.start, marked.end);
		});
	}

	/** A picture, stored against the platform's own organisation and written
	 *  into the body as the path every seller reads it back at. */
	async function insertImage(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		if (file === undefined) {
			return;
		}
		uploading = true;
		try {
			const { handle } = await api.uploadGuideImage(file);
			const written = box;
			const at = written === null ? body.length : written.selectionStart;
			const to = written === null ? body.length : written.selectionEnd;
			const inserted = insertAt(body, at, to, imageMarkdown(handle));
			body = inserted.text;
			if (written !== null) {
				queueMicrotask(() => {
					written.focus();
					written.setSelectionRange(inserted.caret, inserted.caret);
				});
			}
			toast('info', 'Picture inserted. Save to publish it with the guide.');
		} catch (failure) {
			toast(
				'error',
				failure instanceof ApiFailure ? failure.message : 'The picture was not stored.'
			);
		} finally {
			uploading = false;
			// So choosing the same file again fires another change.
			input.value = '';
		}
	}
</script>

<div class="page">
	<PageHead
		icon="book-open"
		title={saved?.title ?? 'Guide'}
		description={`/guides/${slug}`}
		back={{ href: '/admin/guides', label: 'All guides' }}
	/>

	{#if guide.isPending}
		<Panel><p class="quiet">Reading this guide…</p></Panel>
	{:else if guide.isError}
		<Panel>
			<Placeholder
				icon="book-open"
				headline="This guide could not be read"
				body="Either nothing holds that address, or the request did not come back with an
					answer we can act on. The guides list is the place to go from here."
			/>
		</Panel>
	{:else if saved !== undefined}
		<div class="gd-split">
			<Panel title="Write">
				<Field label="Title" id="guide-title" required>
					<input id="guide-title" type="text" bind:value={title} />
				</Field>

				<div class="gd-md" role="group" aria-label="Formatting">
					<button type="button" class="gd-md-b" onclick={() => format('bold')}>
						<b>B</b><span class="sr-only">Bold</span>
					</button>
					<button type="button" class="gd-md-b" onclick={() => format('italic')}>
						<i>I</i><span class="sr-only">Italic</span>
					</button>
					<button type="button" class="gd-md-b" onclick={() => format('bullets')}>
						•<span class="sr-only">Bulleted list</span>
					</button>
					<button type="button" class="gd-md-b" onclick={() => format('numbers')}>
						1.<span class="sr-only">Numbered list</span>
					</button>
					<label class="gd-md-b" aria-disabled={uploading}>
						<Icon name="image" size={14} />
						{uploading ? 'Storing…' : 'Picture'}
						<input
							class="sr-only"
							type="file"
							accept="image/*"
							disabled={uploading}
							onchange={insertImage}
						/>
					</label>
				</div>

				<label class="sr-only" for="guide-body">Guide body, in Markdown</label>
				<textarea
					id="guide-body"
					class="gd-body"
					bind:this={box}
					bind:value={body}
					spellcheck="true"
					placeholder="Markdown. Headings, lists, tables, links and pictures; raw HTML is escaped rather than rendered."
				></textarea>

				<div class="gd-bar">
					<Toggle bind:checked={published} label="Published" />
					<span class="spacer"></span>
					<Button
						tier="outline"
						danger
						disabled={deleting.isPending}
						onclick={() => {
							if (confirm(`Delete “${saved.title}”? The body goes with it.`)) {
								deleting.mutate();
							}
						}}
					>
						Delete
					</Button>
					<Button
						tier="primary"
						disabled={refusal !== null || !unsaved || saving.isPending}
						reason={refusal ??
							(saving.isPending
								? 'The guide is being saved.'
								: unsaved
									? undefined
									: 'Nothing has changed since the last save.')}
						onclick={() => saving.mutate()}
					>
						{saving.isPending ? 'Saving…' : 'Save'}
					</Button>
				</div>
				<p class="gd-state">
					{#if status === 'published'}
						Published — every signed-in seller can read this at /guides/{slug}.
					{:else}
						A draft. The reader's routes 404 it, so nobody but an operator can read it.
					{/if}
					Last saved {utcInstant(saved.updated_at)}.
				</p>
			</Panel>

			<Panel title="Preview">
				{#snippet more()}
					{#if unsaved}
						<StatusPill tone="warn" label="unsaved changes" />
					{:else}
						<StatusPill tone="ok" label="up to date" />
					{/if}
				{/snippet}
				<!-- The server's rendering of the body it stored, not a second
				     renderer in the browser: this console has no Markdown
				     renderer, and adding one would mean the preview and the
				     published page could disagree. Raw HTML is escaped during
				     that rendering, which is what makes `{@html}` safe here. -->
				<div class="guide-body">{@html saved.html}</div>
				<p class="foot-note">
					Preview shows the last saved version, rendered by the server exactly as a seller
					will see it. Save to refresh it.
				</p>
			</Panel>
		</div>
	{/if}
</div>
