<script lang="ts">
	import type { FormVocabularyView } from '$lib/api';
	import Field from '$lib/Field.svelte';
	import FormSection from '$lib/FormSection.svelte';
	import { GROUP_HELP, type Refusal, type TptDraft } from '$lib/tpt-form';

	let {
		draft,
		form,
		refusals = [],
		set
	}: {
		draft: TptDraft;
		form: FormVocabularyView | null;
		refusals?: readonly Refusal[];
		set: <K extends keyof TptDraft>(field: K, value: TptDraft[K]) => void;
	} = $props();
</script>

<FormSection group="details" icon="sliders-horizontal" help={GROUP_HELP.details} {refusals}>
	<div class="res-row">
		<Field label="Teaching Duration" id="draft-teaching-duration">
			<select
				id="draft-teaching-duration"
				value={draft.teachingDuration ?? ''}
				onchange={(event) =>
					set('teachingDuration', event.currentTarget.value === '' ? null : event.currentTarget.value)}
			>
				<option value="">N/A</option>
				{#each form?.teaching_durations ?? [] as duration (duration.id)}
					<option value={duration.id}>{duration.label}</option>
				{/each}
			</select>
		</Field>
		<Field label="Number of Pages or Slides" id="draft-pages">
			<input
				id="draft-pages"
				type="text"
				inputmode="numeric"
				placeholder="Total pages or slides"
				value={draft.pagesOrSlides}
				oninput={(event) => set('pagesOrSlides', event.currentTarget.value)}
			/>
		</Field>
		<Field label="Answer Key" id="draft-answer-key">
			<select
				id="draft-answer-key"
				value={draft.answerKey ?? ''}
				onchange={(event) =>
					set('answerKey', event.currentTarget.value === '' ? null : event.currentTarget.value)}
			>
				<option value="">N/A</option>
				{#each form?.answer_keys ?? [] as key (key.id)}
					<option value={key.id}>{key.label}</option>
				{/each}
			</select>
		</Field>
	</div>
</FormSection>
