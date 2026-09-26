<script lang="ts">
	import type { FormVocabularyView } from '$lib/api';
	import FacetPicker from '$lib/FacetPicker.svelte';
	import Field from '$lib/Field.svelte';
	import FormSection from '$lib/FormSection.svelte';
	import GradeGrid from '$lib/GradeGrid.svelte';
	import {
		GROUP_HELP,
		capOf,
		type GradeLabels,
		type Refusal,
		type TptDraft
	} from '$lib/tpt-form';

	let {
		draft,
		form,
		refusals = [],
		gradeLabels,
		/** Whether this band asks rather than requires. A template's fields are
		 *  starting points, so the Required words are off there. */
		optional = false,
		/** The category typed but not yet entered.
		 *
		 *  Held here by default, which is what the create form wants: the form
		 *  is unmounted when it is done with, so the half-typed word goes with
		 *  it. The Template Manager keeps this band mounted for a whole visit
		 *  and binds this instead, because there it is the editor that decides
		 *  when a template is finished with and so when this word is too. */
		pending = $bindable(''),
		onLabels,
		set
	}: {
		draft: TptDraft;
		form: FormVocabularyView | null;
		refusals?: readonly Refusal[];
		gradeLabels: GradeLabels;
		optional?: boolean;
		pending?: string;
		onLabels: (chosen: GradeLabels) => void;
		set: <K extends keyof TptDraft>(field: K, value: TptDraft[K]) => void;
	} = $props();
</script>

<FormSection group="categories" icon="layout-template" help={GROUP_HELP.categories} {refusals}>
	{#if form}
		<GradeGrid
			vocabulary={form}
			chosen={draft.grades}
			labels={gradeLabels}
			required={!optional}
			onChange={(grades) => set('grades', grades)}
			{onLabels}
		/>

		<FacetPicker
			label="Subject Area"
			required={!optional}
			placeholder="Select up to three subject areas"
			facets={form.subject_areas}
			chosen={draft.subjectAreas}
			cap={capOf(form.caps, 'subjectAreas')}
			onChange={(values) => set('subjectAreas', values)}
		/>

		<FacetPicker
			label="Tag (Theme, Audience, Language)"
			required={!optional}
			placeholder="Select up to six tags"
			facets={form.tags}
			chosen={draft.tags}
			cap={capOf(form.caps, 'tags')}
			onChange={(values) => set('tags', values)}
		/>

		<FacetPicker
			label="Format"
			placeholder="Select up to three formats"
			facets={form.formats}
			chosen={draft.formats}
			cap={capOf(form.caps, 'formats')}
			onChange={(values) => set('formats', values)}
		/>

		<Field
			label="Custom Category"
			id="draft-custom-category"
			hint="Your own word for grouping resources."
		>
			<input
				id="draft-custom-category"
				type="text"
				placeholder="Add a category and press Enter"
				bind:value={pending}
				onkeydown={(event) => {
					if (event.key === 'Enter') {
						event.preventDefault();
						const typed = pending.trim();
						if (typed.length > 0 && !draft.customCategories.includes(typed)) {
							set('customCategories', [...draft.customCategories, typed]);
						}
						pending = '';
					}
				}}
			/>
			{#if draft.customCategories.length > 0}
				<div class="res-chips" role="list">
					{#each draft.customCategories as category (category)}
						<span class="res-chip" role="listitem">
							{category}
							<button
								type="button"
								aria-label="Remove {category}"
								onclick={() =>
									set(
										'customCategories',
										draft.customCategories.filter((held) => held !== category)
									)}
							>
								×
							</button>
						</span>
					{/each}
				</div>
			{/if}
		</Field>
	{:else}
		<p class="res-note">Loading the categories…</p>
	{/if}
</FormSection>
