<script lang="ts">
	import type { FormVocabularyView } from '$lib/api';
	import Field from '$lib/Field.svelte';
	import FormSection from '$lib/FormSection.svelte';
	import Note from '$lib/Note.svelte';
	import { GROUP_HELP, markUp, type MarkKind, type Refusal, type TptDraft } from '$lib/tpt-form';

	// The description band, with its own formatting bar. A component rather
	// than markup inside the create form because the Template Manager holds
	// the same band over the same draft, and two copies of a toolbar are two
	// toolbars to keep in step.
	let {
		draft,
		form,
		refusals = [],
		optional = false,
		set
	}: {
		draft: TptDraft;
		form: FormVocabularyView | null;
		refusals?: readonly Refusal[];
		/** Whether this band asks rather than requires.
		 *
		 *  A resource has to answer these; a template does not, and says so —
		 *  "leave the rest empty and we will ask as usual". So the Required
		 *  words and the `aria-required` that announces them are off there
		 *  rather than telling a seller a template field is mandatory. */
		optional?: boolean;
		set: <K extends keyof TptDraft>(field: K, value: TptDraft[K]) => void;
	} = $props();

	let box = $state<HTMLTextAreaElement | null>(null);

	/** One toolbar button: Markdown written around what the teacher selected,
	 *  and the caret put back where they would type next. */
	function format(kind: MarkKind) {
		if (box === null) {
			return;
		}
		const written = box;
		const marked = markUp(written.value, written.selectionStart, written.selectionEnd, kind);
		set('description', marked.text);
		// After the render that carries the new value, or the browser would
		// restore the caret into the old one.
		queueMicrotask(() => {
			written.focus();
			written.setSelectionRange(marked.start, marked.end);
		});
	}
</script>

<FormSection group="description" icon="book-open" help={GROUP_HELP.description} {refusals}>
	<Field label="Description" id="draft-description" required={!optional}>
		<div class="res-md" role="group" aria-label="Formatting">
			<button type="button" class="res-md-b" onclick={() => format('bold')}>
				<b>B</b><span class="sr-only">Bold</span>
			</button>
			<button type="button" class="res-md-b" onclick={() => format('italic')}>
				<i>I</i><span class="sr-only">Italic</span>
			</button>
			<button type="button" class="res-md-b" onclick={() => format('bullets')}>
				•<span class="sr-only">Bulleted list</span>
			</button>
			<button type="button" class="res-md-b" onclick={() => format('numbers')}>
				1.<span class="sr-only">Numbered list</span>
			</button>
		</div>
		<textarea
			id="draft-description"
			bind:this={box}
			aria-required={optional ? undefined : 'true'}
			placeholder="Describe your product and how it can be helpful to another educator"
			value={draft.description}
			oninput={(event) => set('description', event.currentTarget.value)}
		></textarea>
		{#if form}
			<span class="res-count" class:over={draft.description.length > form.limits.description_max_length}>
				{draft.description.length} of {form.limits.description_max_length} characters
			</span>
		{/if}
	</Field>
	<Note>Use the buttons to format.</Note>
</FormSection>
