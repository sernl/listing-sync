<script lang="ts">
	import type { FormVocabularyView } from '$lib/api';
	import { atCap, counterOf, gradeColumns, togglePick } from '$lib/tpt-form';

	let {
		vocabulary,
		chosen,
		onChange
	}: {
		vocabulary: FormVocabularyView;
		chosen: readonly string[];
		onChange: (grades: string[]) => void;
	} = $props();

	const cap = $derived(vocabulary.caps.grades ?? null);
	const columns = $derived(gradeColumns(vocabulary));
	const counter = $derived(counterOf(chosen.length, cap));
	const full = $derived(atCap(chosen.length, cap));
	const withheld = $derived(vocabulary.grades.filter((grade) => !grade.seller_writable));
</script>

<!-- The seventeen checkboxes TPT renders, in the four-column arrangement its
     own grid reads down. The arrangement is the segregation — primary grades,
     middle grades, high-school grades, then the three non-grade bands — so it
     is reproduced rather than the DOM's row-major order. The column sizes come
     from the server, so a re-polled vocabulary re-shapes the grid instead of
     overflowing one column. -->
<div class="field">
	<span id="grades-label">
		Grade Level<span class="req" aria-hidden="true">*</span>
		<span class="sr-only">Required</span>
		<span class="gg-count" class:over={counter.over}>{counter.text}</span>
	</span>
	<span class="hint">
		Select up to {cap ?? 'four'} grades. If your product works for all grades, select "Not Grade
		Specific."
	</span>
	<div class="gg-grid" role="group" aria-labelledby="grades-label">
		{#each columns as column, index (index)}
			<div class="gg-col">
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
						<span>{grade.label}</span>
					</label>
				{/each}
			</div>
		{/each}
	</div>
	{#if full}
		<span class="hint">
			That is the limit TPT's own form states. Clear one to choose another.
		</span>
	{/if}
	{#if withheld.length > 0}
		<p class="gg-foot">
			{withheld.map((grade) => grade.label).join(', ')}
			{withheld.length === 1 ? 'is a band' : 'are bands'} buyers filter by rather than a grade a seller
			sets, so TPT offers no control for {withheld.length === 1 ? 'it' : 'them'} and neither do we.
		</p>
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
		color: var(--bad);
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

	.gg-foot {
		margin: 8px 0 0;
		font-size: 12px;
		color: var(--faint);
	}
</style>
