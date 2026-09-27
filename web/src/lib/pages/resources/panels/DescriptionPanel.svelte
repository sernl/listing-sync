<script lang="ts">
	import type { FormVocabularyView } from '$lib/api';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import FormSection from '$lib/FormSection.svelte';
	import Note from '$lib/Note.svelte';
	import {
		descriptionLength,
		htmlLoss,
		htmlToMarkdown,
		markdownToHtml,
		safeHref,
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
	let editor = $state<HTMLDivElement | null>(null);
	/** What this editor last wrote into the draft. A description that differs
	 *  arrived from elsewhere — the stored product, a template, a format
	 *  switch — and is drawn into the editor; one that matches is the
	 *  teacher's own typing, and redrawing it would throw the caret away. */
	let written: string | null = null;
	/** The element `written` was drawn into. A switch away from rich text and
	 *  back mounts a new, empty one, which has to be drawn whatever it holds. */
	let drawnInto: HTMLDivElement | null = null;

	const rich = $derived(draft.bodyFormat === 'Html');
	/** What an HTML body holds that the editor cannot show: an imported
	 *  listing's tables or pictures. Kept until the teacher edits here. */
	const unkept = $derived(rich ? htmlLoss(draft.description) : []);

	$effect(() => {
		const description = draft.description;
		if (editor === null || !rich || (editor === drawnInto && description === written)) {
			return;
		}
		// Drawn from the allow-listed copy, never the raw one: an imported
		// body's `<img onerror>` would run the moment it was parsed.
		editor.innerHTML = sanitiseHtml(description) || '<p><br></p>';
		written = description;
		drawnInto = editor;
	});

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

	/** The editor's content as the draft keeps it: the allow-listed HTML. */
	function capture() {
		if (editor === null) {
			return;
		}
		written = sanitiseHtml(editor.innerHTML);
		set('description', written);
	}

	/** One rich-text toolbar button. `execCommand` is what every browser's
	 *  own editing runs on; whatever element it picks, `capture` reduces to
	 *  the allow-list. */
	function command(name: 'bold' | 'italic' | 'insertUnorderedList' | 'insertOrderedList') {
		editor?.focus();
		document.execCommand('styleWithCSS', false, 'false');
		document.execCommand('defaultParagraphSeparator', false, 'p');
		document.execCommand(name);
		capture();
	}

	function link() {
		const answer = prompt('The web address to link to', 'https://');
		if (answer === null) {
			return;
		}
		const href = safeHref(answer);
		if (href === null) {
			alert('A link needs a web address starting with https:// or http://.');
			return;
		}
		editor?.focus();
		const selection = getSelection();
		if (selection === null || selection.isCollapsed) {
			const shown = href.replace(/&/g, '&amp;').replace(/</g, '&lt;');
			const attribute = shown.replace(/"/g, '&quot;');
			document.execCommand('insertHTML', false, `<a href="${attribute}">${shown}</a>`);
		} else {
			document.execCommand('createLink', false, href);
		}
		capture();
	}

	/** A paste arrives already reduced, so a document's fonts and colours
	 *  never enter the editor to be stripped later. */
	function paste(event: ClipboardEvent) {
		const data = event.clipboardData;
		if (data === null) {
			return;
		}
		event.preventDefault();
		const html = data.getData('text/html');
		const text = data.getData('text/plain');
		const escaped = text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
		const clean =
			html !== ''
				? sanitiseHtml(html)
				: escaped
						.split(/\n{2,}/)
						.map((block) => `<p>${block.replace(/\n/g, '<br>')}</p>`)
						.join('');
		document.execCommand('insertHTML', false, clean);
		capture();
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
		return parts.length < 2 ? parts.join('') : `${parts.slice(0, -1).join(', ')} and ${parts.at(-1)}`;
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
					Markdown is plain text with marks in it: <code>**bold**</code>, <code>*italic*</code>,
					and a <code>-</code> or <code>1.</code> at the start of a line for a list. We turn the
					marks into formatting for each marketplace.
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
			<div class="res-md" role="group" aria-label="Formatting">
				<button
					type="button"
					class="res-md-b"
					onmousedown={(e) => e.preventDefault()}
					onclick={() => command('bold')}
				>
					<b>B</b><span class="sr-only">Bold</span>
				</button>
				<button
					type="button"
					class="res-md-b"
					onmousedown={(e) => e.preventDefault()}
					onclick={() => command('italic')}
				>
					<i>I</i><span class="sr-only">Italic</span>
				</button>
				<button
					type="button"
					class="res-md-b"
					onmousedown={(e) => e.preventDefault()}
					onclick={() => command('insertUnorderedList')}
				>
					•<span class="sr-only">Bulleted list</span>
				</button>
				<button
					type="button"
					class="res-md-b"
					onmousedown={(e) => e.preventDefault()}
					onclick={() => command('insertOrderedList')}
				>
					1.<span class="sr-only">Numbered list</span>
				</button>
				<button
					type="button"
					class="res-md-b"
					onmousedown={(e) => e.preventDefault()}
					onclick={link}
				>
					<u>Link</u><span class="sr-only"> to a web address</span>
				</button>
			</div>
			<div
				id="draft-description"
				class="res-rich"
				bind:this={editor}
				contenteditable="true"
				role="textbox"
				tabindex="0"
				aria-multiline="true"
				aria-label="Description"
				aria-required={optional ? undefined : 'true'}
				data-placeholder="Describe your product and how it can be helpful to another educator"
				class:empty={draft.description === ''}
				oninput={capture}
				onpaste={paste}
			></div>
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

<style>
	/* The rich-text box, drawn as the Markdown textarea is so the two modes
	   are one field that changes its manners, not two fields. */
	.res-rich {
		position: relative;
		border: 1px solid var(--line);
		background: var(--card);
		border-radius: var(--r-field);
		padding: 8px 11px;
		min-height: 150px;
		max-height: 60vh;
		overflow-y: auto;
		line-height: 1.5;
		color: var(--ink);
		overflow-wrap: anywhere;
	}

	.res-rich:focus-visible {
		outline: none;
		border-color: var(--primary);
	}

	.res-rich.empty::before {
		content: attr(data-placeholder);
		color: var(--muted);
		pointer-events: none;
		position: absolute;
	}

	.res-rich :global(p) {
		margin: 0 0 0.6em;
	}

	.res-rich :global(ul),
	.res-rich :global(ol) {
		margin: 0 0 0.6em;
		padding-left: 1.4em;
	}

	.res-rich :global(a) {
		color: var(--primary);
		text-decoration: underline;
	}

	.res-format {
		margin-bottom: 10px;
	}
</style>
