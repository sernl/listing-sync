<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import FacetPicker from '$lib/FacetPicker.svelte';
	import Field from '$lib/Field.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import Toggle from '$lib/Toggle.svelte';
	import { queryKeys } from '$lib/query';
	import { templateKeys, templates, type TemplateHead } from './api';
	import {
		consumeBlankRequest,
		emptyForm,
		formOf,
		isComplete,
		refusalOf,
		savedLine,
		toInput
	} from './resource-template';
	import { EMPTY_BODY, EMPTY_HEADING } from './tabs';

	/** A request from the page's header action for a blank form, owned there
	 *  and cleared here.
	 *
	 *  It lives on the page because this component is destroyed while the
	 *  seller is on the other tab, and the header's "New template" can be
	 *  pressed from there — which is the landing tab, so it is the first press
	 *  a seller makes. Clearing it here is what makes a mount and a raised
	 *  request distinguishable: arriving on this tab with nothing pending
	 *  opens no form, and arriving with a request pending opens one. */
	let { blankRequested = $bindable(false) }: { blankRequested?: boolean } = $props();

	const base = $props.id();
	const queryClient = useQueryClient();

	const store = createQuery(() => ({
		queryKey: templateKeys.all,
		queryFn: () => templates.list()
	}));

	// The same controlled lists the create form renders, so a template's
	// starting points are chosen from the marketplace's own vocabulary rather
	// than typed. One cache entry, shared with the create form.
	const vocabulary = createQuery(() => ({
		queryKey: queryKeys.formVocabulary,
		queryFn: () => api.formVocabulary(),
		staleTime: Infinity
	}));

	const rows = $derived(store.data ?? []);
	const now = Date.now();

	let creating = $state(false);
	let form = $state(emptyForm());
	let editing = $state<string | null>(null);
	let opening = $state<string | null>(null);
	let removing = $state<string | null>(null);
	let saving = $state(false);
	/** A refusal about the form, shown beside the controls that caused it. */
	let saveRefusal = $state<string | null>(null);
	/** A refusal about the list — a remove or an open that failed. Its own
	 *  state because it is shown in its own place: the form's panel is closed
	 *  in exactly the case a remove fails, so one shared refusal would be
	 *  written to a paragraph that is not in the document. */
	let listRefusal = $state<string | null>(null);

	const blocked = $derived(refusalOf(form));
	/** One request at a time over the list. A second DELETE inside the first
	 *  one's window answers 404, and the seller would be told a removal failed
	 *  while watching the row go. */
	const busy = $derived(opening !== null || removing !== null);

	$effect(() => {
		const answer = consumeBlankRequest(blankRequested);
		if (answer.open) {
			blankRequested = answer.pending;
			blank();
		}
	});

	/** What a failed call says. The server's own words where it gave any: a
	 *  refusal it decided, such as a name already taken or the ceiling on how
	 *  many templates an organisation may hold, is more use than this file's
	 *  guess at what went wrong. */
	function refusalFor(failure: unknown, fallback: string): string {
		return failure instanceof ApiFailure ? failure.message : fallback;
	}

	function blank() {
		form = emptyForm();
		editing = null;
		saveRefusal = null;
		listRefusal = null;
		creating = true;
	}

	/** Opens a saved template for editing.
	 *
	 *  The draft arrives from `GET /v1/templates/{id}` rather than from the row:
	 *  the list serves heads and carries no draft to load. Read on every press
	 *  rather than from a cached copy: the client sets no `staleTime`, and an
	 *  edit form should show what is stored now rather than what was stored
	 *  when the seller last looked. */
	async function change(head: TemplateHead) {
		if (busy) {
			return;
		}
		opening = head.id;
		listRefusal = null;
		try {
			const template = await queryClient.fetchQuery({
				queryKey: templateKeys.one(head.id),
				queryFn: () => templates.read(head.id)
			});
			form = formOf(template);
			editing = template.id;
			saveRefusal = null;
			creating = true;
		} catch (failure) {
			listRefusal = refusalFor(failure, 'That template could not be opened.');
		} finally {
			opening = null;
		}
	}

	function close() {
		form = emptyForm();
		editing = null;
		saveRefusal = null;
		creating = false;
	}

	async function save() {
		if (!isComplete(form)) {
			return;
		}
		saving = true;
		saveRefusal = null;
		try {
			const input = toInput(form);
			if (editing === null) {
				await templates.create(input);
			} else {
				await templates.update(editing, input);
				await queryClient.invalidateQueries({ queryKey: templateKeys.one(editing) });
			}
			await queryClient.invalidateQueries({ queryKey: templateKeys.all });
			close();
		} catch (failure) {
			saveRefusal = refusalFor(failure, 'That template was not saved.');
		} finally {
			saving = false;
		}
	}

	async function remove(head: TemplateHead) {
		if (busy) {
			return;
		}
		removing = head.id;
		listRefusal = null;
		try {
			await templates.remove(head.id);
			queryClient.removeQueries({ queryKey: templateKeys.one(head.id) });
			await queryClient.invalidateQueries({ queryKey: templateKeys.all });
			if (editing === head.id) {
				close();
			}
		} catch (failure) {
			listRefusal = refusalFor(failure, 'That template was not removed.');
		} finally {
			removing = null;
		}
	}
</script>

{#if creating}
	<Panel
		title={editing === null ? 'New template' : 'Change this template'}
		description="Everything you fill in here is what a new resource starts out with; leave the rest empty to be asked as usual."
	>
		<div class="tpl-grid">
			<Field label="Name" id="{base}-name" required hint="What you will pick it by.">
				<input id="{base}-name" type="text" bind:value={form.name} />
			</Field>
		</div>

		{#if vocabulary.data}
			<div class="tpl-stack">
				<FacetPicker
					label="Subjects"
					placeholder="Search your subjects"
					facets={vocabulary.data.subject_areas}
					chosen={form.subjects}
					cap={vocabulary.data.caps.subject_areas ?? null}
					onChange={(values) => (form.subjects = values)}
				/>
				<FacetPicker
					label="Year levels"
					placeholder="Search year levels"
					facets={vocabulary.data.grades}
					chosen={form.grades}
					cap={vocabulary.data.caps.grades ?? null}
					onChange={(values) => (form.grades = values)}
				/>
			</div>

			<div class="tpl-grid">
				<Field label="Licence" id="{base}-licence" hint={vocabulary.data.copyright.preamble}>
					<select id="{base}-licence" bind:value={form.licence}>
						<option value="">Ask me each time</option>
						{#each vocabulary.data.copyright.options as option (option.id)}
							<option value={option.id}>{option.label}</option>
						{/each}
					</select>
				</Field>

				<Field
					label="Price"
					id="{base}-price"
					hint="Dollars and cents, like 4.50 — leave it empty to be asked each time."
				>
					<input
						id="{base}-price"
						type="text"
						inputmode="decimal"
						placeholder="0.00"
						disabled={form.free}
						bind:value={form.price}
					/>
				</Field>
			</div>

			<Toggle label="Free resource" bind:checked={form.free} />
		{:else}
			<p class="tpl-none">
				{vocabulary.isError
					? 'The lists a template chooses from could not be read.'
					: 'Reading the lists a template chooses from…'}
			</p>
		{/if}

		{#if saveRefusal !== null}
			<p class="tpl-refusal">{saveRefusal}</p>
		{/if}

		<div class="tpl-actions">
			<Button
				tier="additive"
				disabled={saving || blocked !== null}
				reason={blocked ?? (saving ? 'Saving now.' : undefined)}
				onclick={() => void save()}
			>
				{saving ? 'Saving…' : editing === null ? 'Save template' : 'Save changes'}
			</Button>
			<Button tier="quiet" onclick={close}>Cancel</Button>
		</div>
	</Panel>
{/if}

<!-- One branch, so a read that failed can never also render a claim about how
     many templates the seller has. The empty state is reached only from a read
     that succeeded and returned nothing. -->
{#if store.isPending}
	<p class="tpl-none">Reading your templates…</p>
{:else if store.isError}
	<Banner tone="bad" title="Your templates could not be read">
		Nothing has been changed. Reload the page to try again.
	</Banner>
{:else if rows.length > 0}
	<Panel title="Your templates">
		{#if listRefusal !== null}
			<p class="tpl-refusal">{listRefusal}</p>
		{/if}
		{#each rows as head (head.id)}
			<div class="tpl-row">
				<span class="who">
					<span class="t">{head.name}</span>
					<span class="meta">{savedLine(head, now)}</span>
				</span>
				<span class="tpl-row-acts">
					<Button
						small
						disabled={busy}
						reason={busy ? 'One template at a time.' : undefined}
						onclick={() => void change(head)}
					>
						{opening === head.id ? 'Opening…' : 'Change'}
					</Button>
					<Button
						small
						danger
						disabled={busy}
						reason={busy ? 'One template at a time.' : undefined}
						onclick={() => void remove(head)}
					>
						{removing === head.id ? 'Removing…' : 'Remove'}
					</Button>
				</span>
			</div>
		{/each}
	</Panel>
{:else if !creating}
	{#if listRefusal !== null}
		<p class="tpl-refusal">{listRefusal}</p>
	{/if}
	<Placeholder icon="layout-template" headline={EMPTY_HEADING} body={EMPTY_BODY}>
		{#snippet actions()}
			<Button tier="primary" icon="circle-plus" onclick={blank}>New template</Button>
		{/snippet}
	</Placeholder>
{/if}
