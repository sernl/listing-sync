<script lang="ts">
	// Resource templates as one guided flow: pick a template (or start one),
	// see what it sets — the template drawn as an arrow to the fields it fills —
	// and apply it to ticked resources, with Apply in the step's sticky footer.
	// Writing or changing a template is the editor inside step 2.
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api, type ProductHead } from '$lib/api';
	import { licenceGated, licenceOptions } from '$lib/authoring';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { entitlementRead, limitOf } from '$lib/entitlement-read';
	import Field from '$lib/Field.svelte';
	import type { InventoryId } from '$lib/generated/vocab';
	import { MARKETPLACE_OF } from '$lib/listings-view';
	import Explain from '$lib/Explain.svelte';
	import FlowDiagram from '$lib/FlowDiagram.svelte';
	import FlowStep from '$lib/FlowStep.svelte';
	import Pagination from '$lib/Pagination.svelte';
	import Stepper, { type StepMark } from '$lib/Stepper.svelte';
	import { formatPrice } from '$lib/listings-view';
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
	import {
		templateKeys,
		templates,
		type TemplateApplyPlanView,
		type TemplateHead,
		type TemplateView
	} from './api';
	import {
		CLEARED_LINE,
		DESCRIPTION_MAX,
		EXAMPLES,
		SCOPE_CHOICES,
		VERDICT_WORDS,
		fieldWordsOf,
		fieldsSet,
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
	import '$lib/flow.css';

	/** A request from the page's header action to clear the editor, owned
	 *  there and answered here: the header's "New template" is pressable from
	 *  the mapping tab, where this component is showing nothing, and the form
	 *  is this component's state. Answering exactly once is the property that
	 *  matters, which `consumeBlankRequest` holds.
	 *
	 *  This component is mounted for the whole visit by the page's hidden
	 *  wrapper: the editor holds what the seller is typing, and unmounting it
	 *  would empty it behind their back. */
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
		composing = true;
		chosen = null;
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
			composing = true;
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
			let stored: TemplateView;
			if (editing === null) {
				stored = await templates.create(input);
				// The plan is read once and held for the whole visit, so the
				// template just created leaves the cached usage one behind and
				// the cap gate answering from it. Refreshed rather than counted
				// here, because the page header reads the same figure. A change
				// creates nothing, so it refreshes nothing.
				await queryClient.invalidateQueries({ queryKey: queryKeys.entitlement });
			} else {
				stored = await templates.update(editing, input);
				await queryClient.invalidateQueries({ queryKey: templateKeys.one(editing) });
			}
			await queryClient.invalidateQueries({ queryKey: templateKeys.all });
			clear();
			// The saved template is the one picked: the next thing a seller
			// does with a template they just wrote is apply it.
			composing = false;
			chosen = stored.id;
			pickedView = stored;
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
			if (chosen === head.id) {
				chosen = null;
				pickedView = null;
			}
		} catch (failure) {
			listRefusal = refusalFor(failure, 'We couldn’t remove that template.');
		} finally {
			removing = null;
		}
	}

	// --- the flow --------------------------------------------------------------

	/** Whether step 2 shows the editor (writing or changing a template) rather
	 *  than what the picked template sets. The editor stays mounted either way,
	 *  hidden, so what the seller typed survives a look at another template. */
	let composing = $state(false);
	/** The saved template picked in step 1, and its draft once read. */
	let chosen = $state<string | null>(null);
	let pickedView = $state<TemplateView | null>(null);
	let pickRefusal = $state<string | null>(null);

	const chosenHead = $derived(rows.find((head) => head.id === chosen) ?? null);

	/** Picks a saved template and reads its draft, which the list does not
	 *  carry, so step 2 can say what it sets. */
	async function pickTemplate(head: TemplateHead) {
		chosen = head.id;
		composing = false;
		pickRefusal = null;
		if (pickedView?.id === head.id) {
			return;
		}
		pickedView = null;
		try {
			pickedView = await queryClient.fetchQuery({
				queryKey: templateKeys.one(head.id),
				queryFn: () => templates.read(head.id)
			});
		} catch (failure) {
			pickRefusal = refusalFor(failure, 'We couldn’t read that template.');
		}
	}

	/** What step 2 draws: the template being written, or the one picked. */
	const setsName = $derived(
		composing ? form.name.trim() || 'New template' : (chosenHead?.name ?? 'Template')
	);
	const setsFields = $derived(
		composing
			? fieldsSet(toInput(form).draft)
			: pickedView === null
				? []
				: fieldsSet(pickedView.draft)
	);

	// --- step 3: apply to ------------------------------------------------------

	const PICK_PER_PAGE = 25;
	let products = $state<ProductHead[]>([]);
	let ticked = $state<Set<string>>(new Set());
	let pickPage = $state(1);
	let pickCursors = $state<(string | null)[]>([null]);
	let pickNext = $state<string | null>(null);
	let pickBusy = $state(false);
	let pickLoaded = $state(false);
	let pickUnread = $state(false);
	let overwrite = $state(false);
	let applyPlan = $state<TemplateApplyPlanView | null>(null);
	let previewing = $state(false);
	let applying = $state(false);
	let applyRefusal = $state<string | null>(null);
	let applied = $state<string | null>(null);
	/** Minted once per preview the seller confirms and held across a retry, so a
	 *  second press after a dropped answer replays the same apply rather than
	 *  writing the catalogue twice. */
	let key: string | null = null;

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

	// The tick list is read once a template is picked: a seller writing one has
	// nothing to apply yet.
	$effect(() => {
		if (chosen !== null && !pickLoaded && !pickBusy) {
			void readProducts(null, 1);
		}
	});

	// A different template, tick set or overwrite answer retires the preview:
	// a table drawn for another ask is not what Apply would do.
	$effect(() => {
		void chosen;
		void overwrite;
		void ticked;
		applyPlan = null;
		key = null;
		applyRefusal = null;
	});

	function tickOne(product: string, on: boolean) {
		const next = new Set(ticked);
		if (on) {
			next.add(product);
		} else {
			next.delete(product);
		}
		ticked = next;
	}

	const allTickedHere = $derived(
		products.length > 0 && products.every((one) => ticked.has(one.id))
	);

	function tickPage() {
		const next = new Set(ticked);
		for (const one of products) {
			if (allTickedHere) {
				next.delete(one.id);
			} else {
				next.add(one.id);
			}
		}
		ticked = next;
	}

	const willChange = $derived(applyPlan?.counts.will_change ?? 0);
	const previewBlocked = $derived.by(() => {
		if (chosen === null) {
			return composing ? 'Save the template first.' : 'Pick a template first.';
		}
		if (ticked.size === 0) {
			return 'Tick at least one resource.';
		}
		if (previewing) {
			return 'Loading the preview.';
		}
		if (applying) {
			return 'Applying now.';
		}
		return null;
	});
	const applyBlocked = $derived.by(() => {
		if (applying) {
			return 'Applying now.';
		}
		if (previewBlocked !== null) {
			return previewBlocked;
		}
		if (applyPlan === null) {
			return 'Preview first to see what will change.';
		}
		if (willChange === 0) {
			return 'Nothing would change.';
		}
		return null;
	});

	function applyBody() {
		return { selection: { products: [...ticked] }, overwrite };
	}

	async function previewApply() {
		if (chosen === null || previewBlocked !== null) {
			return;
		}
		previewing = true;
		applyRefusal = null;
		applied = null;
		try {
			applyPlan = await templates.applyPlan(chosen, applyBody());
			key = null;
		} catch (failure) {
			applyPlan = null;
			applyRefusal = refusalFor(failure, 'The preview did not load. Nothing was changed.');
		} finally {
			previewing = false;
		}
	}

	async function apply() {
		if (chosen === null || applyBlocked !== null) {
			return;
		}
		applying = true;
		applyRefusal = null;
		try {
			key ??= crypto.randomUUID();
			const ack = await templates.apply(chosen, applyBody(), key);
			// Every figure stated, zeroes included: "changed 12" alone leaves a
			// seller who ticked fifteen wondering about three.
			const parts = [
				`${ack.changed} ${ack.changed === 1 ? 'resource' : 'resources'} changed`,
				`${ack.unchanged} left as ${ack.unchanged === 1 ? 'it was' : 'they were'}`
			];
			if (ack.blocked > 0) {
				parts.push(`${ack.blocked} skipped`);
			}
			applied = `${parts.join(', ')}.`;
			applyPlan = null;
			key = null;
			await queryClient.invalidateQueries({ queryKey: queryKeys.products });
		} catch (failure) {
			applyRefusal = refusalFor(
				failure,
				'The template was not applied. Try again; nothing will be done twice.'
			);
		} finally {
			applying = false;
		}
	}

	/** Which of the footer's two controls is primary: Apply once a preview is
	 *  in hand, Preview before. */
	const primaryIsApply = $derived(applyPlan !== null);

	let pickOpen = $state(true);
	let setsOpen = $state(true);
	let applyOpen = $state(true);

	const steps = $derived<StepMark[]>([
		{ id: 'pick', label: 'Pick', done: chosen !== null },
		{ id: 'sets', label: 'What it sets', done: chosen !== null && pickedView !== null },
		{ id: 'apply', label: 'Apply to', done: applied !== null }
	]);
</script>

<Stepper {steps} label="Template steps" />

<div class="flow">
	<FlowStep
		n={1}
		id="pick"
		title="Pick a template"
		hint="Pick one of yours, or start a new one."
		summary={composing ? 'Writing a new template' : (chosenHead?.name ?? 'None picked')}
		done={chosen !== null}
		bind:open={pickOpen}
	>
		{#snippet aside()}
			<Explain title="What a template is" label="">
				<p>
					A template holds answers you give the same way each time: subject, year levels,
					licence, price. It fills them in on a resource so you don’t type them again.
				</p>
				<p>It never sets the title, the files or the pictures. Those are per resource.</p>
			</Explain>
		{/snippet}

		{#if store.isPending}
			<p class="tpl-none">Loading your templates…</p>
		{:else if store.isError}
			<Banner tone="bad" title="We couldn’t load your templates">Nothing was changed.</Banner>
		{/if}

		<div class="chip-row" role="radiogroup" aria-label="Your templates">
			{#each rows as head (head.id)}
				<button
					type="button"
					role="radio"
					class="choice-chip solo tpl-pick"
					aria-checked={!composing && chosen === head.id}
					title={savedLine(head, now)}
					onclick={() => void pickTemplate(head)}
				>
					<strong>{head.name}</strong>
					<span class="tpl-chip" class:generic={head.scope === null}>{scopeLabel(head.scope)}</span>
				</button>
			{/each}
			<button
				type="button"
				role="radio"
				class="choice-chip solo tpl-pick tpl-pick-new"
				aria-checked={composing && editing === null}
				disabled={capped !== null}
				title={capped ?? undefined}
				onclick={blank}
			>
				+ New template
			</button>
		</div>
		{#if store.isSuccess && rows.length === 0}
			<p class="tpl-none">No templates yet. Start one, then apply it.</p>
		{/if}
		{#if listRefusal !== null}
			<p class="tpl-refusal">{listRefusal}</p>
		{/if}
	</FlowStep>

	<FlowStep
		n={2}
		id="sets"
		title="What it sets"
		hint={composing ? 'Fill in what a new resource should start with.' : 'The fields this template fills.'}
		summary={setsFields.length === 0 ? 'Nothing yet' : setsFields.join(', ')}
		done={chosen !== null && pickedView !== null}
		bind:open={setsOpen}
	>
		{#snippet action()}
			{#if !composing && chosenHead !== null}
				<Button
					small
					icon="pencil"
					disabled={busy}
					reason={busy ? 'Wait for the other template to finish.' : undefined}
					onclick={() => void change(chosenHead)}
				>
					{opening === chosenHead.id ? 'Opening…' : 'Change'}
				</Button>
				<Button
					small
					danger
					icon="trash-2"
					disabled={busy}
					reason={busy ? 'Wait for the other template to finish.' : undefined}
					onclick={() => void remove(chosenHead)}
				>
					{removing === chosenHead.id ? 'Removing…' : 'Remove'}
				</Button>
			{/if}
		{/snippet}

		{#if composing || chosen !== null}
			<FlowDiagram
				from={{ icon: 'layout-template', label: 'Template' }}
				to={[{ icon: 'layout-list', label: 'Your resource' }]}
				rule={setsFields.length === 0
					? null
					: `fills ${setsFields.length}`}
				empty="fills nothing yet"
				pairs={setsFields.length === 0 ? [] : [{ from: [setsName], to: setsFields }]}
				label="{setsName} fills {setsFields.length === 0 ? 'nothing yet' : setsFields.join(', ')}"
			/>
		{:else}
			<p class="tpl-none">Pick a template above.</p>
		{/if}
		{#if !composing && chosen !== null && pickedView === null && pickRefusal === null}
			<p class="tpl-none">Reading the template…</p>
		{/if}
		{#if pickRefusal !== null}
			<p class="tpl-refusal">{pickRefusal}</p>
		{/if}
		{#if !composing && chosenHead?.description}
			<p class="tpl-said">{chosenHead.description}</p>
		{/if}

		<!-- The editor stays mounted while hidden: it holds what the seller is
		     typing, including a half-typed category held only by its input. -->
		<div class="tpl-editor" hidden={!composing}>
			<h3 class="flow-label">{editing === null ? 'New template' : 'Change this template'}</h3>
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

			<div class="flow-actions">
				<Button
					tier="primary"
					disabled={saving || blocked !== null || capRefusal !== null}
					reason={blocked ?? capRefusal ?? (saving ? 'Saving now.' : undefined)}
					onclick={() => void save()}
				>
					{saving ? 'Saving…' : editing === null ? 'Save template' : 'Save changes'}
				</Button>
				<Button
					tier="quiet"
					onclick={() => {
						clear();
						composing = false;
					}}
				>
					{editing === null ? 'Clear' : 'Cancel'}
				</Button>
			</div>
		</div>
	</FlowStep>

	<FlowStep
		n={3}
		id="apply"
		title="Apply to"
		hint="Tick the resources to fill in."
		summary={applied ?? `${ticked.size} ticked`}
		done={applied !== null}
		bind:open={applyOpen}
		footer={applyFooter}
	>
		{#snippet aside()}
			<Explain title="What applying does" label="">
				<p>A template fills in the empty fields on the resources you tick. It never sets the title.</p>
				<p>
					Replace what’s there writes the template’s answers over the ones a resource already
					has. Preview first: nothing changes until you press Apply.
				</p>
			</Explain>
		{/snippet}

		{#if chosen === null}
			<p class="tpl-none">
				{composing ? 'Save the template, then tick where it goes.' : 'Pick a template first.'}
			</p>
		{:else}
			<div class="flow-choice" role="radiogroup" aria-label="Fill or replace">
				<button type="button" role="radio" aria-checked={!overwrite} onclick={() => (overwrite = false)}>
					Fill empty fields
					<span class="sub">What a resource has stays.</span>
				</button>
				<button type="button" role="radio" aria-checked={overwrite} onclick={() => (overwrite = true)}>
					Replace what’s there
					<span class="sub">The template’s answers win.</span>
				</button>
			</div>

			<div class="flow-pick tpl-ticks">
				{#if !pickLoaded}
					<p class="tpl-none">Loading your resources…</p>
				{:else if pickUnread && products.length === 0}
					<Banner tone="bad">Your resources could not be loaded.</Banner>
				{:else if products.length === 0 && pickPage === 1}
					<p class="tpl-none">Import your shop first, then apply templates here.</p>
				{:else}
					<div class="tpl-pick-head">
						<label class="tpl-pick-all">
							<input type="checkbox" checked={allTickedHere} onchange={tickPage} />
							Tick these {products.length}
						</label>
						<span class="tpl-pick-count" role="status" aria-live="polite">{ticked.size} ticked</span>
					</div>
					<div class="pick-list">
						{#each products as product (product.id)}
							<label class="pick-row">
								<input
									type="checkbox"
									checked={ticked.has(product.id)}
									onchange={(event) => tickOne(product.id, event.currentTarget.checked)}
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
						onprevious={() => void readProducts(pickCursors[pickPage - 2] ?? null, pickPage - 1)}
						onnext={() => void readProducts(pickNext, pickPage + 1)}
					/>
				{/if}
			</div>

			{#if applyPlan !== null}
				<p class="tpl-tally">
					{applyPlan.counts.will_change} to change · {applyPlan.counts.unchanged} already match ·
					{applyPlan.counts.blocked} skipped
				</p>
				<div class="flow-table-wrap">
					<table class="flow-table">
						<thead>
							<tr><th>Resource</th><th>Result</th><th>Fields</th></tr>
						</thead>
						<tbody>
							{#each applyPlan.rows as row (row.product)}
								<tr>
									<td><span class="res-name">{row.title}</span></td>
									<td class="marks">
										<span class="tpl-verdict {row.verdict}">{VERDICT_WORDS[row.verdict] ?? row.verdict}</span>
									</td>
									<td class="tpl-why">
										{#if row.reason !== null}
											{row.reason}
										{:else if row.fields.length > 0}
											{row.fields.map(fieldWordsOf).join(', ')}
										{:else}
											Already filled in.
										{/if}
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}

			{#if applied !== null}
				<p class="tpl-done">{applied}</p>
			{/if}
			{#if applyRefusal !== null}
				<p class="tpl-refusal">{applyRefusal}</p>
			{/if}
		{/if}
	</FlowStep>
</div>

{#snippet applyFooter()}
	{@render applyButtons()}
	{#if applyBlocked !== null && chosen !== null}
		<span class="tpl-foot-note">{applyBlocked}</span>
	{/if}
{/snippet}

{#snippet applyButtons()}
	<Button
		tier={primaryIsApply ? 'outline' : 'primary'}
		icon="eye"
		disabled={previewBlocked !== null}
		reason={previewBlocked ?? undefined}
		onclick={() => void previewApply()}
	>
		{previewing ? 'Previewing…' : applyPlan === null ? 'Preview' : 'Preview again'}
	</Button>
	<Button
		tier="primary"
		icon="check"
		disabled={applyBlocked !== null}
		reason={applyBlocked ?? undefined}
		onclick={() => void apply()}
	>
		{applying ? 'Applying…' : willChange > 0 ? `Apply to ${willChange}` : 'Apply'}
	</Button>
{/snippet}
