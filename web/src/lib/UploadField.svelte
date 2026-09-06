<script lang="ts">
	import { ApiFailure, api, type ArchiveMode, type UploadedView } from '$lib/api';
	import { formatBytes, quotaSentence } from '$lib/authoring';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { STORAGE_NOT_RECLAIMED } from '$lib/pages/resources/files';

	let {
		uploaded,
		keepWhole,
		onUploaded,
		onKeepWhole,
		onCleared
	}: {
		uploaded: UploadedView | null;
		keepWhole: boolean;
		onUploaded: (result: UploadedView | null) => void;
		onKeepWhole: (whole: boolean) => void;
		/** The seller taking back a file they had just chosen, before anything
		 *  is created. The parent drops the handles; nothing is deleted and no
		 *  storage is reclaimed, which the copy beside the control says. */
		onCleared: () => void;
	} = $props();

	const base = $props.id();
	const inputId = `${base}-file`;
	const wholeId = `${base}-whole`;

	let sending = $state(false);
	let sent = $state(0);
	let refusal = $state<string | null>(null);
	let name = $state<string | null>(null);

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

	async function send(file: File) {
		sending = true;
		sent = 0;
		refusal = null;
		name = file.name;
		const archive: ArchiveMode = keepWhole ? 'keep_whole' : 'explode';
		try {
			onUploaded(await api.upload(file, archive, (fraction) => (sent = fraction)));
		} catch (failure) {
			onUploaded(null);
			refusal = refusalOf(failure);
		} finally {
			sending = false;
		}
	}

	function chosen(event: Event & { currentTarget: HTMLInputElement }) {
		const file = event.currentTarget.files?.[0];
		if (file) {
			void send(file);
		}
		event.currentTarget.value = '';
	}

	/** Drops what was chosen, here and in the parent's draft.
	 *
	 *  The bytes are untouched: they reached `POST /v1/uploads` when the file
	 *  was chosen, they are already counted against the organisation's storage,
	 *  and this removes the handles rather than the file. */
	function clear() {
		name = null;
		refusal = null;
		sent = 0;
		onCleared();
	}

	const percent = $derived(Math.round(sent * 100));
	const headroom = $derived(
		uploaded === null
			? null
			: `${formatBytes(uploaded.stored_bytes)} of ${formatBytes(uploaded.storage_bytes_max)} stored`
	);
</script>

<label class="drop" for={inputId}>
	<b>{sending ? 'Uploading…' : 'Choose the file buyers download'}</b>
	A PDF, PowerPoint, Word document, image or ZIP, up to 256 MB, checked for viruses before it is
	kept.
	<input id={inputId} type="file" disabled={sending} onchange={chosen} />
</label>

<div class="inline-choices" style="margin-top: 12px">
	<label for={wholeId}>
		<input
			id={wholeId}
			type="checkbox"
			checked={keepWhole}
			disabled={sending}
			onchange={(event) => onKeepWhole(event.currentTarget.checked)}
		/>
		Keep this ZIP whole
	</label>
</div>
<p class="foot-note">
	A ZIP is unpacked into one file per item inside it. Tes carries all of them; TPT takes only one,
	so tick this to send the ZIP as a single file.
</p>

{#if sending}
	<div
		class="uf-meter"
		role="progressbar"
		aria-label="Upload progress"
		aria-valuenow={percent}
		aria-valuemin={0}
		aria-valuemax={100}
	>
		<div class="uf-fill" style="width: {percent}%"></div>
	</div>
	<p class="uf-note">{name} — {percent}% sent</p>
{/if}

{#if refusal !== null}
	<Banner tone="bad">{refusal}</Banner>
{/if}

{#if uploaded !== null}
	<div class="uf-row">
		<span class="uf-what">
			<StatusPill tone="ok" label="stored" />
			<span class="uf-name" title={name ?? undefined}>{name ?? 'the upload'}</span>
		</span>
		<span class="uf-at">{headroom}</span>
		<span class="uf-acts">
			<Button
				small
				danger
				disabled={sending}
				reason={sending ? 'A file is being uploaded.' : undefined}
				onclick={clear}>Remove</Button
			>
		</span>
	</div>
	{#each uploaded.payload as file (file.hash)}
		<div class="uf-row">
			<span class="uf-what">
				<StatusPill tone="flat" label={file.kind} />
				<span class="uf-hash" title={file.hash}>{file.hash.slice(0, 12)}…</span>
			</span>
			<span class="uf-at">{formatBytes(file.byte_len)}</span>
		</div>
	{/each}
	<p class="foot-note">
		{uploaded.payload.length}
		{uploaded.payload.length === 1 ? 'file' : 'files'}, a thumbnail made from it, and
		{uploaded.previews.length}
		{uploaded.previews.length === 1 ? 'preview' : 'previews'}. Uploading again replaces all of them.
	</p>
	<p class="foot-note">{STORAGE_NOT_RECLAIMED}</p>
{/if}

<style>
	/* This control's own rules, in the component. The drop panel, the ZIP
	   choice and the note under it still take `app.css`'s `.drop`,
	   `.inline-choices` and `.foot-note`; converting those changes the idle
	   state, which the current capture set is being read against, so they are
	   named in the report rather than moved here. Tokens only. */

	/* One stored file: a name at the left and one figure at the right. Not a
	   row card — these carry no thumbnail and no action, which is the same
	   reading the resource page's runs and files get. */
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

	/* The handle, which is the only thing that identifies a stored file to the
	   server, so it is shown in a face that makes a digit distinct. */
	.uf-hash {
		font-size: 12px;
		color: var(--faint);
		font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
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
