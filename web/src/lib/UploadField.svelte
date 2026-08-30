<script lang="ts">
	import { ApiFailure, api, type ArchiveMode, type UploadedView } from '$lib/api';
	import { formatBytes, quotaSentence } from '$lib/authoring';

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
		class="meter"
		role="progressbar"
		aria-label="Upload progress"
		aria-valuenow={percent}
		aria-valuemin={0}
		aria-valuemax={100}
	>
		<div class="fill" style="width: {percent}%"></div>
	</div>
	<p class="s">{name} — {percent}% sent</p>
{/if}

{#if refusal !== null}
	<p class="refusal">{refusal}</p>
{/if}

{#if uploaded !== null}
	<div class="file-row">
		<span class="badge live">stored</span>
		<span class="name" title={name ?? undefined}>{name ?? 'the upload'}</span>
		<span class="when">{headroom}</span>
	</div>
	{#each uploaded.payload as file (file.hash)}
		<div class="file-row">
			<span class="badge">{file.kind}</span>
			<span class="name mono" title={file.hash}>{file.hash.slice(0, 12)}…</span>
			<span class="when">{formatBytes(file.byte_len)}</span>
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
