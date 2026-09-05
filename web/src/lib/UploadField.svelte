<script lang="ts">
	import { ApiFailure, api, type ArchiveMode, type UploadedView } from '$lib/api';
	import { formatBytes, quotaSentence } from '$lib/authoring';
	import Banner from '$lib/Banner.svelte';
	import StatusPill from '$lib/StatusPill.svelte';

	let {
		uploaded,
		keepWhole,
		onUploaded,
		onKeepWhole
	}: {
		uploaded: UploadedView | null;
		keepWhole: boolean;
		onUploaded: (result: UploadedView | null) => void;
		onKeepWhole: (whole: boolean) => void;
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

	const percent = $derived(Math.round(sent * 100));
	const headroom = $derived(
		uploaded === null
			? null
			: `${formatBytes(uploaded.stored_bytes)} of ${formatBytes(uploaded.storage_bytes_max)} stored`
	);
</script>

<label class="drop" for={inputId}>
	<b>{sending ? 'Uploading…' : 'Choose the file buyers download'}</b>
	A PDF, PowerPoint, Word document, image or ZIP, up to 256 MB. The bytes are scanned before
	anything is stored.
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
	A ZIP is unpacked into one file per entry by default, which every Tes site carries and TPT does
	not: a TPT listing takes exactly one file. Tick this to upload the bundle as the single file it
	is.
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
		{uploaded.payload.length === 1 ? 'payload file' : 'payload files'}, a generated cover, and
		{uploaded.previews.length}
		{uploaded.previews.length === 1 ? 'preview' : 'previews'}. Uploading again replaces all of
		them.
	</p>
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
