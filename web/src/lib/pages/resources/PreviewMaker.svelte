<script lang="ts">
	import { untrack } from 'svelte';
	import type { PDFDocumentProxy } from 'pdfjs-dist';
	import { buildPreview, previewFileName } from './preview-pdf';
	import type { PreviewRecipe } from './preview-recipe';

	let {
		source,
		sellerName,
		uploadLabel = 'Make preview',
		initial = null,
		title = 'Make a preview',
		onmade,
		oncancel
	}: {
		/** The PDF whose pages this picks from: a name, and a way to get its
		 *  bytes. A `File` the teacher just chose and a file kept on this
		 *  machine's library both fit, and neither has to be read until the
		 *  dialog opens. */
		source: { name: string; bytes: () => Promise<ArrayBuffer> };
		/** Prefills the watermark, which the teacher can then edit. */
		sellerName: string;
		/** What the confirming button says, because on one surface pressing
		 *  it is what sends the derived file to Teachouse and the label has
		 *  to say so. */
		uploadLabel?: string;
		/** The dialog's heading: making one, or changing the one there is. */
		title?: string;
		/** The choices that made the preview being changed, where this
		 *  browser remembers them. Absent opens on the defaults. */
		initial?: PreviewRecipe | null;
		/** The made file, and the choices that made it, so a later Change can
		 *  reopen on them. */
		onmade: (file: File, recipe: PreviewRecipe) => void;
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
	/** The pages the watermark is drawn on, among those chosen. Every page
	 *  starts ticked, so leaving the set alone is the old behaviour. */
	let marked = $state<Set<number>>(new Set());
	let watermarked = $state(untrack(() => initial?.watermark ?? true));
	// The prefill, not a binding: the teacher may write a pen name over it, and
	// a later change to the account's name must not wipe what they typed.
	let name = $state(untrack(() => initial?.watermarkText ?? sellerName));
	let busy = $state(false);
	let refusal = $state<string | null>(null);
	/** Whether the first chosen page is drawn as the preview will carry it. */
	let showResult = $state(false);
	let result = $state<string | null>(null);
	const firstChosen = $derived(chosen.size === 0 ? null : Math.min(...chosen));
	const RESULT_WIDTH = 360;

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

	// Redrawn whenever what it depends on moves, and only while shown. Each
	// run claims a turn, so a slow draw for an older choice cannot land over a
	// newer one.
	let turn = 0;
	$effect(() => {
		const page = firstChosen;
		const mark = watermarked && name.trim() !== '' ? name : null;
		const markThis = page !== null && marked.has(page);
		turn += 1;
		const mine = turn;
		result = null;
		if (!showResult || page === null || pageCount === 0) {
			return;
		}
		void drawResult(page, mark, markThis).then((drawn) => {
			if (mine === turn) {
				result = drawn;
			}
		});
	});

	async function pdfjsLib() {
		const pdfjs = await import('pdfjs-dist');
		const worker = await import('pdfjs-dist/build/pdf.worker.min.mjs?url');
		pdfjs.GlobalWorkerOptions.workerSrc = worker.default;
		return pdfjs;
	}

	async function load(file: { bytes: () => Promise<ArrayBuffer> }) {
		try {
			const whole = await file.bytes();
			bytes = whole;
			const pdfjs = await pdfjsLib();
			const pdf = await pdfjs.getDocument({ data: whole.slice(0) }).promise;
			pageCount = pdf.numPages;
			thumbnails = Array.from({ length: pageCount }, () => null);
			const every = Array.from({ length: pageCount }, (_, index) => index + 1);
			const within = (page: number) => page <= pageCount;
			const remembered = untrack(() => initial);
			// A remembered choice is kept only where the pages still exist: the
			// source may be a different edition of the file than the one that
			// made the preview.
			chosen = new Set(remembered?.pages.filter(within) ?? []);
			marked = new Set(remembered?.marked.filter(within) ?? every);
			// In page order, one at a time: the teacher reads the first pages
			// first, and a document of a hundred pages should not render a
			// hundred canvases at once.
			for (let page = 1; page <= pageCount; page += 1) {
				thumbnails[page - 1] = await thumbnailOf(pdf, page, THUMBNAIL_WIDTH);
			}
		} catch {
			refusal = 'Teachouse could not open that PDF. Choose a different file.';
		}
	}

	/** The one page as the made preview will carry it: built by the same
	 *  `buildPreview` the Make button runs, then drawn, so what is shown is
	 *  the output rather than an imitation of it. */
	async function drawResult(
		page: number,
		mark: string | null,
		markThis: boolean
	): Promise<string | null> {
		if (bytes === null) {
			return null;
		}
		try {
			const made = await buildPreview(bytes.slice(0), [page], mark, markThis ? [page] : []);
			const pdfjs = await pdfjsLib();
			const task = pdfjs.getDocument({ data: made });
			try {
				return await thumbnailOf(await task.promise, 1, RESULT_WIDTH);
			} finally {
				void task.destroy();
			}
		} catch {
			return null;
		}
	}

	async function thumbnailOf(
		pdf: PDFDocumentProxy,
		page: number,
		width: number
	): Promise<string | null> {
		const drawn = await pdf.getPage(page);
		const scale = width / drawn.getViewport({ scale: 1 }).width;
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

	function toggleMark(page: number, on: boolean) {
		const next = new Set(marked);
		if (on) {
			next.add(page);
		} else {
			next.delete(page);
		}
		marked = next;
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
			const made = await buildPreview(bytes.slice(0), pages, mark, [...marked]);
			if (made.byteLength > CAP_BYTES) {
				refusal = 'That preview is over 30 MB. Choose fewer pages.';
				return;
			}
			// `.slice()` because a `Uint8Array` may view a larger buffer, and a
			// `File` takes the buffer, not the view.
			const pdf = made.slice().buffer;
			onmade(new File([pdf], previewFileName(source.name), { type: 'application/pdf' }), {
				pages,
				marked: [...marked].sort((one, other) => one - other),
				watermark: watermarked,
				watermarkText: name
			});
		} catch {
			refusal = 'Teachouse could not make that preview. Try fewer pages.';
		} finally {
			busy = false;
		}
	}
</script>

<dialog bind:this={element} aria-labelledby="preview-maker-title" onclose={oncancel}>
	<div class="dialog-body">
		<h2 id="preview-maker-title">{title}</h2>
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
					{#if watermarked && chosen.has(page)}
						<!-- A second tick, per page, so a cover or a sample page can
						     go out clean while the rest carry the name. Stops the
						     click reaching the page's own label, which would also
						     untick the page. -->
						<span class="mark-on">
							<input
								type="checkbox"
								checked={marked.has(page)}
								aria-label="Watermark page {page}"
								onclick={(event) => event.stopPropagation()}
								onchange={(event) => toggleMark(page, event.currentTarget.checked)}
							/>
							Watermark
						</span>
					{/if}
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

		<label class="mark">
			<input type="checkbox" bind:checked={showResult} disabled={chosen.size === 0} />
			<span>Preview the result</span>
		</label>
		{#if showResult && firstChosen !== null}
			<figure class="result">
				{#if result === null}
					<span class="waiting" aria-hidden="true"></span>
				{:else}
					<img src={result} alt="Page {firstChosen} as buyers will see it" />
				{/if}
				<figcaption class="n">Page {firstChosen}, as buyers will see it</figcaption>
			</figure>
		{/if}

		<div class="actions">
			<span class="count">
				{chosen.size}
				{chosen.size === 1 ? 'page' : 'pages'} selected
			</span>
			<button type="button" class="btn" onclick={oncancel} disabled={busy}>Cancel</button>
			<button type="button" class="cta" onclick={make} disabled={busy || chosen.size === 0}>
				{busy ? 'Making…' : uploadLabel}
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


	.mark-on {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		font-size: 11px;
		color: var(--muted);
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

	.result {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: var(--s-2);
		margin: var(--s-3) 0 0;
		padding: var(--s-3);
		border: 1px solid var(--line);
		border-radius: var(--r-panel);
		background: var(--ground);
	}

	.result img {
		max-width: 100%;
		max-height: 50vh;
		box-shadow: var(--sh-1);
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
