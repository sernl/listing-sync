<script lang="ts">
	import type { FormVocabularyView } from '$lib/api';
	import FormSection from '$lib/FormSection.svelte';
	import { GROUP_HELP, type Refusal, type TptDraft } from '$lib/tpt-form';

	let {
		draft,
		form,
		refusals = [],
		blank,
		set
	}: {
		draft: TptDraft;
		form: FormVocabularyView | null;
		refusals?: readonly Refusal[];
		/** The words of a fourth choice meaning "no answer", or absent.
		 *
		 *  The create form has no such choice: a resource is drafted or live and
		 *  one of the two is always true of it. A template does, because a
		 *  template that answered this would decide the status of every resource
		 *  started from it, and leaving it unanswered is what "we will ask as
		 *  usual" means. */
		blank?: string;
		set: <K extends keyof TptDraft>(field: K, value: TptDraft[K]) => void;
	} = $props();
</script>

<FormSection group="product_status" icon="circle-check" help={GROUP_HELP.product_status} {refusals}>
	<div class="res-choices" role="radiogroup" aria-label="Product status">
		{#if blank !== undefined}
			<label>
				<input
					type="radio"
					name="status"
					checked={draft.status === ''}
					onchange={() => set('status', '')}
				/>
				{blank}
			</label>
		{/if}
		{#each form?.statuses ?? [] as status (status.id)}
			<label>
				<input
					type="radio"
					name="status"
					checked={draft.status === status.id}
					onchange={() => set('status', status.id)}
				/>
				{status.label}
			</label>
		{/each}
	</div>
</FormSection>
