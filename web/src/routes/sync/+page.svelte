<script lang="ts">
	import { goto } from '$app/navigation';
	import {
		ApiFailure,
		api,
		type ConnectionView,
		type JobHead,
		type SyncRequestHead
	} from '$lib/api';
	import { APP_TOO_OLD, desktopInvoker, startImportHere } from '$lib/desktop';
	import { agoLabel } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { MARKETPLACE_NAME, platformTitle } from '$lib/platforms';
	import {
		AUTHORSHIP_FIRST,
		AUTHORSHIP_HREF,
		headStage,
		listRowLine,
		mayMigrate,
		migrateBody,
		migrateSource,
		presentStage,
		siteChoiceQuestion,
		siteLabel,
		sourcesOn,
		targetAuthorship
	} from '$lib/sync-request';
	import type { InventoryId } from '$lib/generated/vocab';

	let jobs = $state<JobHead[]>([]);
	let nextCursor = $state<string | null>(null);
	let loaded = $state(false);
	// The third of the three reads in this page's one effect, and until now the
	// only one that let a failure escape: it left the panel on "Loading…" for
	// ever with an unhandled rejection behind it. A run list that could not be
	// read is not a seller who has never synced, exactly as an unread import
	// list is not a seller with no imports.
	let jobsUnread = $state(false);

	let imports = $state<SyncRequestHead[]>([]);
	let importsLoaded = $state(false);
	// An import list that could not be read is not a seller who has brought no
	// shop across. Told the second when the first is true, they start the
	// migration again -- so this read is given the same three states as the
	// connections read below it, rather than collapsing a failure into an
	// empty panel that says nothing at all.
	let importsUnread = $state(false);

	let connections = $state<ConnectionView[]>([]);
	// A marketplace list that could not be read is not a seller with no
	// marketplace: the panel says which of the two it is looking at rather than
	// hiding the action and letting an outage read as "you have no shop".
	let connectionsUnread = $state(false);
	let choosing = $state(false);
	let starting = $state(false);
	let refusal = $state<string | null>(null);
	let source = $state<InventoryId | null>(null);
	// Set only when the request was created and this computer then declined to
	// run it. The request exists and must stay reachable, so the panel offers it
	// rather than leaving the seller on a refusal with nowhere to go.
	let created = $state<string | null>(null);

	// Read once: whether this console runs inside the desktop application does
	// not change while the page is open.
	const invoke = desktopInvoker();

	// One key per site, minted once: the server takes the idempotency key as the
	// request's own identity, so a retry after a failed submit reaches the same
	// request rather than starting a second import of the same shop -- and a
	// seller who corrects the site before retrying gets a new request rather
	// than a replay of the one that names the shop they did not mean.
	let keys: Record<string, string> = {};

	const from = $derived(migrateSource(connections));
	const sites = $derived(from === null ? [] : sourcesOn(from));
	// The console's own gate, and until the server refuses one of its own it is
	// the only gate: a migrate submitted with no declaration on record settles
	// every listing it creates failed and terminal, and declaring afterwards
	// brings none of them back.
	const declared = $derived(mayMigrate(targetAuthorship(connections)));

	async function loadPage(cursor?: string | null) {
		try {
			const view = await api.jobs(cursor);
			jobs = [...jobs, ...view.jobs];
			nextCursor = view.next_cursor;
			jobsUnread = false;
		} catch {
			// The rows already held stay held: a page that failed to load its
			// second page has still read its first, and throwing those away
			// would turn one unreadable page into an empty history.
			jobsUnread = true;
		}
		loaded = true;
	}

	async function loadImports() {
		try {
			const view = await api.syncRequests();
			imports = view.requests.filter((row) => row.disposition === 'migrate');
			importsUnread = false;
		} catch {
			imports = [];
			importsUnread = true;
		}
		importsLoaded = true;
	}

	$effect(() => {
		void loadPage();
		void loadImports();
		void api
			.connections()
			.then((view) => {
				connections = view.connections;
				connectionsUnread = false;
			})
			.catch(() => {
				connections = [];
				connectionsUnread = true;
			});
	});

	// The site is not remembered between visits, and Cancel discards it: a
	// seller on the New Zealand site re-picks every time. Deliberate for now
	// rather than overlooked — nothing on the wire records which site is theirs,
	// so remembering it would mean this console inventing a preference it has no
	// way to confirm. Revisit when a device reports its own site.
	function choose() {
		choosing = true;
		refusal = null;
		created = null;
		source = sites[0] ?? null;
	}

	async function start() {
		if (source === null) {
			return;
		}
		const key = (keys[source] ??= crypto.randomUUID());
		starting = true;
		refusal = null;
		created = null;
		try {
			const ack = await api.createSyncRequest(migrateBody(source), key);
			// Inside the application the seller asked for the import with one
			// click, so the request is created and this computer is asked to run
			// it without a second one. In a browser there is nothing here to ask,
			// and the request's own page says where the work happens.
			if (invoke !== null) {
				const outcome = await startImportHere(invoke, ack.request);
				if (outcome.kind === 'refused' || outcome.kind === 'unsupported') {
					created = ack.request;
					refusal = outcome.kind === 'unsupported' ? APP_TOO_OLD : outcome.detail;
					return;
				}
			}
			await goto(`/sync/requests/${ack.request}`);
		} catch (caught) {
			refusal =
				caught instanceof ApiFailure ? caught.message : 'The import could not be started.';
		} finally {
			starting = false;
		}
	}
</script>

<div class="page">
	<PageHead
		icon="refresh-cw"
		title="Sync"
		description="Queued and completed runs, with live progress while one is under way."
	/>

	{#if connectionsUnread}
		<Panel title="Bring a shop across">
			<p class="quiet">
				Your marketplaces could not be read, so this page cannot say whether you have a shop
				to bring across. Nothing has been started.
			</p>
		</Panel>
	{:else if from !== null}
		<Panel
			title="Bring a shop across"
			description="Your own machine reads the shop under your own session and sends us what it finds. Nothing is published: everything arrives as a draft for you to review."
		>
			{#if !declared}
				<p class="s">{AUTHORSHIP_FIRST}</p>
				<div class="actions">
					<a class="cta" href={AUTHORSHIP_HREF}>Declare authorship for TPT</a>
				</div>
			{:else if !choosing}
				<div class="actions">
					<button class="cta" type="button" onclick={choose}>
						Import from {MARKETPLACE_NAME[from]}
					</button>
				</div>
			{:else}
				<p class="s">{siteChoiceQuestion(sites)}</p>
				<div class="inline-choices">
					{#each sites as site (site)}
						<label title={platformTitle(site)}>
							<input
								type="radio"
								name="migrate-source"
								disabled={starting}
								checked={source === site}
								onchange={() => (source = site)}
							/>
							{siteLabel(site)}
						</label>
					{/each}
				</div>
				<div class="actions">
					<button
						class="btn"
						type="button"
						disabled={starting}
						onclick={() => (choosing = false)}
					>
						Cancel
					</button>
					<button
						class="cta"
						type="button"
						disabled={starting || source === null}
						onclick={() => void start()}
					>
						{starting ? 'Starting…' : 'Start the import'}
					</button>
				</div>
			{/if}
			{#if refusal !== null}
				<p class="refusal">{refusal}</p>
			{/if}
			{#if created !== null}
				<div class="actions">
					<a class="btn" href={`/sync/requests/${created}`}>Open the import</a>
				</div>
			{/if}
		</Panel>
	{/if}

	{#if importsUnread}
		<Panel title="Your imports">
			<p class="quiet">
				Your imports could not be read, so this page cannot list them. Any import you have
				already started is still running; nothing here has changed it.
			</p>
		</Panel>
	{:else if importsLoaded && imports.length > 0}
		<Panel
			title="Your imports"
			description="Every shop you have brought across, newest first. An import that is still waiting for your device has no run of its own yet, so this is where it lives."
		>
			{#each imports as row (row.request)}
				{@const stage = headStage(row)}
				{@const shown = presentStage(stage)}
				<a class="job" href={`/sync/requests/${row.request}`}>
					<span class="pill {shown.tone}">{shown.label}</span>
					<span class="what">
						<span class="t">{platformTitle(row.source)} → {platformTitle(row.target)}</span>
						<span class="w">{listRowLine(stage)}</span>
					</span>
					<span class="when">{agoLabel(row.created_at, Date.now())}</span>
				</a>
			{/each}
		</Panel>
	{/if}

	<Panel>
		{#if !loaded}
			<p class="quiet">Loading…</p>
		{:else if jobsUnread && jobs.length === 0}
			<p class="quiet">
				Your runs could not be read, so this page cannot list them. Anything already
				running is unaffected.
			</p>
		{:else if jobs.length === 0}
			<div class="placeholder">
				<span class="big" aria-hidden="true">⇄</span>
				<b>No sync has run yet</b>
				<p>
					Start one from Inventory: choose the items to send, and the engine takes them
					from there. Every run keeps its own record here.
				</p>
			</div>
		{:else}
			{#each jobs as job (job.job)}
				<a class="job" href={`/sync/${job.job}`}>
					<span class="badge">{job.inventory}</span>
					<span class="what">
						<span class="t mono" title={job.job}>{job.job}</span>
						<span class="w">{new Date(job.created_at).toLocaleString()}</span>
					</span>
					<span class="when">{agoLabel(job.created_at, Date.now())}</span>
				</a>
			{/each}
			{#if jobsUnread}
				<p class="quiet">More runs could not be read. The ones above are what loaded.</p>
			{/if}
			{#if nextCursor}
				<div class="actions">
					<button class="btn" onclick={() => void loadPage(nextCursor)}>Load more runs</button>
				</div>
			{/if}
		{/if}
	</Panel>
</div>
