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
		CLEARED_LINE,
		DESCRIPTION_MAX,
		EXAMPLES,
		SCOPE_CHOICES,
		consumeBlankRequest,
		emptyForm,
		filledLine,
		formOf,
		isComplete,
		isUntouched,
		refusalOf,
		sameForm,
		savedLine,
		scopeLabel,
		startFromExample,
		toInput,
		type TemplateExample,
		type TemplateForm
	} from './resource-template';
	import { EMPTY_BODY, EMPTY_HEADING, NEW, SAVED, type TabId } from './tabs';

	/** A request from the page's header action to clear the form, owned there
	 *  and answered here.
	 *
	 *  It lives on the page because the header's "New template" is pressable
	 *  from the mapping tab, where this component is showing nothing; the form
	 *  itself is this component's state, so the page raises a flag rather than
	 *  reaching into it. Answering exactly once is the property that matters:
	 *  the effect below re-runs whenever anything it touches changes, and a
	 *  request left pending would clear the form again under whatever the
	 *  seller had begun typing.
	 *
	 *  `view` is which of this component's two tabs is showing, and `onview`
	 *  is how it asks for the other one — pressing Change on a saved row has
	 *  to bring the editor forward. One component for both tabs, and mounted
	 *  for the whole visit by the page's hidden wrapper: the editor holds what
	 *  the seller is typing, and any unmounting — a component per tab, or an
	 *  `{#if}` around this one — empties it behind their back. */
	let {
		blankRequested = $bindable(false),
		view,
		onview
	}: { blankRequested?: boolean; view: string; onview: (id: TabId) => void } = $props();

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

	/** What the last action put in the editor, and what it replaced.
	 *
	 *  `after` is what Undo measures against. Putting `before` back is only
	 *  free while the seller has written nothing since the action; where they
	 *  have, Undo asks before writing over it rather than dropping it silently.
	 *  The template being changed travels for its own reason: a cleared edit
	 *  put back without its id is a new template, and saving it would either be
	 *  refused for a name already taken or leave two records where the seller
	 *  meant to change one. The half-typed category travels because it is part
	 *  of the editor's state and not part of the form.
	 *
	 *  Both forms are snapshots. A form assigned to `form` is written through
	 *  by every field the seller types in, so a plain reference to it would
	 *  quietly become the current form and report that nothing had changed. */
	let started = $state<{
		line: string;
		before: { form: TemplateForm; editing: string | null; pending: string };
		after: { form: TemplateForm; pending: string };
	} | null>(null);
	/** Whether Undo has asked about the writing it would replace. One question
	 *  about one action, which is the whole of what keeps Undo from losing
	 *  work — there is no stack of actions here to walk back. */
	let undoAsking = $state(false);
	/** The custom category typed into its field but not yet entered.
	 *
	 *  Owned here rather than left in the input element, because this editor
	 *  is mounted for the whole visit: the element outlives the template. A
	 *  word left over a Save or a Clear would be carried into the next
	 *  template, where the seller never typed it. Held across the tab bar,
	 *  emptied wherever the editor starts on a different template. */
	let pendingCategory = $state('');
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
	/** The ceiling, but only where a save would create a template. Editing one
	 *  the seller already holds adds none, so the figure that disables New
	 *  template must not disable Save changes as well. */
	const capRefusal = $derived(editing === null ? capped : null);

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

	/** Clears the editor back to a blank template, offering Undo where there
	 *  was anything to lose. The header's "New template" reaches this from the
	 *  mapping tab, so it is also the path that could silently discard a form
	 *  the seller had begun.
	 *
	 *  A template being changed is something to lose in itself: clearing it
	 *  offers an Undo even where the form was left exactly as it was read, so
	 *  that the Undo puts the seller back on the record they had open. */
	function blank() {
		const before = $state.snapshot(form) as TemplateForm;
		const held = editing;
		const typed = pendingCategory;
		form = emptyForm();
		editing = null;
		pendingCategory = '';
		saveRefusal = null;
		listRefusal = null;
		undoAsking = false;
		started =
			held === null && typed === '' && isUntouched(before)
				? null
				: {
						line: CLEARED_LINE,
						before: { form: before, editing: held, pending: typed },
						after: { form: $state.snapshot(form) as TemplateForm, pending: '' }
					};
		onview(NEW);
	}

	/** Fills what the editor has not answered from one of the two examples.
	 *
	 *  Nothing is saved and no allowance is spent: an example is an unsaved
	 *  form until the seller presses Save, and switching between the two fills
	 *  only what is still blank rather than overwriting what they have
	 *  written. Undo puts back the form as it was, asking first where the
	 *  seller has written anything since. */
	function takeExample(example: TemplateExample) {
		const taken = startFromExample($state.snapshot(form) as TemplateForm, example);
		form = taken.form;
		// An example fills the form and nothing else, so the half-typed
		// category is the same word on both sides of it.
		started = {
			line: filledLine(example.label, taken.filled),
			before: { form: taken.before, editing, pending: pendingCategory },
			after: { form: $state.snapshot(form) as TemplateForm, pending: pendingCategory }
		};
		undoAsking = false;
		saveRefusal = null;
	}

	/** Undo, which puts back the form the action replaced and the template it
	 *  was changing. Where the seller has written since, it asks first: an
	 *  example fills what was blank, and the sentences they wrote afterwards
	 *  are not the example's to take away. */
	function undoStart() {
		if (started === null) {
			return;
		}
		if (!sameForm(form, started.after.form) || pendingCategory !== started.after.pending) {
			undoAsking = true;
			return;
		}
		revertStart();
	}

	function revertStart() {
		if (started === null) {
			return;
		}
		form = started.before.form;
		editing = started.before.editing;
		pendingCategory = started.before.pending;
		saveRefusal = null;
		started = null;
		undoAsking = false;
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
			// A different template: whatever was typed into the category field
			// belongs to the one the seller was writing before, not this one.
			pendingCategory = '';
			saveRefusal = null;
			started = null;
			undoAsking = false;
			onview(NEW);
		} catch (failure) {
			listRefusal = refusalFor(failure, 'We couldn’t open that template.');
		} finally {
			opening = null;
		}
	}

	/** Back to a blank new template. The editor is the first tab and is always
	 *  showing, so this empties it rather than hiding it — including the
	 *  category field's half-typed word, which a mounted editor would otherwise
	 *  carry into the next template. */
	function clear() {
		form = emptyForm();
		editing = null;
		pendingCategory = '';
		saveRefusal = null;
		started = null;
		undoAsking = false;
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
				// The plan is read once and held for the whole visit, so the
				// template just created leaves the cached usage one behind and
				// the cap gate answering from it. Refreshed rather than counted
				// here, because the page header reads the same figure. A change
				// creates nothing, so it refreshes nothing.
				await queryClient.invalidateQueries({ queryKey: queryKeys.entitlement });
			} else {
				await templates.update(editing, input);
				await queryClient.invalidateQueries({ queryKey: templateKeys.one(editing) });
			}
			await queryClient.invalidateQueries({ queryKey: templateKeys.all });
			clear();
		} catch (failure) {
			saveRefusal = refusalFor(failure, 'We couldn’t save that template.');
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
			// The other half of the same figure: a removal frees one, and
			// without this the seller is told the allowance is full while they
			// stand in front of the room they just made.
			await queryClient.invalidateQueries({ queryKey: queryKeys.entitlement });
			if (editing === head.id) {
				clear();
			}
		} catch (failure) {
			listRefusal = refusalFor(failure, 'We couldn’t remove that template.');
		} finally {
			removing = null;
		}
	}
</script>

<!-- The editor, which is the landing tab: a seller comes here to write a
     template, so the form is open on arrival rather than behind a press.

     Hidden rather than unmounted, for the reason the page hides this whole
     component: unmounting empties what the seller is typing. It is not only
     `form` that would go — a field still being typed into, such as a custom
     category before Enter, is held by its own input element and by nothing
     else, so it survives a look at the saved list only while that element
     does. `.tpl-pane` is `display: contents`, so the panel keeps the page's
     own spacing. -->
<div class="tpl-pane" hidden={view === SAVED}>
	<Panel
		title={editing === null ? 'New template' : 'Edit this template'}
		description="Fill in what a new resource should start with."
	>
		{#if editing === null}
			<!-- Two worked examples, above the fields they fill. Cards rather than
			     a select: each one says who it is written for and what it holds,
			     which two option labels cannot. Pressing one writes nothing
			     anywhere — the form is unsaved until Save, so neither arriving
			     here nor trying both spends a template out of the allowance. -->
			<div class="tpl-examples">
				<p class="tpl-examples-said">Pick an example, then edit it before you save.</p>
				<div class="tpl-example-cards">
					{#each EXAMPLES as example (example.id)}
						<div class="tpl-example">
							<span class="t">{example.label}</span>
							<span class="tpl-said">{example.summary}</span>
							<Button small onclick={() => takeExample(example)}>Use the {example.label}</Button>
						</div>
					{/each}
				</div>
			</div>
		{/if}

		{#if started !== null}
			<p class="tpl-note">
				{started.line}
				<Button small tier="quiet" onclick={undoStart}>Undo</Button>
			</p>
		{/if}

		{#if undoAsking}
			<p class="tpl-note">
				Undo brings back what you had and removes your later changes.
				<Button small tier="quiet" onclick={revertStart}>Undo anyway</Button>
				<Button small tier="quiet" onclick={() => (undoAsking = false)}>
					Keep what I have
				</Button>
			</p>
		{/if}

		<div class="tpl-grid">
			<Field label="Name" id="{base}-name" required hint="The name you’ll pick it by.">
				<input id="{base}-name" type="text" bind:value={form.name} />
			</Field>

			<Field
				label="Written for"
				id="{base}-scope"
				hint="Pick a marketplace to add its own questions too."
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

		<!-- Labelled "Template note" rather than "Description", which is what the
		     wire calls it: the resource's own Description is a band a few inches
		     below, and two fields under one word had the seller reading the hint
		     to tell them apart. The field, the write and the column are
		     unchanged. -->
		<Field
			label="Template note"
			id="{base}-description"
			hint="A note to yourself about when to use it."
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
					bind:pending={pendingCategory}
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
					? 'We couldn’t load the choices for this form.'
					: 'Loading the choices…'}
			</p>
		{/if}

		{#if saveRefusal !== null}
			<p class="tpl-refusal">{saveRefusal}</p>
		{/if}

		<div class="tpl-actions">
			<Button
				tier="additive"
				disabled={saving || blocked !== null || capRefusal !== null}
				reason={blocked ?? capRefusal ?? (saving ? 'Saving now.' : undefined)}
				onclick={() => void save()}
			>
				{saving ? 'Saving…' : editing === null ? 'Save template' : 'Save changes'}
			</Button>
			<Button tier="quiet" onclick={clear}>{editing === null ? 'Clear' : 'Cancel'}</Button>
		</div>
	</Panel>
</div>

<!-- The saved templates, their own tab. One branch, so a read that failed can
     never also render a claim about how many templates the seller has; the
     empty state is reached only from a read that succeeded and returned
     nothing. -->
{#if view === SAVED}
	{#if store.isPending}
		<p class="tpl-none">Loading your templates…</p>
	{:else if store.isError}
		<Banner tone="bad" title="We couldn’t load your templates">
			Nothing was changed.
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
							reason={busy ? 'Wait for the other template to finish.' : undefined}
							onclick={() => void change(head)}
						>
							{opening === head.id ? 'Opening…' : 'Change'}
						</Button>
						<Button
							small
							danger
							disabled={busy}
							reason={busy ? 'Wait for the other template to finish.' : undefined}
							onclick={() => void remove(head)}
						>
							{removing === head.id ? 'Removing…' : 'Remove'}
						</Button>
					</span>
				</div>
			{/each}
		</Panel>
	{:else}
		{#if listRefusal !== null}
			<p class="tpl-refusal">{listRefusal}</p>
		{/if}
		<!-- The action is the other tab, not a second blank: the editor there
		     is already open, and clearing it from here would discard a form the
		     seller may have begun before they came to look at the list. -->
		<Placeholder icon="layout-template" headline={EMPTY_HEADING} body={EMPTY_BODY}>
			{#snippet actions()}
				<Button
					tier="primary"
					icon="circle-plus"
					disabled={capped !== null}
					reason={capped ?? undefined}
					onclick={() => onview(NEW)}
				>
					New template
				</Button>
			{/snippet}
		</Placeholder>
	{/if}
{/if}
