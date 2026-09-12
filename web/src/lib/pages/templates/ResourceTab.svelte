<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import { licenceGated, licenceOptions } from '$lib/authoring';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { entitlementRead, limitOf } from '$lib/entitlement-read';
	import Field from '$lib/Field.svelte';
	import type { InventoryId } from '$lib/generated/vocab';
	import { MARKETPLACE_OF } from '$lib/listings-view';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import { GRADE_LABELS_KEY, type GradeLabels, type TptDraft } from '$lib/tpt-form';
	import CategoriesPanel from '$lib/pages/resources/panels/CategoriesPanel.svelte';
	import DescriptionPanel from '$lib/pages/resources/panels/DescriptionPanel.svelte';
	import DetailsPanel from '$lib/pages/resources/panels/DetailsPanel.svelte';
	import MarketplacePanel from '$lib/pages/resources/panels/MarketplacePanel.svelte';
	import PricePanel from '$lib/pages/resources/panels/PricePanel.svelte';
	import StandardsPanel from '$lib/pages/resources/panels/StandardsPanel.svelte';
	import StatusPanel from '$lib/pages/resources/panels/StatusPanel.svelte';
	import '$lib/pages/resources/resources.css';
	import { templateKeys, templates, type TemplateHead } from './api';
	import {
		DESCRIPTION_MAX,
		SCOPE_CHOICES,
		consumeBlankRequest,
		emptyForm,
		formOf,
		isComplete,
		refusalOf,
		savedLine,
		scopeLabel,
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

	// The same figure the page header's control reads, so the two "New
	// template" controls cannot disagree about whether one more will fit.
	const plan = createQuery(() => entitlementRead);
	const capped = $derived(limitOf(plan.data, 'templates'));

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

	const held = $derived(vocabulary.data ?? null);
	const scoped = $derived(form.scope === '' ? null : (form.scope as InventoryId));

	/** What the scoped marketplace itself declares. Read only where a scope is
	 *  chosen: a generic template shows no marketplace panel, so there is
	 *  nothing for this to answer. */
	const platform = createQuery(() => ({
		queryKey: queryKeys.vocabulary(scoped ?? 'Tpt'),
		queryFn: () => api.vocabulary(scoped ?? 'Tpt'),
		staleTime: Infinity,
		enabled: scoped !== null
	}));

	const known = $derived(
		scoped === null || platform.data === undefined
			? new Map()
			: new Map([[scoped, platform.data]])
	);
	const gatesLicence = $derived(scoped !== null && licenceGated([scoped], known).length > 0);
	const licences = $derived(
		scoped === null ? [] : licenceOptions(platform.data, form.draft.free ? 'free' : 'paid')
	);

	/** Which words the grade grid is written in. The same browser-held answer
	 *  the create form reads, so a seller who works in years works in years
	 *  here too. */
	let gradeLabels = $state<GradeLabels>('american');
	$effect(() => {
		const stored = localStorage.getItem(GRADE_LABELS_KEY);
		if (stored === 'american' || stored === 'british') {
			gradeLabels = stored;
		}
	});

	$effect(() => {
		const answer = consumeBlankRequest(blankRequested);
		if (answer.open) {
			blankRequested = answer.pending;
			blank();
		}
	});

	function setGradeLabels(chosen: GradeLabels) {
		gradeLabels = chosen;
		localStorage.setItem(GRADE_LABELS_KEY, chosen);
	}

	/** One field of the draft the bands write into. The same signature the
	 *  create form passes them, which is what lets the bands be the same
	 *  components rather than two sets that drifted. */
	function set<K extends keyof TptDraft>(field: K, value: TptDraft[K]) {
		form = { ...form, draft: { ...form.draft, [field]: value } };
	}

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
		description="What you fill in here is what a new resource starts with. Leave the rest empty and we will ask as usual."
	>
		<div class="tpl-grid">
			<Field label="Name" id="{base}-name" required hint="What you will pick it by.">
				<input id="{base}-name" type="text" bind:value={form.name} />
			</Field>

			<Field
				label="Written for"
				id="{base}-scope"
				hint="A template written for one marketplace also holds that marketplace's own questions."
			>
				<select id="{base}-scope" bind:value={form.scope}>
					{#each SCOPE_CHOICES as choice (choice.label)}
						<option
							value={choice.value}
							disabled={choice.reason !== null}
							title={choice.reason ?? undefined}
						>
							{choice.label}{choice.reason === null ? '' : ` — ${choice.reason}`}
						</option>
					{/each}
				</select>
			</Field>
		</div>

		<Field
			label="Description"
			id="{base}-description"
			hint="A note to yourself about when to reach for this template. It is not the resource's own description, which is the band below."
		>
			<textarea id="{base}-description" rows="2" bind:value={form.description}></textarea>
			<span class="tpl-count" class:over={form.description.length > DESCRIPTION_MAX}>
				{form.description.length} of {DESCRIPTION_MAX} characters
			</span>
		</Field>

		{#if held}
			<!-- The whole new-resource form, band for band, over the same
			     components the create form renders: a template is a partial
			     `DraftInput`, so anything that form can answer this one can hold
			     as a starting point. Four bands are absent and each for the same
			     reason — the title, the files, the previews and the thumbnails
			     are made per resource rather than chosen once, and the
			     marketplace grid asks where a resource goes rather than what it
			     says.

			     No refusals are passed. Every field here is optional by
			     construction: an unanswered one is left out of the draft and the
			     create form asks for it as usual. -->
			<!-- `resources.css` scopes every band rule under `.resources-page`, so
			     the bands carry that hook wherever they are rendered. It is the
			     form's scope rather than one page's: without it the same
			     components would draw here with no hairlines, no gutters and no
			     heading rhythm. -->
			<div class="resources-page tpl-form">
				<!-- `optional` on every band: the panel says "leave the rest empty
				     and we will ask as usual", so a Required chip here would tell
				     the seller the opposite of what the panel and the write both
				     do. The create form leaves the prop alone and is unchanged. -->
				<DescriptionPanel draft={form.draft} form={held} optional {set} />
				<PricePanel draft={form.draft} form={held} optional {set} />
				<CategoriesPanel
					draft={form.draft}
					form={held}
					{gradeLabels}
					onLabels={setGradeLabels}
					optional
					{set}
				/>
				<StandardsPanel draft={form.draft} form={held} {set} />
				<DetailsPanel draft={form.draft} form={held} {set} />
				{#if scoped !== null}
					<MarketplacePanel
						marketplace={MARKETPLACE_OF[scoped]}
						draft={form.draft}
						form={held}
						{gatesLicence}
						{licences}
						optional
						{set}
					/>
				{/if}
				<StatusPanel draft={form.draft} form={held} blank="Ask me each time" {set} />
			</div>
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
	<Banner tone="bad" title="We could not read your templates">
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
					<span class="t">
						{head.name}
						<span class="tpl-chip" class:generic={head.scope === null}>
							{scopeLabel(head.scope)}
						</span>
					</span>
					{#if head.description !== null && head.description.length > 0}
						<span class="tpl-said">{head.description}</span>
					{/if}
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
			<Button
				tier="primary"
				icon="circle-plus"
				disabled={capped !== null}
				reason={capped ?? undefined}
				onclick={blank}
			>
				New template
			</Button>
		{/snippet}
	</Placeholder>
{/if}
