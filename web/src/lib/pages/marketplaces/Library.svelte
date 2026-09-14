<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import {
		desktopInvoker,
		type LibraryEntry,
		libraryEntries,
		libraryRemove,
		librarySettings,
		libraryUsage,
		setLibrarySettings
	} from '$lib/desktop';
	import { machineHere } from '$lib/machine.svelte';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import Toggle from '$lib/Toggle.svelte';
	import {
		BROWSER_SENTENCE,
		KEEP_LABEL,
		NOT_KEEPING_SENTENCE,
		elsewhereRows,
		libraryRows,
		removePrompt,
		transferSentence,
		usageLine
	} from './library';

	// Read once: whether this console is running inside the application does
	// not change while the page is open.
	const invoke = desktopInvoker();
	const queryClient = useQueryClient();

	type Read =
		| { state: 'pending' }
		| { state: 'unavailable' }
		| { state: 'notKeeping' }
		| { state: 'failed'; detail: string }
		| { state: 'read'; entries: LibraryEntry[]; usage: number; keep: boolean };

	let read = $state<Read>({ state: 'pending' });
	let removing = $state<string | null>(null);
	let asking = $state<string | null>(null);
	let saving = $state(false);

	// The server's account of who holds what, for the rows below the local
	// list: what another machine has that this one could take a copy of.
	// Read in a browser too, where it says which machines hold which files
	// even though nothing can be fetched here.
	const elsewhere = createQuery(() => ({
		queryKey: queryKeys.library,
		queryFn: () => api.library()
	}));

	// Fenced by a generation counter, as the page's own local session read
	// is: a refresh while a slower read is in flight would otherwise land
	// the older answer last.
	let generation = 0;

	async function load() {
		if (invoke === null) {
			read = { state: 'unavailable' };
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
		if (entries.kind === 'notKeeping' || usage.kind === 'notKeeping' || settings.kind === 'notKeeping') {
			read = { state: 'notKeeping' };
			return;
		}
		if (entries.kind !== 'ok' || usage.kind !== 'ok' || settings.kind !== 'ok') {
			const failed = [entries, usage, settings].find((answer) => answer.kind === 'refused');
			read = {
				state: 'failed',
				detail: failed?.kind === 'refused' ? failed.detail : 'The files could not be listed.'
			};
			return;
		}
		read = {
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

	const rows = $derived(read.state === 'read' ? libraryRows(read.entries) : []);
	const heldHere = $derived(
		new Set(read.state === 'read' ? read.entries.map((entry) => entry.hash) : [])
	);
	const thisDevice = $derived(machineHere.where.device?.id ?? null);
	const remote = $derived(
		read.state === 'read' ? elsewhereRows(elsewhere.data?.files ?? [], thisDevice, heldHere) : []
	);

	async function setKeep(keep: boolean) {
		saving = true;
		const answer = await setLibrarySettings(invoke, keep);
		saving = false;
		if (answer.kind !== 'ok') {
			toast('error', answer.kind === 'refused' ? answer.detail : NOT_KEEPING_SENTENCE);
			await load();
			return;
		}
		if (read.state === 'read') {
			read = { ...read, keep: answer.value.keep_originals };
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

	/** Ask this machine to take a copy from whichever other machine holds
	 *  it. The application fetches on its next check-in, directly, and the
	 *  row reads "Waiting for…" until then. */
	async function get(hash: string, name: string) {
		if (thisDevice === null) {
			toast('error', 'This machine has not checked in yet, so it cannot ask for a file.');
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
</script>

<section id="library">
	<div class="mp-sect">
		<h2>Files on this machine</h2>
		<p>
			The originals the Teachouse app read from your marketplaces, kept sealed on this machine
			so you can preview or open them without fetching them again. They never reach our servers,
			and a copy travels straight from one of your machines to another or waits.
		</p>
	</div>

	{#if read.state === 'pending'}
		<p class="quiet">Reading the files on this machine…</p>
	{:else if read.state === 'unavailable'}
		<p class="quiet">{BROWSER_SENTENCE}</p>
	{:else if read.state === 'notKeeping'}
		<p class="quiet">{NOT_KEEPING_SENTENCE}</p>
	{:else if read.state === 'failed'}
		<p class="quiet">{read.detail}</p>
	{:else}
		<div class="mp-card">
			<div class="mp-machine">
				<Toggle
					label={KEEP_LABEL}
					checked={read.keep}
					disabled={saving}
					onchange={(value) => void setKeep(value)}
				/>
				<p class="spec">{usageLine(read.entries, read.usage)}</p>
			</div>
			{#each rows as row (row.hash)}
				<div class="mp-machine">
					<div class="who">
						<span class="t">{row.name}</span>
						<span class="act">
							<Button
								tier="outline"
								danger
								small
								disabled={removing === row.hash}
								reason={removing === row.hash ? 'The file is being removed.' : undefined}
								onclick={() => remove(row.hash, row.name)}
							>
								{removing === row.hash ? 'Removing…' : 'Remove from this machine'}
							</Button>
						</span>
					</div>
					<p class="spec">{row.marketplaceName} · {row.size} · kept {row.kept}</p>
				</div>
			{/each}
			{#each remote as row (row.hash)}
				<div class="mp-machine">
					<div class="who">
						<span class="t">{row.name}</span>
						<span class="act">
							{#if row.label.kind === 'get'}
								<Button
									tier="outline"
									small
									disabled={asking === row.hash}
									reason={asking === row.hash ? 'The copy is being asked for.' : undefined}
									onclick={() => get(row.hash, row.name)}
								>
									{asking === row.hash ? 'Asking…' : 'Get on this machine'}
								</Button>
							{/if}
						</span>
					</div>
					<p class="spec">{row.size} · {transferSentence(row.label)}</p>
				</div>
			{/each}
		</div>
	{/if}
</section>
