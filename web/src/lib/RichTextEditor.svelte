<script lang="ts">
	import type { Snippet } from 'svelte';
	import { ApiFailure } from '$lib/api';
	import { safeHref } from '$lib/rich-text';
	import { toast } from '$lib/toast';
	import '$lib/pages/resources/resources.css';

	// A rich-text box and its formatting bar. The box is `contenteditable`,
	// and what it holds is reduced by `sanitise` on every keystroke, so the
	// value a caller keeps is always inside its allow-list: the description's
	// eight elements, or an email's with headings and pictures.
	let {
		id,
		label,
		value,
		onchange,
		sanitise,
		placeholder,
		required = false,
		headings = false,
		uploadImage,
		extra
	}: {
		id: string;
		/** The box's accessible name. */
		label: string;
		value: string;
		onchange: (html: string) => void;
		/** The allow-list, applied to what is drawn, typed and pasted. */
		sanitise: (html: string) => string;
		placeholder: string;
		required?: boolean;
		/** Adds heading and paragraph buttons. */
		headings?: boolean;
		/** Adds a Picture button; answers the stored picture's address. */
		uploadImage?: (file: File) => Promise<string>;
		/** More buttons at the end of the bar. */
		extra?: Snippet;
	} = $props();

	let editor = $state<HTMLDivElement | null>(null);
	let picker = $state<HTMLInputElement | null>(null);
	let uploading = $state(false);
	/** What this editor last handed out. A value that differs arrived from
	 *  elsewhere — the stored draft, a template, a format switch — and is
	 *  drawn into the box; one that matches is the teacher's own typing, and
	 *  redrawing it would throw the caret away. */
	let written: string | null = null;
	/** The element `written` was drawn into; a new one is drawn whatever. */
	let drawnInto: HTMLDivElement | null = null;
	/** Where the caret last was inside the box, so a button that takes the
	 *  focus (a file picker, a menu) still inserts there. */
	let saved: Range | null = null;

	$effect(() => {
		const html = value;
		if (editor === null || (editor === drawnInto && html === written)) {
			return;
		}
		// Drawn from the allow-listed copy, never the raw one: an imported
		// body's `<img onerror>` would run the moment it was parsed.
		editor.innerHTML = sanitise(html) || '<p><br></p>';
		written = html;
		drawnInto = editor;
	});

	$effect(() => {
		const remember = () => {
			const selection = getSelection();
			if (selection === null || selection.rangeCount === 0 || editor === null) {
				return;
			}
			const range = selection.getRangeAt(0);
			if (editor.contains(range.commonAncestorContainer)) {
				saved = range.cloneRange();
			}
		};
		document.addEventListener('selectionchange', remember);
		return () => document.removeEventListener('selectionchange', remember);
	});

	/** The box's content as the caller keeps it: the allow-listed HTML. */
	function capture() {
		if (editor === null) {
			return;
		}
		written = sanitise(editor.innerHTML);
		onchange(written);
	}

	/** Focus the box with the caret where it last was, or at the end. */
	function restore() {
		if (editor === null) {
			return;
		}
		editor.focus();
		const selection = getSelection();
		if (selection === null) {
			return;
		}
		if (saved !== null && editor.contains(saved.commonAncestorContainer)) {
			selection.removeAllRanges();
			selection.addRange(saved);
		} else if (!editor.contains(selection.anchorNode)) {
			const end = document.createRange();
			end.selectNodeContents(editor);
			end.collapse(false);
			selection.removeAllRanges();
			selection.addRange(end);
		}
	}

	function run(name: string, argument?: string) {
		restore();
		document.execCommand('styleWithCSS', false, 'false');
		document.execCommand('defaultParagraphSeparator', false, 'p');
		document.execCommand(name, false, argument);
		capture();
	}

	/** Words at the caret, as typed. */
	export function insertText(text: string) {
		run('insertText', text);
	}

	/** Markup at the caret, reduced by the allow-list once it lands. */
	export function insertHtml(html: string) {
		run('insertHTML', html);
	}

	function escapeAttribute(text: string): string {
		return text
			.replace(/&/g, '&amp;')
			.replace(/</g, '&lt;')
			.replace(/>/g, '&gt;')
			.replace(/"/g, '&quot;');
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
		restore();
		const selection = getSelection();
		if (selection === null || selection.isCollapsed) {
			const shown = href.replace(/&/g, '&amp;').replace(/</g, '&lt;');
			run('insertHTML', `<a href="${escapeAttribute(href)}">${shown}</a>`);
		} else {
			run('createLink', href);
		}
	}

	async function picture(event: Event & { currentTarget: HTMLInputElement }) {
		const input = event.currentTarget;
		const file = input.files?.[0];
		if (file === undefined || uploadImage === undefined) {
			return;
		}
		uploading = true;
		try {
			const url = await uploadImage(file);
			insertHtml(`<img src="${escapeAttribute(url)}" alt="">`);
		} catch (failure) {
			toast(
				'error',
				failure instanceof ApiFailure ? failure.message : 'The picture was not uploaded. Try again.'
			);
		} finally {
			uploading = false;
			// So choosing the same file again fires another change.
			input.value = '';
		}
	}

	/** A paste arrives already reduced, so a document's fonts and colours
	 *  never enter the box to be stripped later. */
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
				? sanitise(html)
				: escaped
						.split(/\n{2,}/)
						.map((block) => `<p>${block.replace(/\n/g, '<br>')}</p>`)
						.join('');
		document.execCommand('insertHTML', false, clean);
		capture();
	}
</script>

<div class="rte">
	<div class="res-md rte-bar" role="group" aria-label="Formatting">
		{#if headings}
			<button
				type="button"
				class="res-md-b rte-b"
				onmousedown={(e) => e.preventDefault()}
				onclick={() => run('formatBlock', '<h2>')}
			>
				H2<span class="sr-only"> Heading</span>
			</button>
			<button
				type="button"
				class="res-md-b rte-b"
				onmousedown={(e) => e.preventDefault()}
				onclick={() => run('formatBlock', '<h3>')}
			>
				H3<span class="sr-only"> Smaller heading</span>
			</button>
			<button
				type="button"
				class="res-md-b rte-b"
				onmousedown={(e) => e.preventDefault()}
				onclick={() => run('formatBlock', '<p>')}
			>
				¶<span class="sr-only">Paragraph</span>
			</button>
		{/if}
		<button
			type="button"
			class="res-md-b rte-b"
			onmousedown={(e) => e.preventDefault()}
			onclick={() => run('bold')}
		>
			<b>B</b><span class="sr-only">Bold</span>
		</button>
		<button
			type="button"
			class="res-md-b rte-b"
			onmousedown={(e) => e.preventDefault()}
			onclick={() => run('italic')}
		>
			<i>I</i><span class="sr-only">Italic</span>
		</button>
		<button
			type="button"
			class="res-md-b rte-b"
			onmousedown={(e) => e.preventDefault()}
			onclick={() => run('insertUnorderedList')}
		>
			•<span class="sr-only">Bulleted list</span>
		</button>
		<button
			type="button"
			class="res-md-b rte-b"
			onmousedown={(e) => e.preventDefault()}
			onclick={() => run('insertOrderedList')}
		>
			1.<span class="sr-only">Numbered list</span>
		</button>
		<button
			type="button"
			class="res-md-b rte-b"
			onmousedown={(e) => e.preventDefault()}
			onclick={link}
		>
			<u>Link</u><span class="sr-only"> to a web address</span>
		</button>
		{#if uploadImage !== undefined}
			<button
				type="button"
				class="res-md-b rte-b"
				disabled={uploading}
				aria-busy={uploading}
				onmousedown={(e) => e.preventDefault()}
				onclick={() => picker?.click()}
			>
				{uploading ? 'Uploading…' : 'Picture'}
			</button>
			<input
				bind:this={picker}
				hidden
				type="file"
				accept="image/png,image/jpeg,image/gif,image/webp"
				onchange={picture}
			/>
		{/if}
		{@render extra?.()}
	</div>
	<div
		{id}
		class="res-rich"
		bind:this={editor}
		contenteditable="true"
		role="textbox"
		tabindex="0"
		aria-multiline="true"
		aria-label={label}
		aria-required={required ? 'true' : undefined}
		data-placeholder={placeholder}
		class:empty={value === ''}
		oninput={capture}
		onpaste={paste}
	></div>
</div>

<style>
	/* The bar's own look, wherever the box is drawn: the resource form's
	   sheet scopes `.res-md` under its page, and the same values here keep
	   the two identical there. */
	.rte-bar {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		margin-bottom: 6px;
	}

	.rte-bar :global(.rte-b) {
		border: 1px solid var(--line);
		background: var(--surface);
		border-radius: var(--r-field);
		min-width: 30px;
		height: var(--control-h-sm);
		font: inherit;
		font-size: 13px;
		color: var(--primary);
		cursor: pointer;
	}

	.rte-bar :global(.rte-b:hover) {
		background: var(--hover);
	}

	.rte-bar :global(.rte-b:disabled) {
		cursor: progress;
		color: var(--muted);
	}

	.rte :global(.res-rich h2) {
		margin: 0.4em 0 0.4em;
		font-size: 1.3em;
		line-height: 1.3;
	}

	.rte :global(.res-rich h3) {
		margin: 0.4em 0 0.4em;
		font-size: 1.1em;
		line-height: 1.3;
	}

	.rte :global(.res-rich img) {
		display: block;
		max-width: 100%;
		height: auto;
		border-radius: var(--r-field);
	}
</style>
