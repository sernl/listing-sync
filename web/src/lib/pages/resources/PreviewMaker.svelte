<script lang="ts">
	import { untrack } from 'svelte';
	import type { PDFDocumentProxy } from 'pdfjs-dist';
	import { buildPreview, previewFileName } from './preview-pdf';

	let {
		source,
		sellerName,
		onmade,
		oncancel
	}: {
		/** The PDF the teacher uploaded, whose pages this picks from. */
		source: File;
		/** Prefills the watermark, which the teacher can then edit. */
		sellerName: string;
		onmade: (file: File) => void;
		oncancel: () => void;
	} = $props();

	/** TPT's preview cap, which is what the console is holding the preview to. */
	const CAP_BYTES = 30 * 1024 * 1024;

	const THUMBNAIL_WIDTH = 160;

	let element = $state<HTMLDialogElement | null>(null);
	let pageCount = $state(0);
	/** One data URL per page, in page order, null until that page is drawn. */
	let thumbnails = $state<(string | null)[]>([]);
	let chosen = $state<Set<number>>(new Set());
	let watermarked = $state(true);
	// The prefill, not a binding: the teacher may write a pen name over it, and
	// a later change to the account's name must not wipe what they typed.
	let name = $state(untrack(() => sellerName));
	let busy = $state(false);
	let refusal = $state<string | null>(null);

	/** The upload's bytes, kept whole: pdf.js is handed a copy because it may
	 *  transfer what it is given to its worker, and a detached buffer is not a
	 *  document pdf-lib can read afterwards. */
	let bytes: ArrayBuffer | null = null;

	$effect(() => {
		if (element !== null && !element.open) {
			element.showModal();
		}
	});

	$effect(() => {
		void load(source);
	});

	async function load(file: File) {
		try {
			const whole = await file.arrayBuffer();
			bytes = whole;
			const pdfjs = await import('pdfjs-dist');
			const worker = await import('pdfjs-dist/build/pdf.worker.min.mjs?url');
			pdfjs.GlobalWorkerOptions.workerSrc = worker.default;
			const pdf = await pdfjs.getDocument({ data: whole.slice(0) }).promise;
			pageCount = pdf.numPages;
			thumbnails = Array.from({ length: pageCount }, () => null);
			// In page order, one at a time: the teacher reads the first pages
			// first, and a document of a hundred pages should not render a
			// hundred canvases at once.
			for (let page = 1; page <= pageCount; page += 1) {
				thumbnails[page - 1] = await thumbnailOf(pdf, page);
			}
		} catch {
			refusal = 'Teachouse could not open that PDF. Choose a different file.';
		}
	}

	async function thumbnailOf(pdf: PDFDocumentProxy, page: number): Promise<string | null> {
		const drawn = await pdf.getPage(page);
		const scale = THUMBNAIL_WIDTH / drawn.getViewport({ scale: 1 }).width;
		const viewport = drawn.getViewport({ scale });
		const canvas = document.createElement('canvas');
		canvas.width = Math.ceil(viewport.width);
		canvas.height = Math.ceil(viewport.height);
		const context = canvas.getContext('2d');
		if (context === null) {
			return null;
		}
		await drawn.render({ canvas, canvasContext: context, viewport }).promise;
		return canvas.toDataURL('image/png');
	}

	function toggle(page: number, on: boolean) {
		const next = new Set(chosen);
		if (on) {
			next.add(page);
		} else {
			next.delete(page);
		}
		chosen = next;
	}

	function selectAll() {
		chosen = new Set(Array.from({ length: pageCount }, (_, index) => index + 1));
	}

	function clear() {
		chosen = new Set();
	}

	async function make() {
		if (bytes === null || chosen.size === 0) {
			return;
		}
		busy = true;
		refusal = null;
		try {
			const pages = [...chosen].sort((one, other) => one - other);
			const mark = watermarked && name.trim() !== '' ? name : null;
			const made = await buildPreview(bytes.slice(0), pages, mark);
			if (made.byteLength > CAP_BYTES) {
				refusal = 'That preview is over 30 MB. Choose fewer pages.';
				return;
			}
			// `.slice()` because a `Uint8Array` may view a larger buffer, and a
			// `File` takes the buffer, not the view.
			const pdf = made.slice().buffer;
			onmade(new File([pdf], previewFileName(source.name), { type: 'application/pdf' }));
		} catch {
			refusal = 'Teachouse could not make that preview. Try fewer pages.';
		} finally {
			busy = false;
		}
	}
</script>

<dialog bind:this={element} aria-labelledby="preview-maker-title" onclose={oncancel}>
	<div class="dialog-body">
		<h2 id="preview-maker-title">Make a preview</h2>
		<p>Tick the pages you want buyers to see.</p>

		{#if refusal !== null}
			<p class="refusal">{refusal}</p>
		{/if}

		<div class="picks">
			<button type="button" class="btn small" onclick={selectAll} disabled={pageCount === 0}>
				Select all
			</button>
			<button type="button" class="btn small" onclick={clear} disabled={chosen.size === 0}>
				Clear
			</button>
		</div>

		<div class="pages">
			{#if pageCount === 0 && refusal === null}
				<p class="opening">Opening your PDF…</p>
			{/if}
			{#each thumbnails as thumbnail, index (index)}
				{@const page = index + 1}
				<label class="page {chosen.has(page) ? 'on' : ''}">
					<input
						type="checkbox"
						checked={chosen.has(page)}
						onchange={(event) => toggle(page, event.currentTarget.checked)}
					/>
					<span class="sheet">
						{#if thumbnail === null}
							<span class="waiting" aria-hidden="true"></span>
						{:else}
							<img src={thumbnail} alt="" />
						{/if}
					</span>
					<span class="n">Page {page}</span>
				</label>
			{/each}
		</div>

		<label class="mark">
			<input type="checkbox" bind:checked={watermarked} />
			<span>Watermark with my name</span>
		</label>
		{#if watermarked}
			<input class="who" bind:value={name} aria-label="Name written across each page" />
		{/if}

		<div class="actions">
			<span class="count">
				{chosen.size}
				{chosen.size === 1 ? 'page' : 'pages'} selected
			</span>
			<button type="button" class="btn" onclick={oncancel} disabled={busy}>Cancel</button>
			<button type="button" class="cta" onclick={make} disabled={busy || chosen.size === 0}>
				{busy ? 'Making…' : 'Make preview'}
			</button>
		</div>
	</div>
</dialog>

<style>
	.picks {
		display: flex;
		gap: 8px;
		margin-bottom: 10px;
	}

	.pages {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
		gap: 12px;
		max-height: 46vh;
		overflow-y: auto;
		padding: 2px;
	}

	.page {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 6px;
		padding: 8px;
		border: 1px solid var(--line);
		border-radius: var(--r-panel);
		background: var(--surface);
		cursor: pointer;
	}

	.page.on {
		border-color: var(--primary);
		box-shadow: 0 0 0 1px var(--primary);
	}

	/* A fixed box so a page that has not drawn yet holds the place its
	   thumbnail will take, and the grid does not jump as the pages arrive. */
	.sheet {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 100%;
		height: 200px;
		background: var(--ground);
		border-radius: var(--r-field);
		overflow: hidden;
	}

	.sheet img {
		max-width: 100%;
		max-height: 100%;
	}

	.waiting {
		width: 22px;
		height: 22px;
		border: 2px solid var(--line);
		border-top-color: var(--primary);
		border-radius: var(--r-pill);
		animation: spin 0.8s linear infinite;
	}

	@keyframes spin {
		to {
			transform: rotate(360deg);
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.waiting {
			animation: none;
		}
	}

	.opening {
		grid-column: 1 / -1;
		margin: 0;
		font-size: 12.5px;
		color: var(--muted);
	}

	.n {
		font-size: 12px;
		color: var(--muted);
	}

	.mark {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-top: 14px;
		font-size: 13px;
		color: var(--text);
	}

	.who {
		margin-top: 8px;
		width: 100%;
		padding: 7px 10px;
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		background: var(--surface);
		color: var(--text);
		font: inherit;
	}

	.count {
		margin-right: auto;
		font-size: 12.5px;
		color: var(--muted);
	}

	/* Two thumbnails across on a phone, which is as many as fit beside a
	   readable page number. */
	@media (max-width: 640px) {
		.pages {
			grid-template-columns: repeat(2, 1fr);
		}

		.sheet {
			height: 150px;
		}
	}
</style>
