<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { ApiFailure, api } from '$lib/api';
	import { formatBytes } from '$lib/authoring';
	import Button from '$lib/Button.svelte';
	import {
		desktopInvoker,
		type LibraryEntry,
		libraryEntries,
		libraryOpenExternal,
		libraryRemove,
		librarySettings,
		libraryUsage,
		setLibrarySettings
	} from '$lib/desktop';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import Icon from '$lib/Icon.svelte';
	import { machineHere } from '$lib/machine.svelte';
	import Note from '$lib/Note.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Pagination from '$lib/Pagination.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import Toggle from '$lib/Toggle.svelte';
	import {
		AVAILABILITY_LABEL,
		BROWSER_SENTENCE,
		EMPTY_FILTERS,
		FILES_STAY_ON_YOUR_MACHINES,
		HERE,
		KEEP_LABEL,
		NOT_KEEPING_SENTENCE,
		PAGE_SIZE,
		type Availability,
		type FileFilters,
		type Linked,
		countSentence,
		fileRows,
		filtersActive,
		filtersFromUrl,
		filtersToQuery,
		filtersToUrl,
		keptLine,
		pageWindow,
		removePrompt,
		seenLine,
		transferSentence,
		usageLine
	} from './files-browser';
	import '$lib/flow.css';
	import './resources.css';

	// Read once: whether this console runs inside the application does not
	// change while the page is open.
	const invoke = desktopInvoker();
	const queryClient = useQueryClient();

	/** The filters are the address, not component state: a machine's row links
	 *  here with `?device=`, and the page a seller copies has to be the page
	 *  they were looking at. */
	const filters = $derived(filtersFromUrl(page.url.searchParams));
	// The box is seeded from the address and writes it back as it is typed,
	// and the two must not disagree.
	let box = $state(filtersFromUrl(page.url.searchParams).q);
	$effect(() => {
		if (filters.q !== box.trim()) {
			box = filters.q;
		}
	});

	/** Replaces rather than pushes: a filter is not a place, and a back button
	 *  that walked every keystroke would be. Every narrowing returns to the
	 *  first page, because an offset reached under other filters points at
	 *  rows this one never had. */
	function narrow(next: Partial<FileFilters>) {
		void goto(filtersToUrl({ ...filters, offset: 0, ...next }), {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	}

	function turn(offset: number) {
		void goto(filtersToUrl({ ...filters, offset }), { noScroll: false });
	}

	const files = createQuery(() => ({
		queryKey: queryKeys.libraryPage(filtersToQuery(filters)),
		queryFn: () => api.library(filtersToQuery(filters))
	}));

	// The machines the tabs are drawn from. Signed-out machines are left out:
	// the server does not list what a revoked device holds, so a tab for one
	// could only ever answer nothing.
	const devices = createQuery(() => ({
		queryKey: queryKeys.devices,
		queryFn: () => api.devices()
	}));

	type Read =
		| { state: 'pending' }
		| { state: 'unavailable' }
		| { state: 'notKeeping' }
		| { state: 'failed'; detail: string }
		| { state: 'read'; entries: LibraryEntry[]; usage: number; keep: boolean };

	let local = $state<Read>({ state: 'pending' });
	let removing = $state<string | null>(null);
	let asking = $state<string | null>(null);
	let opening = $state<string | null>(null);
	let saving = $state(false);

	// Fenced by a generation counter: a refresh while a slower read is in
	// flight would otherwise land the older answer last.
	let generation = 0;

	async function load() {
		if (invoke === null) {
			local = { state: 'unavailable' };
			return;
		}
		const current = ++generation;
		const [entries, usage, settings] = await Promise.all([
			libraryEntries(invoke),
			libraryUsage(invoke),
			librarySettings(invoke)
		]);
		if (current !== generation) {
			return;
		}
		if (
			entries.kind === 'notKeeping' ||
			usage.kind === 'notKeeping' ||
			settings.kind === 'notKeeping'
		) {
			local = { state: 'notKeeping' };
			return;
		}
		if (entries.kind !== 'ok' || usage.kind !== 'ok' || settings.kind !== 'ok') {
			const failed = [entries, usage, settings].find((answer) => answer.kind === 'refused');
			local = {
				state: 'failed',
				detail: failed?.kind === 'refused' ? failed.detail : 'Your files didn’t load.'
			};
			return;
		}
		local = {
			state: 'read',
			entries: entries.value,
			usage: usage.value,
			keep: settings.value.keep_originals
		};
	}

	$effect(() => {
		void load();
		return () => {
			generation += 1;
		};
	});

	const thisDevice = $derived(machineHere.where.device?.id ?? null);
	const kept = $derived(
		local.state === 'read' ? new Map(local.entries.map((entry) => [entry.hash, entry])) : null
	);
	/** Each machine's last check-in, which is what the Last seen column reads. */
	const seen = $derived(
		new Map((devices.data?.devices ?? []).map((device) => [device.id, device.last_seen_at]))
	);
	const rows = $derived(fileRows(files.data?.files ?? [], { thisDevice, kept, seen }));
	// Re-read with the files, so the ages are written against the read.
	const now = $derived.by(() => {
		void files.dataUpdatedAt;
		return Date.now();
	});
	const shown = $derived(
		pageWindow(
			files.data?.files.length ?? 0,
			files.data?.total ?? 0,
			files.data?.offset ?? filters.offset,
			files.data?.limit ?? PAGE_SIZE
		)
	);

	let localSearch = $state('');
	let localPage = $state(1);
	const localMatches = $derived.by(() => {
		if (local.state !== 'read') return [];
		const query = localSearch.trim().toLowerCase();
		return local.entries.filter(
			(entry) => entry.file_name.toLowerCase().includes(query) || entry.hash.includes(query)
		);
	});
	const localShownPage = $derived(
		Math.min(localPage, Math.max(1, Math.ceil(localMatches.length / PAGE_SIZE)))
	);
	const localShown = $derived(
		localMatches.slice((localShownPage - 1) * PAGE_SIZE, localShownPage * PAGE_SIZE)
	);

	const tabs = $derived([
		{ id: 'all', label: 'All machines', count: null },
		...(devices.data?.devices ?? [])
			.filter((device) => device.revoked_at === null)
			.map((device) => ({
				id: device.id,
				label: device.id === thisDevice ? HERE : device.name,
				count: null
			}))
	]);
	// The bar writes its own selection, so it is held here and kept level with
	// the address: the address is what the filter is read from, and arriving
	// on `?device=` must select that machine's tab rather than "All machines".
	let tab = $state(filtersFromUrl(page.url.searchParams).device ?? 'all');
	$effect(() => {
		tab = filters.device ?? 'all';
	});

	async function setKeep(keep: boolean) {
		saving = true;
		const answer = await setLibrarySettings(invoke, keep);
		saving = false;
		if (answer.kind !== 'ok') {
			toast('error', answer.kind === 'refused' ? answer.detail : NOT_KEEPING_SENTENCE);
			await load();
			return;
		}
		if (local.state === 'read') {
			local = { ...local, keep: answer.value.keep_originals };
		}
	}

	async function remove(hash: string, name: string) {
		if (!confirm(removePrompt(name))) {
			return;
		}
		removing = hash;
		const answer = await libraryRemove(invoke, hash);
		removing = null;
		if (answer.kind !== 'ok') {
			toast('error', answer.kind === 'refused' ? answer.detail : NOT_KEEPING_SENTENCE);
		} else {
			toast('info', `"${name}" was removed from this machine.`);
		}
		await load();
	}

	/** Hand one kept file to another application on this machine. Only ever
	 *  offered for a file this machine itself keeps. */
	async function openHere(hash: string) {
		opening = hash;
		const answer = await libraryOpenExternal(invoke, hash);
		opening = null;
		if (answer.kind !== 'ok') {
			toast(
				'error',
				answer.kind === 'refused'
					? 'This machine couldn’t open that file in another app.'
					: NOT_KEEPING_SENTENCE
			);
		}
	}

	/** Ask this machine to take a copy from whichever other machine holds it.
	 *  The application fetches on its next check-in, directly, and the row
	 *  reads "Waiting for…" until then. */
	async function get(hash: string, name: string) {
		if (thisDevice === null) {
			toast('error', 'This machine isn’t connected yet, so it can’t get files. Try again soon.');
			return;
		}
		if (local.state !== 'read') {
			toast('error', 'This machine’s files haven’t loaded, so it can’t receive files yet.');
			return;
		}
		asking = hash;
		try {
			await api.wantFile(thisDevice, hash);
			toast('info', `"${name}" will be copied to this machine.`);
			await queryClient.invalidateQueries({ queryKey: queryKeys.library });
		} catch (failure) {
			toast('error', failure instanceof ApiFailure ? failure.message : 'The copy wasn’t requested. Try again.');
		} finally {
			asking = null;
		}
	}

	async function cancel(hash: string) {
		if (thisDevice === null) {
			return;
		}
		asking = hash;
		try {
			await api.cancelWantFile(thisDevice, hash);
			await queryClient.invalidateQueries({ queryKey: queryKeys.library });
		} catch (failure) {
			toast('error', failure instanceof ApiFailure ? failure.message : 'The copy wasn’t cancelled. Try again.');
		} finally {
			asking = null;
		}
	}
</script>

<div class="page resources-page files-page">
	<PageHead icon="files" title="Your machines' files" guide="your-files">
		{#snippet aside()}
			<Explain title="What “on your computer” means" label="Where are they?">
				<p>{FILES_STAY_ON_YOUR_MACHINES}</p>
				<p>
					The Teachouse app on each of your computers keeps the files you import there. This page
					shows which computer holds each file and which resources use it.
				</p>
				<p>A computer that is off can’t hand its files over. Copy a file to this machine to use it here.</p>
				<p>{BROWSER_SENTENCE}</p>
			</Explain>
		{/snippet}
	</PageHead>

	<p class="flow-hint res-files-hint">Every file your resources use, and the machine it is on.</p>

	<div class="res-bar files-bar">
		<label class="res-search">
			<Icon name="search" size={16} />
			<span class="sr-only">Search</span>
			<input
				id="file-search"
				type="search"
				placeholder="Search by file or resource"
				value={box}
				oninput={(event) => {
					box = event.currentTarget.value;
					narrow({ q: event.currentTarget.value.trim() });
				}}
			/>
		</label>

		<!-- The machines as chips. A machine that could not be read has no chip
		     rather than an empty one, so the row never offers a filter that can
		     only answer nothing. -->
		<div class="res-fchips" role="group" aria-label="Machine">
			{#each tabs as one (one.id)}
				<button
					type="button"
					class="res-fchip"
					aria-pressed={tab === one.id}
					onclick={() => {
						tab = one.id;
						narrow({ device: one.id === 'all' ? null : one.id });
					}}
				>
					{#if one.id !== 'all'}<Icon name="laptop" size={14} />{/if}
					{one.label}
				</button>
			{/each}
		</div>

		<span class="files-selects">
			<label class="sr-only" for="file-availability">Where it is</label>
			<select
				class="res-sort"
				id="file-availability"
				value={filters.availability ?? 'any'}
				onchange={(event) =>
					narrow({
						availability:
							event.currentTarget.value === 'any'
								? null
								: (event.currentTarget.value as Availability)
					})}
			>
				<option value="any">Anywhere</option>
				<option value="online">On a machine that is online</option>
				<option value="offline">On a machine that is offline</option>
				<option value="missing">On no machine</option>
			</select>

			<label class="sr-only" for="file-linked">Resources</label>
			<select
				class="res-sort"
				id="file-linked"
				value={filters.linked ?? 'any'}
				onchange={(event) =>
					narrow({
						linked:
							event.currentTarget.value === 'any' ? null : (event.currentTarget.value as Linked)
					})}
			>
				<option value="any">Used or not</option>
				<option value="linked">Used by a resource</option>
				<option value="unlinked">Used by none</option>
			</select>
		</span>
	</div>

	<!-- This machine's own library, which only exists inside the application.
	     A browser is told where files live, behind the Explain, rather than
	     shown an empty one. -->
	{#if local.state === 'notKeeping'}
		<p class="res-note">{NOT_KEEPING_SENTENCE}</p>
	{:else if local.state === 'failed'}
		<p class="res-note">{local.detail}</p>
	{:else if local.state === 'read'}
		<div class="files-keep">
			<Toggle
				label={KEEP_LABEL}
				checked={local.keep}
				disabled={saving}
				onchange={(value) => void setKeep(value)}
			/>
			<p class="res-note">{usageLine(local.entries, local.usage)}</p>
		</div>
		<details class="flow-more files-local">
			<summary>Files saved on this machine ({local.entries.length})</summary>
			<p class="res-note">The filters above don’t apply to this list.</p>
			<Field label="Search saved files" id="local-file-search">
				<input
					id="local-file-search"
					type="search"
					value={localSearch}
					oninput={(event) => {
						localSearch = event.currentTarget.value;
						localPage = 1;
					}}
				/>
			</Field>
			<div class="flow-table-wrap">
				<table class="flow-table files-table">
					<thead>
						<tr><th>File</th><th>Size</th><th><span class="sr-only">Actions</span></th></tr>
					</thead>
					<tbody>
						{#each localShown as entry (entry.hash)}
							<tr>
								<td class="files-name" data-label="File">{entry.file_name}</td>
								<td data-label="Size">{formatBytes(entry.byte_len)}</td>
								<td class="files-acts">
									<Button
										tier="outline"
										small
										disabled={opening === entry.hash}
										onclick={() => void openHere(entry.hash)}
									>
										{opening === entry.hash ? 'Opening…' : 'Open'}
									</Button>
									<Button
										tier="outline"
										danger
										small
										disabled={removing === entry.hash}
										onclick={() => void remove(entry.hash, entry.file_name)}
									>
										{removing === entry.hash ? 'Removing…' : 'Remove from this machine'}
									</Button>
								</td>
							</tr>
						{:else}
							<tr>
								<td colspan="3" class="res-note">
									{localSearch.trim() ? 'No saved file matches this search.' : 'No files are saved here.'}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
			{#if localMatches.length > PAGE_SIZE}
				<Pagination
					page={localShownPage}
					hasNext={localShownPage * PAGE_SIZE < localMatches.length}
					busy={false}
					label="Saved files"
					summary={`${localMatches.length} saved files`}
					onprevious={() => (localPage = localShownPage - 1)}
					onnext={() => (localPage = localShownPage + 1)}
				/>
			{/if}
		</details>
	{/if}

	<div class="res-toolbar">
		<span class="eyebrow">
			{#if files.isPending}
				Loading your files
			{:else if files.isError}
				Couldn’t count your files
			{:else}
				{countSentence(shown)}
			{/if}
		</span>
		{#if filtersActive(filters)}
			<Button
				tier="quiet"
				small
				icon="x"
				onclick={() => {
					box = '';
					void goto(filtersToUrl(EMPTY_FILTERS), { replaceState: true, noScroll: true });
				}}
			>
				Clear filters
			</Button>
		{/if}
	</div>

	{#if files.isPending}
		<p class="res-note">Loading your files…</p>
	{:else if files.isError}
		<Note icon="triangle-alert">
			{files.error instanceof ApiFailure
				? files.error.message
				: 'Your files didn’t load. Reload to try again.'}
		</Note>
	{:else if rows.length === 0}
		<Placeholder
			icon="files"
			headline={shown.hasPrev ? 'No files on this page' : filtersActive(filters) ? 'No file matches those filters' : 'No files yet'}
			body={shown.hasPrev
				? 'Go to the previous page to see earlier files.'
				: filtersActive(filters)
					? 'Widen the search, or choose another machine.'
					: 'Files show up here after an import, or when you add a file to a resource.'}
		/>
	{:else}
		<div class="flow-table-wrap">
			<table class="flow-table files-table">
				<thead>
					<tr>
						<th>File</th>
						<th>Size</th>
						<th>Machine</th>
						<th>Last seen</th>
						<th>Used by</th>
						<th><span class="sr-only">Actions</span></th>
					</tr>
				</thead>
				<tbody>
					{#each rows as row (row.hash)}
						<tr>
							<td class="files-name" data-label="File">
								<span class:res-file-anon={row.anonymous}>{row.name}</span>
							</td>
							<td class="files-num" data-label="Size">{row.size}</td>
							<td data-label="Machine">
								<!-- The dot says whether it can be reached now; the names say
								     where, in the seller's own words for their machines. -->
								<span class="files-where" title={AVAILABILITY_LABEL[row.availability]}>
									<span class="files-dot {row.availability}" aria-hidden="true"></span>
									{row.machines.length === 0 ? 'None' : row.machines.join(', ')}
									<span class="sr-only">({AVAILABILITY_LABEL[row.availability]})</span>
								</span>
								{#if keptLine(row) !== null}<span class="files-sub">{keptLine(row)}</span>{/if}
							</td>
							<td class="files-num" data-label="Last seen">{seenLine(row, now)}</td>
							<td data-label="Used by">
								{#if row.resources.length === 0}
									<span class="files-sub">None</span>
								{:else}
									<span class="files-uses">
										{#each row.resources as resource (resource.id)}
											<a href={`/resources/${resource.id}`}><span class="res-name">{resource.title}</span></a>
										{/each}
									</span>
								{/if}
							</td>
							<td class="files-acts">
								<!-- Only a copy under way needs a sentence: who holds the file
								     is already the Machine column. -->
								{#if row.label.kind === 'waiting' || row.label.kind === 'fetching'}
									<span class="files-sub">{transferSentence(row.label)}</span>
								{/if}
								{#if row.kept !== null}
									<Button
										tier="outline"
										small
										disabled={opening === row.hash}
										reason={opening === row.hash ? 'Opening the file…' : undefined}
										onclick={() => void openHere(row.hash)}
									>
										{opening === row.hash ? 'Opening…' : 'Open'}
									</Button>
									<Button
										tier="outline"
										danger
										small
										disabled={removing === row.hash}
										reason={removing === row.hash ? 'Removing the file…' : undefined}
										onclick={() => void remove(row.hash, row.name)}
									>
										{removing === row.hash ? 'Removing…' : 'Remove'}
									</Button>
								{:else if row.label.kind === 'get'}
									<Button
										tier="outline"
										small
										disabled={asking === row.hash}
										reason={asking === row.hash ? 'Requesting a copy…' : undefined}
										onclick={() => void get(row.hash, row.name)}
									>
										{asking === row.hash ? 'Requesting…' : 'Copy here'}
									</Button>
								{:else if row.label.kind === 'waiting' || row.label.kind === 'fetching'}
									<Button
										tier="quiet"
										small
										disabled={asking === row.hash}
										reason={asking === row.hash ? 'Cancelling the copy…' : undefined}
										onclick={() => void cancel(row.hash)}
									>
										Cancel
									</Button>
								{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
	{#if !files.isPending && !files.isError && (shown.hasPrev || shown.hasNext)}
		<Pagination
			page={Math.ceil((files.data?.offset ?? filters.offset) / (files.data?.limit ?? PAGE_SIZE)) + 1}
			hasNext={shown.hasNext}
			busy={files.isFetching}
			label="Files"
			summary={countSentence(shown)}
			onprevious={() => turn(shown.prevOffset)}
			onnext={() => turn(shown.nextOffset)}
		/>
	{/if}
</div>
