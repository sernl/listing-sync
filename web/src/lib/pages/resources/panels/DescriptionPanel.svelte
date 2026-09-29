<script lang="ts">
	import type { FormVocabularyView } from '$lib/api';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import FormSection from '$lib/FormSection.svelte';
	import Note from '$lib/Note.svelte';
	import RichTextEditor from '$lib/RichTextEditor.svelte';
	import {
		descriptionLength,
		htmlLoss,
		htmlToMarkdown,
		markdownToHtml,
		sanitiseHtml
	} from '$lib/rich-text';
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
		choosesFormat = true,
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
		/** Whether the seller picks rich text or Markdown here. Off in the
		 *  Template Manager: a template stores its description without a
		 *  format, as Markdown, and the resource it fills renders it into
		 *  whichever format that resource is written in. */
		choosesFormat?: boolean;
		set: <K extends keyof TptDraft>(field: K, value: TptDraft[K]) => void;
	} = $props();

	let box = $state<HTMLTextAreaElement | null>(null);

	const rich = $derived(draft.bodyFormat === 'Html');
	/** What an HTML body holds that the editor cannot show: an imported
	 *  listing's tables or pictures. Kept until the teacher edits here. */
	const unkept = $derived(rich ? htmlLoss(draft.description) : []);

	/** One Markdown toolbar button: Markdown written around what the teacher
	 *  selected, and the caret put back where they would type next. */
	function format(kind: MarkKind) {
		if (box === null) {
			return;
		}
		const typed = box;
		const marked = markUp(typed.value, typed.selectionStart, typed.selectionEnd, kind);
		set('description', marked.text);
		// After the render that carries the new value, or the browser would
		// restore the caret into the old one.
		queueMicrotask(() => {
			typed.focus();
			typed.setSelectionRange(marked.start, marked.end);
		});
	}

	/** Switch the description's format, converting what is written. Asks first
	 *  where the other format cannot hold something. */
	function choose(target: 'Html' | 'Markdown', event: Event & { currentTarget: HTMLInputElement }) {
		if (target === draft.bodyFormat) {
			return;
		}
		const converted =
			target === 'Html'
				? markdownToHtml(draft.description)
				: { html: htmlToMarkdown(draft.description), lost: htmlLoss(draft.description) };
		if (
			converted.lost.length > 0 &&
			!confirm(
				`${target === 'Html' ? 'Rich text' : 'Markdown'} can’t keep the ${wordsOf(converted.lost)} in this description. Switch anyway?`
			)
		) {
			// The browser has already moved the dot; put it back.
			const group = event.currentTarget.closest('[role="radiogroup"]');
			const kept = group?.querySelector<HTMLInputElement>(`input[value="${draft.bodyFormat}"]`);
			if (kept) kept.checked = true;
			return;
		}
		set('bodyFormat', target);
		set('description', converted.html);
	}

	function wordsOf(parts: readonly string[]): string {
		return parts.length < 2
			? parts.join('')
			: `${parts.slice(0, -1).join(', ')} and ${parts.at(-1)}`;
	}

	const count = $derived(descriptionLength(draft.description, draft.bodyFormat));
</script>

<FormSection group="description" icon="book-open" help={GROUP_HELP.description} {refusals}>
	{#if choosesFormat}
		<div class="res-choices res-format" role="radiogroup" aria-label="Write the description in">
			<label>
				<input
					type="radio"
					name="description-format"
					value="Html"
					checked={rich}
					onchange={(event) => choose('Html', event)}
				/>
				Rich text
			</label>
			<label>
				<input
					type="radio"
					name="description-format"
					value="Markdown"
					checked={!rich}
					onchange={(event) => choose('Markdown', event)}
				/>
				Markdown
			</label>
			<Explain title="Rich text or Markdown" label="Which?">
				<p>
					Rich text looks as it will on TPT: bold, italics, lists and links, and nothing you can’t
					see.
				</p>
				<p>
					Markdown is plain text with marks in it: <code>**bold**</code>, <code>*italic*</code>, and
					a <code>-</code> or <code>1.</code> at the start of a line for a list. We turn the marks into
					formatting for each marketplace.
				</p>
				<p>
					Either way, Tes gets plain text: paragraphs, and lists with a dash or a number on each
					line. Switching converts what you have written.
				</p>
			</Explain>
		</div>
	{/if}
	<Field label="Description" id="draft-description" required={!optional}>
		{#if rich}
			<RichTextEditor
				id="draft-description"
				label="Description"
				value={draft.description}
				onchange={(html) => set('description', html)}
				sanitise={sanitiseHtml}
				placeholder="Describe your product and how it can be helpful to another educator"
				required={!optional}
			/>
		{:else}
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
		{/if}
		{#if form}
			<span class="res-count" class:over={count > form.limits.description_max_length}>
				{count} of {form.limits.description_max_length} characters
			</span>
		{/if}
	</Field>
	{#if unkept.length > 0}
		<Note icon="triangle-alert">
			This description has {wordsOf(unkept)} the editor can’t show; they stay until you edit it here.
		</Note>
	{/if}
	<Note>Use the buttons to format.</Note>
</FormSection>
