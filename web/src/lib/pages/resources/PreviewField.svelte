<script lang="ts">
	// The free sample buyers look at before they buy: its own drop zone, and a
	// way to cut one out of the file the teacher already uploaded.
	//
	// Separate from the file zone because the founder found the two conflated:
	// a document with "Preview" in its name went in as the product file and
	// nothing on the page said otherwise. The maker is the second half of the
	// same answer — most teachers have no preview to hand, and the pages they
	// would show are already inside the PDF they uploaded.
	import { ApiFailure, api, type FileHandle, type FormLimits } from '$lib/api';
	import { quotaSentence } from '$lib/authoring';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import FileViewer from '$lib/FileViewer.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { desktopInvoker, libraryEntries, type LibraryEntry } from '$lib/desktop';
	import { sizeWords } from '$lib/tpt-form';
	import FileRename from './FileRename.svelte';
	import PreviewMaker from './PreviewMaker.svelte';
	import { buildPreview } from './preview-pdf';
	import { forgetRecipe, readRecipe, writeRecipe, type PreviewRecipe } from './preview-recipe';
	import { type ByteSource, sourceOfFile, sourceOfKept } from './file-viewer';

	let {
		previews,
		limits,
		source,
		sellerName,
		uploadLabel = 'Make preview',
		onAdd,
		onReplace,
		onRename,
		onRemove
	}: {
		/** The previews this listing already carries. */
		previews: readonly FileHandle[];
		limits: FormLimits | null;
		/** The PDF a preview can be cut out of: the one chosen in this session,
		 *  or one the Teachouse app keeps on this machine. Either way the bytes
		 *  are on this machine; a file uploaded on some earlier visit from
		 *  another one is not. */
		source: File | ByteSource | null;
		/** What the confirming button says. On the kept-file path pressing it
		 *  is what sends the derived file to Teachouse, so it says so. */
		uploadLabel?: string;
		/** Whose name is written across the pages, where the teacher asks for
		 *  it. Their own shop name, never one composed here. */
		sellerName: string;
		onAdd: (handle: FileHandle) => void;
		/** Swap the preview named by `hash` for `handle`, in one step. */
		onReplace: (hash: string, handle: FileHandle) => void;
		onRename: (hash: string, name: string) => void;
		onRemove: (hash: string) => void;
	} = $props();

	const base = $props.id();
	const inputId = `${base}-preview`;

	let over = $state(false);
	let sending = $state(false);
	let refusal = $state<string | null>(null);
	let making = $state(false);
	/** The preview a Change is remaking, or `null` for a new one. */
	let changing = $state<string | null>(null);
	let renaming = $state<string | null>(null);
	let viewing = $state<FileHandle | null>(null);

	const cap = $derived(limits?.preview.max_size_bytes ?? null);
	const pdfSource = $derived(
		source === null ? null : source instanceof File ? sourceOfFile(source) : source
	);

	// The bytes of every preview sent from this page, by digest, so View works
	// on a draft that has no product to read it back through.
	let sent = $state<Map<string, File>>(new Map());
	// What the Teachouse app keeps on this machine; empty in a browser.
	const invoke = desktopInvoker();
	let kept = $state<Map<string, LibraryEntry>>(new Map());
	$effect(() => {
		void (async () => {
			const answer = await libraryEntries(invoke);
			if (answer.kind === 'ok') {
				kept = new Map(answer.value.map((entry) => [entry.hash, entry]));
			}
		})();
	});

	/** Where a preview's bytes can be read on this machine, or `null`: sent
	 *  from this page, kept by the app, or remade from the remembered choices
	 *  and the PDF that is here. The server hands no file's bytes back. */
	function viewSource(preview: FileHandle): ByteSource | null {
		const name = preview.name ?? 'Preview';
		const local = sent.get(preview.hash);
		if (local !== undefined) {
			return sourceOfFile(local);
		}
		if (kept.has(preview.hash)) {
			return sourceOfKept(invoke, name, preview.hash);
		}
		const recipe = readRecipe(preview.hash);
		const from = pdfSource;
		if (recipe === null || from === null) {
			return null;
		}
		return {
			name,
			bytes: async () => {
				const mark = recipe.watermark && recipe.watermarkText.trim() !== '' ? recipe.watermarkText : null;
				const made = await buildPreview(await from.bytes(), recipe.pages, mark, recipe.marked);
				return made.slice().buffer;
			}
		};
	}

	function contentTypeOf(preview: FileHandle): string {
		const known = sent.get(preview.hash)?.type || kept.get(preview.hash)?.content_type;
		if (known) {
			return known;
		}
		return preview.kind === 'pdf' ? 'application/pdf' : 'application/octet-stream';
	}

	/** The upload, with the one refusal this browser can make before a byte is
	 *  sent: the server refuses an oversized preview too, and reading that
	 *  answer back after a 30 MB upload is the worse way to learn it. */
	async function store(file: File, recipe: PreviewRecipe | null, replacing: string | null) {
		if (cap !== null && file.size > cap) {
			refusal = `That preview is ${sizeWords(file.size)}. The limit is ${sizeWords(cap)}.`;
			return;
		}
		sending = true;
		refusal = null;
		try {
			const landed = await api.upload(file, 'keep_whole');
			const uploaded = landed.payload[0];
			if (uploaded === undefined) {
				refusal = 'That file didn’t upload properly. Try again.';
				return;
			}
			// The name is the client's word: the upload's raw body carries none.
			const handle = { ...uploaded, name: file.name };
			sent = new Map(sent).set(handle.hash, file);
			if (recipe !== null) {
				writeRecipe(handle.hash, recipe);
			}
			if (replacing === null) {
				onAdd(handle);
			} else {
				onReplace(replacing, handle);
				forgetRecipe(replacing);
			}
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure
					? (quotaSentence(failure.body?.errors[0]?.detail) ?? failure.message)
					: 'The upload didn’t finish, so nothing was saved. Try again.';
		} finally {
			sending = false;
		}
	}

	function take(list: FileList | null) {
		const chosen = list?.[0];
		if (chosen) {
			void store(chosen, null, null);
		}
	}

	function openMaker(replacing: string | null) {
		changing = replacing;
		making = true;
	}
</script>

<label
	class="drop"
	class:over
	for={inputId}
	ondragover={(event) => {
		event.preventDefault();
		over = true;
	}}
	ondragleave={() => (over = false)}
	ondrop={(event) => {
		event.preventDefault();
		over = false;
		take(event.dataTransfer?.files ?? null);
	}}
>
	<b>{sending ? 'Uploading…' : 'Drag and drop a preview file, or browse.'}</b>
	{#if cap !== null}Up to {sizeWords(cap)}.{/if}
	<input
		id={inputId}
		type="file"
		disabled={sending}
		onchange={(event) => {
			take(event.currentTarget.files);
			event.currentTarget.value = '';
		}}
	/>
</label>

<div class="res-acts">
	<Button
		disabled={source === null || sending}
		reason={source === null
			? 'Upload a PDF first to make a preview from it.'
			: undefined}
		onclick={() => openMaker(null)}
	>
		Make a preview from your file
	</Button>
</div>

{#if refusal !== null}
	<Banner tone="bad">{refusal}</Banner>
{/if}

{#each previews as preview (preview.hash)}
	{@const viewable = viewSource(preview)}
	<div class="res-line res-file">
		<span class="res-line-what">
			<StatusPill tone="ok" label="uploaded" />
			<span class="res-line-t">{preview.name ?? 'Preview'}</span>
		</span>
		<span class="res-file-acts">
			<Button
				small
				disabled={viewable === null}
				reason={viewable === null ? 'Open this on the computer that made it.' : undefined}
				onclick={() => (viewing = preview)}>View</Button
			>
			<Button
				small
				disabled={source === null || sending}
				reason={source === null
					? 'Upload the PDF first to remake this preview.'
					: sending
						? 'Wait for the upload to finish.'
						: undefined}
				onclick={() => openMaker(preview.hash)}>Change</Button
			>
			<Button small disabled={sending} reason={sending ? 'Wait for the upload to finish.' : undefined} onclick={() => (renaming = preview.hash)}>Rename</Button>
			<Button
				small
				danger
				onclick={() => {
					forgetRecipe(preview.hash);
					onRemove(preview.hash);
				}}>Remove</Button
			>
		</span>
		{#if renaming === preview.hash}
			<FileRename
				name={preview.name ?? ''}
				what={preview.name ?? 'this preview'}
				onSave={(name) => {
					renaming = null;
					onRename(preview.hash, name);
				}}
				onCancel={() => (renaming = null)}
			/>
		{/if}
	</div>
{/each}

{#if making && pdfSource !== null}
	<PreviewMaker
		source={pdfSource}
		{sellerName}
		{uploadLabel}
		title={changing === null ? 'Make a preview' : 'Change the preview'}
		initial={changing === null ? null : readRecipe(changing)}
		onmade={(file, recipe) => {
			making = false;
			void store(file, recipe, changing);
		}}
		oncancel={() => (making = false)}
	/>
{/if}

{#if viewing !== null}
	{@const shown = viewing}
	{@const from = viewSource(shown)}
	{#if from !== null}
		<FileViewer
			open={viewing !== null}
			name={shown.name ?? 'Preview'}
			contentType={contentTypeOf(shown)}
			bytes={from.bytes}
			onClose={() => (viewing = null)}
		/>
	{/if}
{/if}

