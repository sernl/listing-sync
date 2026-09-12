<script lang="ts">
	import { ApiFailure } from '$lib/api';
	import {
		templates,
		type TemplateApplyPlanView,
		type TemplateHead
	} from '$lib/pages/templates/api';
	import {
		VERDICT_WORDS,
		fieldWordsOf,
		scopeLabel
	} from '$lib/pages/templates/resource-template';

	// Applying a template over a selection is a plan and then a submit, as a
	// migration is and for the same reason: `patch_product` refuses a resource
	// whose live listing we cannot edit, so a seller ticking forty is told
	// which ones will change, which are already as the template says and which
	// are refused — before any of it is written. The submit re-runs the plan
	// rather than trusting this screen's copy of it.
	let {
		open,
		products,
		onClose,
		onDone
	}: {
		open: boolean;
		/** The resources the board has selected, by identity. */
		products: string[];
		onClose: () => void;
		/** Fired after a submit that wrote something, so the board can re-read
		 *  the rows it has just changed. */
		onDone: () => void;
	} = $props();

	let element = $state<HTMLDialogElement | null>(null);
	let heads = $state<TemplateHead[]>([]);
	let headsUnread = $state(false);
	let chosen = $state('');
	let overwrite = $state(false);
	let plan = $state<TemplateApplyPlanView | null>(null);
	let previewing = $state(false);
	let applying = $state(false);
	let refusal = $state<string | null>(null);
	let done = $state<string | null>(null);

	/** Minted once per plan the seller confirms, and held across a retry: a
	 *  second press after a dropped answer replays the same apply rather than
	 *  writing the catalogue twice. Cleared when the plan is. */
	let key: string | null = null;

	$effect(() => {
		if (open && element !== null && !element.open) {
			element.showModal();
		} else if (!open) {
			element?.close();
		}
	});

	// Read once the dialog is actually opened rather than at mount: this sits
	// on the resource board, where most sessions never press the verb.
	$effect(() => {
		if (!open || heads.length > 0 || headsUnread) {
			return;
		}
		void templates
			.list()
			.then((listed) => (heads = listed))
			.catch(() => (headsUnread = true));
	});

	/** Any change to what the apply is over retires the plan it produced. A
	 *  table drawn against another template or the other overwrite answer would
	 *  be a preview of something the Apply button no longer does. */
	function retire() {
		plan = null;
		key = null;
		done = null;
		refusal = null;
	}

	const willChange = $derived(plan?.counts.will_change ?? 0);
	const scopedTo = $derived(heads.find((head) => head.id === chosen)?.scope ?? null);

	function body() {
		return { selection: { products }, overwrite };
	}

	async function preview() {
		if (chosen === '') {
			return;
		}
		previewing = true;
		refusal = null;
		done = null;
		try {
			plan = await templates.applyPlan(chosen, body());
			key = null;
		} catch (failure) {
			plan = null;
			refusal = sentence(failure, 'That preview could not be read, so nothing was changed.');
		} finally {
			previewing = false;
		}
	}

	async function apply() {
		if (chosen === '' || plan === null || willChange === 0) {
			return;
		}
		applying = true;
		refusal = null;
		try {
			key ??= crypto.randomUUID();
			const ack = await templates.apply(chosen, body(), key);
			done = doneLine(ack.changed, ack.unchanged, ack.blocked);
			plan = null;
			key = null;
			onDone();
		} catch (failure) {
			refusal = sentence(failure, 'The apply did not finish. Nothing was written twice; try again.');
		} finally {
			applying = false;
		}
	}

	/** What the submit did, in the three figures it answers with. Every one is
	 *  stated, including the zeroes: "changed 12" alone leaves a seller who
	 *  ticked fifteen wondering about three. */
	function doneLine(changed: number, unchanged: number, blocked: number): string {
		const parts = [
			`${changed} ${changed === 1 ? 'resource' : 'resources'} changed`,
			`${unchanged} left as ${unchanged === 1 ? 'it was' : 'they were'}`
		];
		if (blocked > 0) {
			parts.push(`${blocked} refused`);
		}
		return `${parts.join(', ')}.`;
	}

	function sentence(failure: unknown, fallback: string): string {
		return failure instanceof ApiFailure ? failure.message : fallback;
	}
</script>

<dialog bind:this={element} aria-labelledby="apply-template-title" onclose={onClose}>
	<div class="dialog-body">
		<h2 id="apply-template-title">
			Apply a template to {products.length}
			{products.length === 1 ? 'resource' : 'resources'}
		</h2>
		<p>
			A template fills the fields these resources have left empty. The title is never one of
			them: a template names none, and two resources sharing a title is not something we would
			write for you.
		</p>

		{#if headsUnread}
			<p class="refusal">Your templates could not be read, so there is nothing to apply.</p>
		{:else}
			<label class="apply-pick">
				<span class="t">Template</span>
				<select
					bind:value={chosen}
					disabled={applying}
					onchange={retire}
				>
					<option value="">Choose a template</option>
					{#each heads as head (head.id)}
						<option value={head.id}>{head.name} — {scopeLabel(head.scope)}</option>
					{/each}
				</select>
			</label>
			{#if scopedTo !== null}
				<p class="foot-note">
					This template is written for {scopeLabel(scopedTo)}. Its own questions are applied to
					every resource you have ticked, whether or not that resource is listed there yet.
				</p>
			{/if}

			<label class="choice">
				<input
					type="checkbox"
					checked={overwrite}
					disabled={applying}
					onchange={(event) => {
						overwrite = event.currentTarget.checked;
						retire();
					}}
				/>
				<span class="t">Overwrite what these resources already say</span>
			</label>
			<p class="foot-note">
				Off, a template fills only the fields a resource has left empty. On, every field the
				template holds is written over what the resource says now — the title excepted, which
				is never written.
			</p>

			{#if plan !== null}
				<div class="apply-rows" role="table" aria-label="What the apply would do">
					{#each plan.rows as row (row.product)}
						<div class="apply-row" role="row">
							<span class="t" role="cell">{row.title}</span>
							<span class="verdict {row.verdict}" role="cell">
								{VERDICT_WORDS[row.verdict] ?? row.verdict}
							</span>
							<span class="why" role="cell">
								{#if row.reason !== null}
									{row.reason}
								{:else if row.fields.length > 0}
									{row.fields.map(fieldWordsOf).join(', ')}
								{:else}
									Every field this template holds is already answered.
								{/if}
							</span>
						</div>
					{/each}
				</div>
				<p class="foot-note">
					{plan.counts.will_change} to change, {plan.counts.unchanged} already as the template says,
					{plan.counts.blocked} refused.
				</p>
			{/if}

			{#if done !== null}
				<p class="apply-done">{done}</p>
			{/if}

			{#if refusal !== null}
				<p class="refusal">{refusal}</p>
			{/if}
		{/if}

		<div class="actions">
			<button class="btn" type="button" onclick={onClose} disabled={applying}>Close</button>
			<button
				class="btn"
				type="button"
				onclick={() => void preview()}
				disabled={chosen === '' || previewing || applying}
			>
				{previewing ? 'Reading…' : 'Preview'}
			</button>
			<button
				class="cta"
				type="button"
				onclick={() => void apply()}
				disabled={plan === null || willChange === 0 || applying}
			>
				{#if applying}
					Applying…
				{:else if plan === null}
					Apply
				{:else}
					Apply to {willChange}
				{/if}
			</button>
		</div>
	</div>
</dialog>

<style>
	.apply-pick {
		display: flex;
		flex-direction: column;
		gap: 4px;
		margin-top: 12px;
	}

	.apply-pick .t {
		font-size: 13px;
		font-weight: 600;
	}

	/* The plan, one line per resource. Bounded and scrolled rather than
	   growing the dialog past the viewport: a seller may tick a hundred, and a
	   modal taller than the window has no scrollbar of its own. */
	.apply-rows {
		margin-top: 12px;
		max-height: 40vh;
		overflow-y: auto;
		border: 1px solid var(--line);
		border-radius: 8px;
	}

	.apply-row {
		display: grid;
		grid-template-columns: minmax(0, 1fr) auto minmax(0, 1.2fr);
		align-items: baseline;
		gap: 10px;
		padding: 8px 10px;
	}

	.apply-row + .apply-row {
		border-top: 1px solid var(--line);
	}

	.apply-row .t {
		font-size: 13px;
		font-weight: 600;
		overflow-wrap: anywhere;
	}

	.apply-row .why {
		font-size: 12px;
		color: var(--muted);
		overflow-wrap: anywhere;
	}

	.verdict {
		justify-self: start;
		padding: 1px 8px;
		border-radius: 999px;
		font-size: 11px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.02em;
		white-space: nowrap;
	}

	.verdict.will_change {
		background: var(--ok-soft);
		color: var(--ok-ink);
	}

	.verdict.unchanged {
		background: var(--soon-soft);
		color: var(--soon);
	}

	.verdict.blocked {
		background: var(--bad-soft);
		color: var(--bad-ink);
	}

	.apply-done {
		margin-top: 12px;
		font-size: 13px;
		font-weight: 600;
	}

	/* A phone holds one column, so the three cells stack and the chip sits
	   under the title rather than squeezing it to two words. */
	@media (max-width: 720px) {
		.apply-row {
			grid-template-columns: 1fr;
			gap: 4px;
		}

		/* The phone sheet scrolls its whole body, so a second scroller inside
		   it is a list the seller has to find the edge of before they can
		   reach the actions under it. */
		.apply-rows {
			max-height: none;
			overflow-y: visible;
		}
	}
</style>
