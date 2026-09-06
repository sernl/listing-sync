<script lang="ts">
	import type { FormVocabularyView } from '$lib/api';
	import {
		atCap,
		counterOf,
		gradeBandLabel,
		gradeColumns,
		gradeLabel,
		togglePick,
		type GradeLabels
	} from '$lib/tpt-form';

	let {
		vocabulary,
		chosen,
		labels,
		onChange,
		onLabels
	}: {
		vocabulary: FormVocabularyView;
		chosen: readonly string[];
		/** Which words the grid is written in. One selection underneath either
		 *  way: the slug a teacher ticks is the same slug whichever system it
		 *  was shown to them in. */
		labels: GradeLabels;
		onChange: (grades: string[]) => void;
		onLabels: (labels: GradeLabels) => void;
	} = $props();

	const cap = $derived(vocabulary.caps.grades ?? null);
	const columns = $derived(gradeColumns(vocabulary));
	const counter = $derived(counterOf(chosen.length, cap));
	const full = $derived(atCap(chosen.length, cap));

	const SYSTEMS: readonly { id: GradeLabels; label: string }[] = [
		{ id: 'american', label: 'American (Grades)' },
		{ id: 'british', label: 'British (Years)' }
	];
</script>

<!-- TPT's grades, in the four-column arrangement its own grid reads down, and
     headed in whichever system the teacher works in. The column sizes and the
     band headings both come from the server, so a re-polled vocabulary
     re-shapes the grid rather than overflowing one column. -->
<div class="field">
	<span id="grades-label">
		Grade Level<span class="req">Required</span>
		<span class="gg-count" class:over={counter.over}>{counter.text}</span>
	</span>
	<span class="hint">
		Pick in either system. We translate for the other marketplaces.
	</span>
	<div class="gg-systems" role="group" aria-label="Grade wording">
		{#each SYSTEMS as system (system.id)}
			<button
				type="button"
				class="gg-system"
				aria-pressed={labels === system.id}
				onclick={() => onLabels(system.id)}
			>
				{system.label}
			</button>
		{/each}
	</div>
	<div class="gg-grid" role="group" aria-labelledby="grades-label">
		{#each columns as column, index (index)}
			{@const band = gradeBandLabel(vocabulary, index, labels)}
			<div class="gg-col">
				{#if band !== ''}<span class="gg-band">{band}</span>{/if}
				{#each column as grade (grade.slug)}
					{@const on = chosen.includes(grade.slug)}
					<label class="gg-tick" class:off={full && !on}>
						<input
							type="checkbox"
							checked={on}
							disabled={full && !on}
							onchange={(event) =>
								onChange(togglePick(chosen, grade.slug, event.currentTarget.checked, cap))}
						/>
						<span>{gradeLabel(grade, labels)}</span>
					</label>
				{/each}
			</div>
		{/each}
	</div>
	{#if full}
		<span class="hint">That is the limit. Clear one to choose another.</span>
	{/if}
</div>

<style>
	/* This grid's own rules, in the component: the wrapper, the label and the
	   hint are `Field`'s own classes so the control reads as one of the
	   catalogue's, and what is here is the arrangement `Field` has no notion
	   of. Tokens only. */
	.gg-count {
		margin-left: 8px;
		font-size: 11.5px;
		font-weight: 400;
		color: var(--muted);
		font-variant-numeric: tabular-nums;
	}

	.gg-count.over {
		color: var(--bad-ink);
		font-weight: 600;
	}

	/* The two systems as one segmented control rather than two buttons: they
	   are one question with two answers, and a pressed segment is the answer
	   standing rather than an action waiting to be taken. */
	.gg-systems {
		display: inline-flex;
		border: 1px solid var(--line);
		border-radius: var(--r-pill);
		background: var(--surface);
		padding: 2px;
		margin: 6px 0 2px;
		align-self: flex-start;
	}

	.gg-system {
		border: 0;
		background: none;
		border-radius: var(--r-pill);
		padding: 5px 14px;
		font: inherit;
		font-size: 12.5px;
		color: var(--muted);
		cursor: pointer;
	}

	.gg-system[aria-pressed='true'] {
		background: var(--primary);
		color: var(--on-fill);
		font-weight: 600;
	}

	.gg-grid {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
		gap: 6px 14px;
		margin-top: 4px;
	}

	.gg-col {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	/* The band a column stands for, in the teacher's own system. A heading
	   rather than a footnote, because the founder's table is what the toggle
	   exists to teach. */
	.gg-band {
		font-size: 11.5px;
		font-weight: 600;
		color: var(--muted);
		text-transform: uppercase;
		letter-spacing: 0.04em;
		padding: 0 6px 2px;
	}

	.gg-tick {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 5px 6px;
		border-radius: 6px;
		font-size: 13px;
		min-height: 32px;
		cursor: pointer;
	}

	.gg-tick:hover {
		background: var(--hover);
	}

	/* At the cap the rest are refused rather than hidden, so the seller can see
	   what they would have to give up to choose another. */
	.gg-tick.off {
		color: var(--soon);
		cursor: not-allowed;
	}
</style>
