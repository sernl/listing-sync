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
	import StatusPill from '$lib/StatusPill.svelte';
	import { sizeWords } from '$lib/tpt-form';
	import PreviewMaker from './PreviewMaker.svelte';

	let {
		previews,
		limits,
		source,
		sellerName,
		onAdd,
		onRemove
	}: {
		/** The previews this listing already carries. */
		previews: readonly FileHandle[];
		limits: FormLimits | null;
		/** The PDF chosen in this session, which is the only thing a preview
		 *  can be cut out of: the bytes have to be in this browser, and a file
		 *  uploaded on some earlier visit is not. */
		source: File | null;
		/** Whose name is written across the pages, where the teacher asks for
		 *  it. Their own shop name, never one composed here. */
		sellerName: string;
		onAdd: (handle: FileHandle) => void;
		onRemove: (hash: string) => void;
	} = $props();

	const base = $props.id();
	const inputId = `${base}-preview`;

	let over = $state(false);
	let sending = $state(false);
	let refusal = $state<string | null>(null);
	let making = $state(false);

	const cap = $derived(limits?.preview.max_size_bytes ?? null);

	/** The upload, with the one refusal this browser can make before a byte is
	 *  sent: the server refuses an oversized preview too, and reading that
	 *  answer back after a 30 MB upload is the worse way to learn it. */
	async function store(file: File) {
		if (cap !== null && file.size > cap) {
			refusal = `That preview is ${sizeWords(file.size)} and the limit is ${sizeWords(cap)}.`;
			return;
		}
		sending = true;
		refusal = null;
		try {
			const landed = await api.upload(file, 'keep_whole');
			const handle = landed.payload[0];
			if (handle === undefined) {
				refusal = 'That file was stored with no handle to attach.';
			} else {
				onAdd(handle);
			}
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure
					? (quotaSentence(failure.body?.errors[0]?.detail) ?? failure.message)
					: 'The upload did not finish. Nothing was stored.';
		} finally {
			sending = false;
		}
	}

	function take(list: FileList | null) {
		const chosen = list?.[0];
		if (chosen) {
			void store(chosen);
		}
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
			? 'Upload a PDF above and this can make a preview from it.'
			: undefined}
		onclick={() => (making = true)}
	>
		Make a preview from your file
	</Button>
</div>

{#if refusal !== null}
	<Banner tone="bad">{refusal}</Banner>
{/if}

{#each previews as preview (preview.hash)}
	<div class="res-line">
		<span class="res-line-what">
			<StatusPill tone="ok" label="stored" />
			<span class="res-line-t">{preview.name ?? 'Preview'}</span>
		</span>
		<Button small danger onclick={() => onRemove(preview.hash)}>Remove</Button>
	</div>
{/each}

{#if making && source !== null}
	<PreviewMaker
		{source}
		{sellerName}
		onmade={(file) => {
			making = false;
			void store(file);
		}}
		oncancel={() => (making = false)}
	/>
{/if}

