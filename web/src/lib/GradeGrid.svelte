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
		Grade Level <span class="req">Required</span>
		<span class="counter {counter.over ? 'over' : ''}">{counter.text}</span>
	</span>
	<span class="hint">
		Select up to {cap ?? 'four'} grades. If your product works for all grades, select "Not Grade
		Specific."
	</span>
	<div class="grade-grid" role="group" aria-labelledby="grades-label">
		{#each columns as column, index (index)}
			<div class="grade-col">
				{#each column as grade (grade.slug)}
					{@const on = chosen.includes(grade.slug)}
					<label class="tick {full && !on ? 'off' : ''}">
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
		<p class="foot-note">
			{withheld.map((grade) => grade.label).join(', ')}
			{withheld.length === 1 ? 'is a band' : 'are bands'} buyers filter by rather than a grade a seller
			sets, so TPT offers no control for {withheld.length === 1 ? 'it' : 'them'} and neither do we.
		</p>
	{/if}
</div>
