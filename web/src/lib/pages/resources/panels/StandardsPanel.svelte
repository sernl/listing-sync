<script lang="ts">
	import type { FormVocabularyView } from '$lib/api';
	import FormSection from '$lib/FormSection.svelte';
	import StandardsPicker from '$lib/StandardsPicker.svelte';
	import { STANDARDS_HELP, type Refusal, type TptDraft } from '$lib/tpt-form';

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

<FormSection
	group="education_standards"
	icon="shield-check"
	help={(form?.standards_frameworks.length ?? 0) > 0 ? STANDARDS_HELP : undefined}
	{refusals}
>
	{#if form}
		<StandardsPicker
			frameworks={form.standards_frameworks}
			chosen={draft.standards}
			onChange={(standards) => set('standards', standards)}
		/>
	{:else}
		<p class="res-note">The standards frameworks are still being read.</p>
	{/if}
</FormSection>
