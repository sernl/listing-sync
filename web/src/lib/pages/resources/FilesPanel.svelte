<script lang="ts">
	// The stored files of one resource, and the three sub-resource routes that
	// change them. Lifted out of `ResourceDetail` unchanged when the edit form
	// became the create form's second mode: the panel belongs beside the fields
	// a seller is editing, and neither page is a place for a hundred lines of
	// file machinery.
	import { useQueryClient } from '@tanstack/svelte-query';
	import { api, type FileView } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import Note from '$lib/Note.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import type { InventoryId } from '$lib/generated/vocab';
	import {
		desktopInvoker,
		libraryEntries,
		libraryOpenExternal,
		type LibraryEntry
	} from '$lib/desktop';
	import FileViewer from '$lib/FileViewer.svelte';
	import { OPEN_UNAVAILABLE, sourceOfKept } from './file-viewer';
	import {
		IDLE,
		advance,
		busy,
		fileRefusal,
		LAST_FILE_LOSES_THUMBNAIL,
		ROLE_WORD,
		STORAGE_NOT_RECLAIMED,
		fileName,
		fileWords,
		reachSentence,
		redrawsCover,
		removable,
		replaceable,
		retiresCover,
		stalesCover,
		type FileAction
	} from './files';

	let {
		product,
		files,
		inventories
	}: {
		product: string;
		files: readonly FileView[];
		inventories: readonly InventoryId[];
	} = $props();

	const queryClient = useQueryClient();

	// One action at a time, so a seller cannot remove the file an upload is in
	// the middle of replacing; the machine in `files.ts` is what holds that, and
	// it is tested there.
	let fileAction = $state<FileAction>(IDLE);
	// What a file change reaches, kept from the last one so the panel can say
	// where it landed rather than only that it landed.
	let fileReach = $state<string | null>(null);
	let picking: HTMLInputElement | null = $state(null);
	// Which row the picker was opened for, and `null` for the add. Held here
	// rather than in the machine because it outlives the confirmation: the
	// picker fires long after the control was pressed.
	let pickingFor: string | null = null;
	let pickingVerb: 'add' | 'replace' = 'replace';
	let chosen: File | null = null;

	const stored = $derived([...files]);

	// Which stored files this machine keeps, so a row can offer to show
	// them. Read once from the application; empty in a browser, where the
	// bytes are not here and nothing is offered.
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
	let viewing = $state<FileView | null>(null);

	async function openElsewhere(file: FileView) {
		const answer = await libraryOpenExternal(invoke, file.hash);
		if (answer.kind !== 'ok') {
			toast('error', answer.kind === 'refused' ? answer.detail : OPEN_UNAVAILABLE);
		}
	}

	function fileEvent(event: Parameters<typeof advance>[1]) {
		fileAction = advance(fileAction, event);
	}

	/** Opens the file picker for one row, or for the add. Nothing is destroyed
	 *  yet: a replacement asks for confirmation once the file is chosen, so the
	 *  question can name both files rather than only the one being lost. */
	function choose(verb: 'add' | 'replace', file: string | null) {
		if (busy(fileAction)) {
			return;
		}
		pickingVerb = verb;
		pickingFor = file;
		chosen = null;
		picking?.click();
	}

	function picked(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0] ?? null;
		// Cleared so choosing the same file twice in a row still fires a change.
		input.value = '';
		if (file === null) {
			return;
		}
		chosen = file;
		if (pickingVerb === 'add') {
			void sendFile('add', null, file);
			return;
		}
		fileEvent({
			kind: 'ask',
			verb: 'replace',
			file: pickingFor ?? '',
			chosen: file.name
		});
	}

	/** Uploads the chosen bytes, then names the handle back. `keep_whole`
	 *  because one replacement is one file: `explode` would answer with a
	 *  handle per entry for an archive, and the row takes exactly one. */
	async function sendFile(verb: 'add' | 'replace', file: string | null, bytes: File) {
		fileEvent({ kind: 'send', verb, file });
		try {
			const uploaded = await api.upload(bytes, 'keep_whole', (fraction) =>
				fileEvent({ kind: 'progress', fraction })
			);
			fileEvent({ kind: 'stored' });
			const landed = uploaded.payload[0];
			if (landed === undefined) {
				fileEvent({ kind: 'failed', sentence: 'That upload carried no file we can store.' });
				return;
			}
			// The name is the client's word: the upload sends a raw body, which
			// carries no filename, so it travels back beside the handle.
			const handle = { ...landed, name: bytes.name };
			if (verb === 'add') {
				const added = await api.addProductFile(product, 'payload', handle);
				await settleFile(added.reaches, 'File added.');
				return;
			}
			// No thumbnail is sent: the server renders one from these bytes
			// where it decides a redraw is due, and reports what it drew. A
			// handle offered from here would be a handle a client could point
			// at any file, which is the hole that closed by removing the field.
			const replaced = await api.replaceProductFile(product, file ?? '', handle);
			await settleFile(
				replaced.reaches,
				replaced.cover === undefined ? 'File replaced.' : 'File replaced, thumbnail redrawn.'
			);
		} catch (failure) {
			fileEvent({ kind: 'failed', sentence: fileRefusal(failure, verb) });
		}
	}

	async function removeFile(file: string) {
		fileEvent({ kind: 'send', verb: 'remove', file });
		try {
			const landed = await api.removeProductFile(product, file);
			await settleFile(
				landed.reaches,
				landed.thumbnail.state === 'redrawn'
					? 'File removed, thumbnail redrawn.'
					: landed.thumbnail.state === 'retired'
						? 'File removed, and the thumbnail with it.'
						: 'File removed.'
			);
		} catch (failure) {
			fileEvent({ kind: 'failed', sentence: fileRefusal(failure, 'remove') });
		}
	}

	async function settleFile(reaches: InventoryId[], said: string) {
		fileReach = reachSentence(reaches);
		fileEvent({ kind: 'settled' });
		await queryClient.invalidateQueries({ queryKey: queryKeys.product(product) });
		await queryClient.invalidateQueries({ queryKey: queryKeys.products });
		toast('info', said);
	}

	function confirmFile() {
		if (fileAction.kind !== 'confirming') {
			return;
		}
		const { verb, file } = fileAction;
		if (verb === 'remove') {
			void removeFile(file);
			return;
		}
		if (chosen !== null) {
			void sendFile('replace', file, chosen);
		}
	}
</script>

<!-- No panel of its own: this renders inside the form's own Files section,
     which already carries the heading and the sentence saying what the group
     is for. A second "Files" heading nested in the first is one heading too
     many, and it is what the render showed. -->
<div class="res-files">
	{#each stored as file (file.id)}
		{@const may = removable(stored, file.id, inventories.length > 0)}
		{@const swappable = replaceable(stored, file.id)}
		{@const mine = fileAction.kind !== 'idle' && fileAction.file === file.id}
		<div class="res-line res-file">
			<span class="res-line-what">
				<StatusPill tone="flat" label={ROLE_WORD[file.role]} />
				<span class="res-line-t" class:res-file-anon={file.name === undefined}
					>{fileName(file)}</span
				>
				<span class="res-line-id">{file.kind} · {file.scan}</span>
			</span>
			<span class="res-line-at">{file.byte_len.toLocaleString('en-GB')} bytes</span>
			<span class="res-file-acts">
				{#if kept.has(file.hash)}
					<Button small onclick={() => (viewing = file)}>View</Button>
				{/if}
				<Button
					small
					disabled={busy(fileAction) || !swappable.ok}
					reason={busy(fileAction)
						? 'One file change runs at a time.'
						: swappable.ok
							? undefined
							: swappable.reason}
					onclick={() => choose('replace', file.id)}>Replace…</Button
				>
				<Button
					small
					danger
					disabled={busy(fileAction) || !may.ok}
					reason={busy(fileAction)
						? 'One file change runs at a time.'
						: may.ok
							? undefined
							: may.reason}
					onclick={() => fileEvent({ kind: 'ask', verb: 'remove', file: file.id, chosen: null })}
					>Remove…</Button
				>
			</span>
			{#if mine && fileAction.kind === 'uploading'}
				<p class="res-file-say" role="status">
					Replacing {fileWords(file)}… {Math.round(fileAction.fraction * 100)}% sent
				</p>
			{:else if mine && fileAction.kind === 'writing'}
				<p class="res-file-say" role="status">Saving…</p>
			{:else if mine && fileAction.kind === 'confirming'}
				<div class="res-file-ask">
					<p class="res-file-say">
						{#if fileAction.verb === 'remove'}
							Remove {fileWords(file)} from this resource?
							{#if retiresCover(stored, file.id)}
								{LAST_FILE_LOSES_THUMBNAIL}
							{:else if stalesCover(stored, file.id)}
								The thumbnail is drawn from this file, so it is redrawn from the next one.
							{/if}
						{:else}
							Replace {fileWords(file)} with {fileAction.chosen}?
							{#if redrawsCover(stored, file.id)}
								The thumbnail is drawn from this file, so it is redrawn from the new one.
							{/if}
						{/if}
						{reachSentence(inventories)}
						{STORAGE_NOT_RECLAIMED}
					</p>
					<span class="res-file-acts">
						<Button small tier="primary" onclick={confirmFile}>
							{fileAction.verb === 'remove' ? 'Remove it' : 'Replace it'}
						</Button>
						<Button small onclick={() => fileEvent({ kind: 'cancel' })}>Keep it</Button>
					</span>
				</div>
			{:else if mine && fileAction.kind === 'refused'}
				<p class="res-file-say res-file-no" role="alert">{fileAction.sentence}</p>
			{/if}
		</div>
	{:else}
		<p class="res-note">No files recorded.</p>
	{/each}

	<div class="res-file-add">
		<Button
			small
			icon="plus"
			disabled={busy(fileAction)}
			reason={busy(fileAction) ? 'One file change runs at a time.' : undefined}
			onclick={() => choose('add', null)}>Add file</Button
		>
		{#if fileAction.kind === 'uploading' && fileAction.file === null}
			<span class="res-file-say" role="status"
				>Sending… {Math.round(fileAction.fraction * 100)}%</span
			>
		{:else if fileAction.kind === 'writing' && fileAction.file === null}
			<span class="res-file-say" role="status">Saving…</span>
		{:else if fileAction.kind === 'refused' && fileAction.file === null}
			<span class="res-file-say res-file-no" role="alert">{fileAction.sentence}</span>
		{/if}
	</div>

	<!-- Off-screen rather than hidden: a display:none input cannot be
	     clicked open in every browser, and this one is opened by script. -->
	<input
		class="res-file-pick"
		type="file"
		bind:this={picking}
		onchange={picked}
		tabindex="-1"
		aria-hidden="true"
	/>

	<Note>Changing a file here changes your Resources.</Note>
	<Note>{fileReach ?? reachSentence(inventories)}</Note>
	<Note>The thumbnail is drawn from the first file.</Note>
</div>

{#if viewing !== null}
	{@const shown = viewing}
	<FileViewer
		open={viewing !== null}
		name={fileName(shown)}
		contentType={kept.get(shown.hash)?.content_type ?? 'application/octet-stream'}
		bytes={sourceOfKept(invoke, fileName(shown), shown.hash).bytes}
		onClose={() => (viewing = null)}
		onOpenElsewhere={() => void openElsewhere(shown)}
	/>
{/if}
