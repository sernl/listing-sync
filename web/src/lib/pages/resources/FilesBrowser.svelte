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
	import Field from '$lib/Field.svelte';
	import Icon from '$lib/Icon.svelte';
	import { machineHere } from '$lib/machine.svelte';
	import Note from '$lib/Note.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Pagination from '$lib/Pagination.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import TabBar from '$lib/TabBar.svelte';
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
		metaLine,
		pageWindow,
		removePrompt,
		transferSentence,
		usageLine
	} from './files-browser';
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
				detail: failed?.kind === 'refused' ? failed.detail : 'The files could not be listed.'
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
	const rows = $derived(fileRows(files.data?.files ?? [], { thisDevice, kept }));
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
					? 'This computer would not hand that file to another application.'
					: NOT_KEEPING_SENTENCE
			);
		}
	}

	/** Ask this machine to take a copy from whichever other machine holds it.
	 *  The application fetches on its next check-in, directly, and the row
	 *  reads "Waiting for…" until then. */
	async function get(hash: string, name: string) {
		if (thisDevice === null) {
			toast('error', 'This machine has not checked in yet, so it cannot ask for a file.');
			return;
		}
		if (local.state !== 'read') {
			toast('error', 'Open this machine’s file library before asking it to receive files.');
			return;
		}
		asking = hash;
		try {
			await api.wantFile(thisDevice, hash);
			toast('info', `"${name}" will be copied to this machine from the one that holds it.`);
			await queryClient.invalidateQueries({ queryKey: queryKeys.library });
		} catch (failure) {
			toast('error', failure instanceof ApiFailure ? failure.message : 'The copy was not asked for.');
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
			toast('error', failure instanceof ApiFailure ? failure.message : 'The copy was not cancelled.');
		} finally {
			asking = null;
		}
	}

	const PILL_TONE: Record<Availability, 'ok' | 'warn' | 'bad'> = {
		online: 'ok',
		offline: 'warn',
		missing: 'bad'
	};
</script>

<div class="page resources-page files-page">
	<PageHead
		icon="files"
		title="Your machines' files"
		description="Every file your machines hold, and the resources that use them."
		guide="your-files"
	/>

	<Panel>
		<div class="res-filters">
			<Field label="Search" id="file-search">
				<span class="res-search">
					<Icon name="search" size={16} />
					<input
						id="file-search"
						type="search"
						placeholder="Search by file, resource or digest"
						value={box}
						oninput={(event) => {
							box = event.currentTarget.value;
							narrow({ q: event.currentTarget.value.trim() });
						}}
					/>
				</span>
			</Field>

			<Field label="Where it is" id="file-availability">
				<select
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
			</Field>

			<Field label="Resources" id="file-linked">
				<select
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
			</Field>
		</div>

		{#if filtersActive(filters)}
			<div class="res-clear">
				<Button
					tier="quiet"
					small
					onclick={() => {
						box = '';
						void goto(filtersToUrl(EMPTY_FILTERS), { replaceState: true, noScroll: true });
					}}
				>
					Clear filters
				</Button>
			</div>
		{/if}
	</Panel>

	<!-- The machines, as tabs. A machine that could not be read has no tab
	     rather than an empty one, so the bar never offers a filter that can
	     only answer nothing. -->
	{#if tabs.length > 1}
		<TabBar
			{tabs}
			bind:current={tab}
			onselect={(id) => narrow({ device: id === 'all' ? null : id })}
		/>
	{/if}

	<!-- This machine's own library, which only exists inside the application.
	     A browser is told where files live rather than shown an empty one. -->
	{#if local.state === 'unavailable'}
		<p class="res-note">{BROWSER_SENTENCE}</p>
	{:else if local.state === 'notKeeping'}
		<p class="res-note">{NOT_KEEPING_SENTENCE}</p>
	{:else if local.state === 'failed'}
		<p class="res-note">{local.detail}</p>
	{:else if local.state === 'read'}
		<Panel>
			<div class="files-keep">
				<Toggle
					label={KEEP_LABEL}
					checked={local.keep}
					disabled={saving}
					onchange={(value) => void setKeep(value)}
				/>
				<p class="res-note">{usageLine(local.entries, local.usage)}</p>
			</div>
			<details class="files-local">
				<summary>Files saved on this machine ({local.entries.length})</summary>
				<Note>Read directly from this app, whatever the filters above say.</Note>
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
				<div class="files-rows">
					{#each localShown as entry (entry.hash)}
						<div class="files-row">
							<span class="files-name">{entry.file_name}</span>
							<p class="res-note">{formatBytes(entry.byte_len)}</p>
							<div class="files-acts">
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
							</div>
						</div>
					{:else}
						<p class="res-note">
							{localSearch.trim() ? 'No saved file matches this search.' : 'No files are saved here.'}
						</p>
					{/each}
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
		</Panel>
	{/if}

	<div class="res-toolbar">
		<span class="eyebrow">
			{#if files.isPending}
				Reading your files
			{:else if files.isError}
				Nothing counted
			{:else}
				{countSentence(shown)}
			{/if}
		</span>
	</div>

	{#if files.isPending}
		<p class="res-note">Reading your files…</p>
	{:else if files.isError}
		<Note icon="triangle-alert">
			{files.error instanceof ApiFailure
				? files.error.message
				: 'The server’s file records could not be listed.'}
		</Note>
	{:else if rows.length === 0}
		<Placeholder
			icon="files"
			headline={shown.hasPrev ? 'No files on this page' : filtersActive(filters) ? 'No file matches those filters' : 'No files yet'}
			body={shown.hasPrev
				? 'Go to the previous page to see earlier files.'
				: filtersActive(filters)
					? 'Widen the search, or choose another machine.'
					: 'Files appear here once one of your machines keeps an import, or a resource carries one.'}
		/>
	{:else}
		<div class="res-rows files-rows">
			{#each rows as row (row.hash)}
				<div class="files-row">
					<div class="files-what">
						<span class="files-name" class:res-file-anon={row.anonymous}>{row.name}</span>
						<StatusPill
							tone={PILL_TONE[row.availability]}
							label={AVAILABILITY_LABEL[row.availability]}
						/>
					</div>
					<p class="res-note">{metaLine(row)}</p>
					{#if row.resources.length > 0}
						<p class="files-uses">
							{#each row.resources as resource, index (resource.id)}
								{#if index > 0}<span class="files-sep">·</span>{/if}
								<a href={`/resources/${resource.id}`}>{resource.title}</a>
							{/each}
						</p>
					{/if}
					<div class="files-acts">
						{#if transferSentence(row.label) !== null}
							<span class="res-note">{transferSentence(row.label)}</span>
						{/if}
						{#if row.kept !== null}
							<Button
								tier="outline"
								small
								disabled={opening === row.hash}
								reason={opening === row.hash ? 'The file is being handed over.' : undefined}
								onclick={() => void openHere(row.hash)}
							>
								{opening === row.hash ? 'Opening…' : 'Open'}
							</Button>
							<Button
								tier="outline"
								danger
								small
								disabled={removing === row.hash}
								reason={removing === row.hash ? 'The file is being removed.' : undefined}
								onclick={() => void remove(row.hash, row.name)}
							>
								{removing === row.hash ? 'Removing…' : 'Remove from this machine'}
							</Button>
						{:else if row.label.kind === 'get'}
							<Button
								tier="outline"
								small
								disabled={asking === row.hash}
								reason={asking === row.hash ? 'The copy is being asked for.' : undefined}
								onclick={() => void get(row.hash, row.name)}
							>
								{asking === row.hash ? 'Asking…' : 'Get on this machine'}
							</Button>
						{:else if row.label.kind === 'waiting' || row.label.kind === 'fetching'}
							<Button
								tier="quiet"
								small
								disabled={asking === row.hash}
								reason={asking === row.hash ? 'The copy is being cancelled.' : undefined}
								onclick={() => void cancel(row.hash)}
							>
								Cancel
							</Button>
						{/if}
					</div>
				</div>
			{/each}
		</div>

		<Note icon="lock">{FILES_STAY_ON_YOUR_MACHINES}</Note>
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
