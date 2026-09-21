<script lang="ts">
	// The files buyers download: a drop zone that takes several at once, one
	// upload per file, and one row per file with a way to take it back.
	//
	// Several files rather than one, because a teacher's resource is often a
	// worksheet and its answer key and TPT's own ZIP-it-yourself workaround is
	// the thing the ZIP tick exists for. `POST /v1/uploads` takes one file per
	// request and answers with that file's own handles, so the client posts one
	// request per file and joins the answers; the cover is the first upload's,
	// because the thumbnail is drawn from the file buyers see first.
	import { ApiFailure, api, type ArchiveMode, type FormLimits } from '$lib/api';
	import { formatBytes, quotaSentence } from '$lib/authoring';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import Note from '$lib/Note.svelte';
	import { sizeWords, type StoredFile } from '$lib/tpt-form';

	let {
		files,
		keepWhole,
		limits,
		onFiles,
		onPdf,
		onKeepWhole
	}: {
		/** Every file the teacher has added, in the order they added them. */
		files: readonly StoredFile[];
		keepWhole: boolean;
		/** The server's own caps, or null while the vocabulary is still read. */
		limits: FormLimits | null;
		/** The whole list, after an addition or a removal. The parent recomposes
		 *  the draft's payload and cover from it, so this control never has to
		 *  decide which handle is the cover. */
		onFiles: (files: StoredFile[]) => void;
		/** A PDF the teacher chose, with its bytes still in this browser, which
		 *  is what the preview maker cuts pages out of. Reported as the `File`
		 *  rather than as a handle: the maker renders the pages here on the
		 *  teacher's own device, and a stored handle would have to be fetched
		 *  back to do it. */
		onPdf: (file: File) => void;
		onKeepWhole: (whole: boolean) => void;
	} = $props();

	const base = $props.id();
	const inputId = `${base}-file`;
	const wholeId = `${base}-whole`;

	let over = $state(false);
	let sending = $state<{ name: string; percent: number } | null>(null);
	let queued = $state(0);
	let refusal = $state<string | null>(null);

	function refusalOf(failure: unknown): string {
		if (!(failure instanceof ApiFailure)) {
			return 'The upload did not finish. Nothing was stored.';
		}
		if (failure.status === 0) {
			return 'The upload did not reach us. Nothing was stored; try again.';
		}
		if (failure.code() === 'quota_exceeded') {
			return quotaSentence(failure.body?.errors[0]?.detail) ?? failure.message;
		}
		if (failure.code() === 'blob_store_unavailable') {
			return 'This deployment cannot store bytes yet, so no file was accepted.';
		}
		return failure.message;
	}

	/** One request per file, in the order they were chosen.
	 *
	 *  Sequential rather than parallel: the storage quota is checked per upload
	 *  and a refusal part-way through has to leave the files before it stored
	 *  and the rest untouched, which is only legible if they went one at a
	 *  time. A refusal stops the run, because the next file would be refused
	 *  for the same reason. */
	async function send(chosen: File[]) {
		refusal = null;
		const archive: ArchiveMode = keepWhole ? 'keep_whole' : 'explode';
		let landed = [...files];
		queued = chosen.length;
		for (const file of chosen) {
			sending = { name: file.name, percent: 0 };
			try {
				const result = await api.upload(
					file,
					archive,
					(fraction) => (sending = { name: file.name, percent: Math.round(fraction * 100) })
				);
				landed = [
					...landed,
					{
						name: file.name,
						bytes: file.size,
						payload: result.payload,
						cover: result.cover,
						storedBytes: result.stored_bytes,
						storageBytesMax: result.storage_bytes_max
					}
				];
				onFiles(landed);
			} catch (failure) {
				refusal = refusalOf(failure);
				break;
			} finally {
				queued -= 1;
			}
		}
		sending = null;
		queued = 0;
	}

	function take(list: FileList | null) {
		const chosen = [...(list ?? [])];
		if (chosen.length === 0) {
			return;
		}
		const pdf = chosen.find((file) => file.type === 'application/pdf');
		if (pdf !== undefined) {
			onPdf(pdf);
		}
		void send(chosen);
	}

	const headroom = $derived.by(() => {
		const last = files[files.length - 1];
		return last === undefined
			? null
			: `${formatBytes(last.storedBytes)} of ${formatBytes(last.storageBytesMax)} stored`;
	});
</script>

<!-- A label over a file input rather than a button: the input is what a
     keyboard reaches and what the browser's own picker is bound to, and the
     drop handlers sit on the same element so dropping and browsing land in one
     place. -->
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
	<b>{sending === null ? 'Drag and drop your files here, or browse.' : 'Uploading…'}</b>
	You can add more than one.
	<input
		id={inputId}
		type="file"
		multiple
		disabled={sending !== null}
		onchange={(event) => {
			take(event.currentTarget.files);
			event.currentTarget.value = '';
		}}
	/>
</label>

{#if limits}
	<Note>
		Files up to {sizeWords(limits.product_file.max_size_bytes)}; each thumbnail up to
		{sizeWords(limits.thumbnail.max_size_bytes)}.
	</Note>
{/if}

<div class="inline-choices" style="margin-top: 12px">
	<label for={wholeId}>
		<input
			id={wholeId}
			type="checkbox"
			checked={keepWhole}
			disabled={sending !== null}
			onchange={(event) => onKeepWhole(event.currentTarget.checked)}
		/>
		Keep this ZIP as one file
	</label>
</div>
<Note>TPT takes one file; Tes takes all of them.</Note>

{#if sending !== null}
	<div
		class="uf-meter"
		role="progressbar"
		aria-label="Upload progress"
		aria-valuenow={sending.percent}
		aria-valuemin={0}
		aria-valuemax={100}
	>
		<div class="uf-fill" style="width: {sending.percent}%"></div>
	</div>
	<p class="uf-note">
		{sending.name} — {sending.percent}% sent{queued > 1 ? `, ${queued - 1} to go` : ''}
	</p>
{/if}

{#if refusal !== null}
	<Banner tone="bad">{refusal}</Banner>
{/if}

{#each files as file, index (file.name + index)}
	<div class="uf-row">
		<span class="uf-what">
			<StatusPill tone="ok" label="stored" />
			<span class="uf-name" title={file.name}>{file.name}</span>
			{#if index === 0}<span class="uf-at">the thumbnail is drawn from this one</span>{/if}
		</span>
		<span class="uf-at">{formatBytes(file.bytes)}</span>
		<span class="uf-acts">
			<Button
				small
				danger
				disabled={sending !== null}
				reason={sending === null ? undefined : 'A file is being uploaded.'}
				onclick={() => onFiles(files.filter((_, at) => at !== index))}>Remove</Button
			>
		</span>
	</div>
{/each}

{#if headroom !== null}
	<Note>{headroom}</Note>
{/if}

<style>
	/* This control's own rules, in the component. The drop panel and the ZIP
	   choice still take `app.css`'s `.drop` and `.inline-choices` — including
	   the state a drag puts the panel in — the note under it is `Note.svelte`,
	   and what is here is the row and the meter. Tokens only. */

	/* One stored file: a name at the left and one figure at the right. Not a
	   row card — these carry no thumbnail, only the one action. */
	.uf-row {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: 12px;
		flex-wrap: wrap;
		padding: 9px 2px;
		border-top: 1px solid var(--line);
	}

	.uf-row:first-of-type {
		margin-top: 8px;
	}

	.uf-what {
		display: flex;
		align-items: baseline;
		gap: 8px;
		flex-wrap: wrap;
		min-width: 0;
	}

	.uf-name {
		font-size: 13.5px;
		font-weight: 600;
		overflow-wrap: anywhere;
	}

	/* The row's own action, kept off the baseline the name and the figure share
	   so a button does not sit on a text baseline. */
	.uf-acts {
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.uf-at {
		font-size: 12px;
		color: var(--faint);
		white-space: nowrap;
		font-variant-numeric: tabular-nums;
	}

	.uf-meter {
		height: 6px;
		border-radius: var(--r-pill);
		background: var(--soon-soft);
		overflow: hidden;
		margin-top: 10px;
	}

	.uf-fill {
		height: 100%;
		background: var(--accent);
		transition: width 120ms linear;
	}

	@media (prefers-reduced-motion: reduce) {
		.uf-fill {
			transition: none;
		}
	}

	.uf-note {
		margin: 6px 0 0;
		font-size: 12.5px;
		color: var(--muted);
	}
</style>
