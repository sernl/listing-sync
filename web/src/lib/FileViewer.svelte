<script lang="ts">
	import type { PDFDocumentLoadingTask, PDFDocumentProxy } from 'pdfjs-dist';
	import { lightDismiss } from '$lib/dismiss';
	import {
		OPENS_ELSEWHERE,
		currentPage,
		pageWindow,
		viewerMode
	} from '$lib/pages/resources/file-viewer';

	let {
		open,
		name,
		contentType,
		bytes,
		url,
		onClose,
		onOpenElsewhere
	}: {
		open: boolean;
		name: string;
		contentType: string;
		/** Read when the dialog opens, never earlier. */
		bytes: () => Promise<ArrayBuffer>;
		/** Where the file can be streamed from instead: Teachouse's copy. A
		 *  PDF is then read a range at a time, so its first pages draw before
		 *  the rest of a large file has arrived. */
		url?: string;
		onClose: () => void;
		/** Hand the file to another application on this machine, where the
		 *  surface can. Absent where it cannot, and the button is not shown. */
		onOpenElsewhere?: () => void;
	} = $props();

	const mode = $derived(viewerMode(contentType));

	let element = $state<HTMLDialogElement | null>(null);
	let scroller = $state<HTMLDivElement | null>(null);
	let refusal = $state<string | null>(null);
	let imageUrl = $state<string | null>(null);
	let pageCount = $state(0);
	/** One canvas slot per page, drawn only inside the window. */
	let canvases = $state<(HTMLCanvasElement | null)[]>([]);
	let heights = $state<number[]>([]);
	let current = $state(1);

	let task: PDFDocumentLoadingTask | null = null;
	let pdf: PDFDocumentProxy | null = null;
	let drawn = new Map<number, HTMLCanvasElement>();
	let width = 600;

	// Guarded on the element's own state: `showModal` on a dialog that is
	// already modal throws, and this effect re-runs whenever the element is
	// bound as well as when `open` moves.
	$effect(() => {
		if (open && element !== null && !element.open) {
			element.showModal();
			void load();
		} else if (!open) {
			element?.close();
		}
	});

	$effect(() => () => release());

	function release() {
		if (imageUrl !== null) {
			if (imageUrl.startsWith('blob:')) {
				URL.revokeObjectURL(imageUrl);
			}
			imageUrl = null;
		}
		for (const canvas of drawn.values()) {
			canvas.width = 0;
			canvas.height = 0;
		}
		drawn = new Map();
		void task?.destroy();
		task = null;
		pdf = null;
	}

	async function load() {
		refusal = null;
		try {
			if (mode === 'image') {
				imageUrl =
					url ?? URL.createObjectURL(new Blob([await bytes()], { type: contentType }));
				return;
			}
			if (mode !== 'pdf') {
				return;
			}
			const pdfjs = await import('pdfjs-dist');
			const worker = await import('pdfjs-dist/build/pdf.worker.min.mjs?url');
			pdfjs.GlobalWorkerOptions.workerSrc = worker.default;
			// A megabyte a range: a 30 MB PDF is thirty requests at most, and
			// the first page usually needs one or two.
			task = pdfjs.getDocument(
				url === undefined
					? { data: (await bytes()).slice(0) }
					: { url, rangeChunkSize: 1024 * 1024, withCredentials: true }
			);
			pdf = await task.promise;
			pageCount = pdf.numPages;
			width = Math.max(320, (scroller?.clientWidth ?? 640) - 32);
			// Every page's height at the drawn width, so the scroller has its
			// full length before any page is drawn and the reader can jump.
			const first = await pdf.getPage(1);
			const ratio = width / first.getViewport({ scale: 1 }).width;
			heights = Array.from({ length: pageCount }, () => 0);
			for (let page = 1; page <= pageCount; page += 1) {
				const viewport = (await pdf.getPage(page)).getViewport({ scale: ratio });
				heights[page - 1] = Math.ceil(viewport.height);
			}
			canvases = Array.from({ length: pageCount }, () => null);
			await draw();
		} catch {
			refusal =
				url === undefined
					? 'Teachouse could not open that file on this device.'
					: 'Teachouse could not open that file. Try again, or download it.';
		}
	}

	/** Draws the pages in the window around the current one, at device-pixel
	 *  width, and releases the ones that left it. */
	async function draw() {
		if (pdf === null) {
			return;
		}
		const wanted = new Set(pageWindow(current, pageCount));
		for (const [page, canvas] of drawn) {
			if (!wanted.has(page)) {
				canvas.width = 0;
				canvas.height = 0;
				drawn.delete(page);
				canvases[page - 1] = null;
			}
		}
		for (const page of wanted) {
			if (drawn.has(page)) {
				continue;
			}
			const proxy = await pdf.getPage(page);
			const ratio = width / proxy.getViewport({ scale: 1 }).width;
			const pixels = window.devicePixelRatio || 1;
			const viewport = proxy.getViewport({ scale: ratio * pixels });
			const canvas = document.createElement('canvas');
			canvas.width = Math.ceil(viewport.width);
			canvas.height = Math.ceil(viewport.height);
			canvas.style.width = `${width}px`;
			const context = canvas.getContext('2d');
			if (context === null) {
				continue;
			}
			await proxy.render({ canvas, canvasContext: context, viewport }).promise;
			proxy.cleanup();
			drawn.set(page, canvas);
			canvases[page - 1] = canvas;
		}
	}

	function scrolled() {
		if (scroller === null || mode !== 'pdf') {
			return;
		}
		let top = 0;
		const tops = heights.map((height) => {
			const at = top;
			top += height + 12;
			return at;
		});
		const next = currentPage(tops, scroller.scrollTop, scroller.clientHeight);
		if (next !== current) {
			current = next;
			void draw();
		}
	}

	function slot(node: HTMLDivElement, canvas: HTMLCanvasElement | null) {
		const place = (drawnCanvas: HTMLCanvasElement | null) => {
			node.replaceChildren(...(drawnCanvas === null ? [] : [drawnCanvas]));
		};
		place(canvas);
		return { update: place };
	}
</script>

<dialog use:lightDismiss bind:this={element} aria-labelledby="file-viewer-title" onclose={onClose} class="viewer">
	<div class="dialog-body">
		<h2 id="file-viewer-title">{name}</h2>
		{#if refusal !== null}
			<p class="refusal">{refusal}</p>
		{:else if mode === 'image'}
			{#if imageUrl !== null}
				<img class="picture" src={imageUrl} alt={name} />
			{:else}
				<p class="quiet">Opening…</p>
			{/if}
		{:else if mode === 'pdf'}
			<div class="pages" bind:this={scroller} onscroll={scrolled}>
				{#if pageCount === 0}
					<p class="quiet">Opening…</p>
				{/if}
				{#each heights as height, index (index)}
					<div class="sheet" style="min-height: {height}px" use:slot={canvases[index]}></div>
				{/each}
			</div>
			{#if pageCount > 0}
				<p class="quiet">Page {current} of {pageCount}</p>
			{/if}
		{:else}
			<p>{OPENS_ELSEWHERE}</p>
		{/if}

		<div class="actions">
			<button class="btn" type="button" onclick={onClose}>Close</button>
			{#if onOpenElsewhere !== undefined}
				<button class="btn" type="button" onclick={onOpenElsewhere}>Open in another app</button>
			{/if}
		</div>
	</div>
</dialog>

<style>
	.viewer {
		width: min(920px, calc(100vw - 32px));
	}

	.pages {
		max-height: 70vh;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 12px;
		padding: 4px 0;
	}

	.sheet {
		background: var(--surface);
		box-shadow: 0 1px 3px color-mix(in srgb, var(--ink) 18%, transparent);
	}

	.picture {
		max-width: 100%;
		max-height: 70vh;
		display: block;
		margin: 0 auto;
	}
</style>
