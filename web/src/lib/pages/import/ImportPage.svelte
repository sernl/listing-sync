<script lang="ts">
	import { untrack } from 'svelte';
	import { createQuery } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import {
		ApiFailure,
		api,
		type ConnectionView,
		type ImportRunFilterState,
		type ImportRunHead,
		type ImportRunOrder
	} from '$lib/api';
	import type { InventoryId, Marketplace } from '$lib/generated/vocab';
	import Banner from '$lib/Banner.svelte';
	import { anyConnectionStands } from '$lib/connection-standing';
	import Button from '$lib/Button.svelte';
	import { desktopInvoker, sessionStatusHere, startImportHere, type LocalSessionOutcome } from '$lib/desktop';
	import { agoLabel } from '$lib/elapsed';
	import { entitlementRead, featureOf } from '$lib/entitlement-read';
	import Field from '$lib/Field.svelte';
	import Note from '$lib/Note.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Pagination from '$lib/Pagination.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { saveDocument } from '$lib/pages/export/download';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { FILES_STAY_ON_YOUR_COMPUTER } from '$lib/sync-request';
	import {
		countWord,
		deleteRefusal,
		keptSelection,
		retainedBadge,
		type WorkDeleteOutcome,
		type WorkItem
	} from '$lib/work-delete';
	import WorkDeleteDialog from '$lib/WorkDeleteDialog.svelte';
	import {
		CONNECTIONS_UNREAD,
		CONNECT_HREF,
		CONNECT_LABEL,
		IMPORTS_UNREAD,
		NEEDS_THE_APP,
		NOTHING_CONNECTED,
		NO_IMPORT_YET,
		deviceLine,
		importBlocked,
		importCards,
		importLabel,
		importRows,
		importRunLabel,
		startRefusal,
		standingBadge,
		type ImportRow
	} from './import-view';
	import { fetchTemplate, openBatchFrom, uploadSheet } from './api';
	import { IMPORT_ALREADY_OPEN, batchHref } from './sheet-view';
	import { pageCount, pageSummary } from './run-view';
	import './import.css';
	import './sheet.css';

	// Null until the list has actually been read. An empty array is a seller
	// with no marketplace, which is a claim; not having read the list is not.
	let connections = $state<ConnectionView[] | null>(null);
	let connectionsUnread = $state(false);

	// Both ways in, in one list. A seller who read a shop on Monday and a
	// spreadsheet on Tuesday has made two imports, not one of each: the two
	// tables behind them are ours rather than theirs.
	//
	// One page of that list, cut by the server. `runsTotal` is the whole
	// history under the filter in force, which is what the pager states.
	let runs = $state<ImportRunHead[]>([]);
	let runsTotal = $state(0);
	// What the seller asked for, and what the rows on screen came from. A
	// failed read leaves the old rows up, and they must carry the page they
	// are actually from.
	let runPage = $state(1);
	let shownRunPage = $state(1);
	let runsFailedFor = $state<number | null>(null);
	let runsUnread = $state(false);
	let runsBusy = $state(false);
	let runsLoaded = $state(false);

	/** How many imports one page of the history holds. The server's own
	 *  default, restated so the pager can count pages without a round trip. */
	const RUNS_PER_PAGE = 10;

	let runState = $state<ImportRunFilterState | ''>('');
	let runSource = $state('');
	let runOrder = $state<ImportRunOrder>('newest');

	// The runs still expecting work, read apart from the history page.
	//
	// A card offers "Open this import" for the shop's own open run, and a run
	// that had scrolled off page one used to be invisible to that lookup —
	// which would offer a seller a second import of a shop they are already
	// importing. This asks the server for open runs rather than searching
	// whatever page happens to be on screen.
	let openRuns = $state<ImportRunHead[]>([]);

	let runsRead = 0;
	let openRead = 0;
	let connectionRead = 0;
	let localRead = 0;
	let localSessions = $state<Map<Marketplace, LocalSessionOutcome>>(new Map());
	// The open batch, so the spreadsheet card's upload control states why it
	// is unavailable rather than losing its button. The server's own answer
	// rather than a search of a list.
	let openBatch = $state<string | null>(null);
	let sheetRefusal = $state<string | null>(null);
	let sending = $state(false);
	let downloading = $state(false);

	let starting = $state<Set<InventoryId>>(new Set());
	// The application's own words when it declines, kept apart from the read
	// failures above: one is this page failing to read something, the other is
	// this computer declining to run something, and they are different facts.
	let declined = $state<string | null>(null);
	// Set when the run was created and the reading could not be started from
	// here — in a browser, or by an application that refused. The run exists
	// and must stay reachable, so the page offers it.
	let raised = $state<string | null>(null);

	const base = $props.id();
	const sheetInputId = `${base}-sheet`;

	// Read once: whether this console is running inside the desktop
	// application does not change while the page is open.
	const invoke = desktopInvoker();
	const inApp = invoke !== null;

	const cards = $derived(importCards(connections));

	// The two ways in are two capabilities, and a plan can carry one without
	// the other: the free plan reads a spreadsheet and cannot read a shop.
	// Each control states its own refusal rather than the page stating one for
	// both.
	const plan = createQuery(() => entitlementRead);
	const uploadRefusal = $derived(featureOf(plan.data, 'import_spreadsheet'));
	const shopRefusal = $derived(featureOf(plan.data, 'import_marketplace'));

	/** Whether the seller has no marketplace connected at all.
	 *
	 *  Over every connection rather than over this screen's own cards, because
	 *  the sentence claims about all of them.
	 *
	 *  Only once the list has been read: a null list is not a seller with no
	 *  shop, and raising this on it would tell them there is nothing when all
	 *  we know is that we could not ask. */
	const nothingHeld = $derived(
		!connectionsUnread && connections !== null && !anyConnectionStands(connections)
	);
	const rows = $derived(importRows(runs));
	const runPages = $derived(pageCount(runsTotal, RUNS_PER_PAGE));
	const runsFiltered = $derived(runState !== '' || runSource !== '' || runOrder !== 'newest');

	/** What the seller has ticked in the history: the run's id against the
	 *  words a confirmation names it by.
	 *
	 *  A map rather than a set of ids, because a selection survives turning
	 *  the page and a row that has scrolled out of the page in hand still has
	 *  to be nameable by the dialog that is about to delete it. */
	let picked = $state<Map<string, string>>(new Map());
	/** The runs a Delete is being confirmed for, or null while none is. One
	 *  dialog for a row's own control and for the selection, because the
	 *  sentence a seller has to read is the same sentence. */
	let deleting = $state<WorkItem[] | null>(null);

	const IMPORTS = { one: 'import', many: 'imports' };
	// The rows on this page a Delete could still act on. A run already
	// stopping or kept for review is not one of them: the server would accept
	// the call and nothing the seller can see would change.
	const pickable = $derived(rows.filter((row) => row.deletion === null));
	const allPickedHere = $derived(
		pickable.length > 0 && pickable.every((row) => picked.has(row.id))
	);
	const pickedItems = $derived([...picked].map(([id, label]) => ({ id, label })));
	// How many ticks are not on the page in hand, said plainly: a figure that
	// only counted the visible rows would make a selection look lost the
	// moment the page turned.
	const pickedElsewhere = $derived(
		[...picked.keys()].filter((id) => !rows.some((row) => row.id === id)).length
	);

	function pick(row: ImportRow, on: boolean) {
		const next = new Map(picked);
		if (on) {
			next.set(row.id, importRunLabel(row, Date.now()));
		} else {
			next.delete(row.id);
		}
		picked = next;
	}

	/** Tick or untick the page in hand, leaving every other page's ticks
	 *  alone: "all of these" and "all of my imports" are different
	 *  sentences. */
	function pickPage() {
		const next = new Map(picked);
		for (const row of pickable) {
			if (allPickedHere) {
				next.delete(row.id);
			} else {
				next.set(row.id, importRunLabel(row, Date.now()));
			}
		}
		picked = next;
	}

	/** What one Delete settled. The rows the server answered about are
	 *  unticked and the refusals stay ticked, so a half-finished bulk is one
	 *  press from being finished rather than a selection to rebuild by hand.
	 *  Both reads are refreshed: a stopped run leaves the open-run list as
	 *  well as the history page. */
	function settled(outcome: WorkDeleteOutcome) {
		const kept = keptSelection(new Set(picked.keys()), outcome);
		picked = new Map([...picked].filter(([id]) => kept.has(id)));
		void loadRuns();
		void loadOpenRuns();
	}
	let startEpoch = 0;

	// Mount-time work, untracked on purpose. `loadRuns` reads the page and the
	// filters, so a tracked body would make this effect a dependent of them:
	// turning a page would then tear the listeners down, bump `startEpoch` and
	// invalidate a start that was in flight. This effect is about the page
	// being open, and nothing else.
	$effect(() => {
		return untrack(() => {
			startEpoch += 1;
			const refresh = () => {
				void loadConnections();
				void loadLocalSessions();
				// The page the seller is on, under the filters they set. A
				// background refresh that reset to page one would move the history
				// out from under someone reading it.
				void loadRuns();
				void loadOpenRuns();
			};
			const visible = () => { if (document.visibilityState === 'visible') refresh(); };
			refresh();
			void loadOpenBatch();
			window.addEventListener('focus', refresh);
			document.addEventListener('visibilitychange', visible);
			return () => {
				startEpoch += 1;
				runsRead += 1;
				openRead += 1;
				connectionRead += 1;
				localRead += 1;
				window.removeEventListener('focus', refresh);
				document.removeEventListener('visibilitychange', visible);
			};
		});
	});

	async function loadConnections() {
		const current = ++connectionRead;
		try {
			const held = await api.connections();
			if (current !== connectionRead) return;
			connections = held;
			connectionsUnread = false;
		} catch {
			if (current !== connectionRead) return;
			connectionsUnread = true;
		}
	}

	async function loadLocalSessions() {
		const current = ++localRead;
		const answers = await Promise.all(importCards(null).map(async (card) => ({
			marketplace: card.marketplace,
			outcome: await sessionStatusHere(invoke, card.marketplace)
		})));
		if (current !== localRead) return;
		localSessions = new Map(answers.map((answer) => [answer.marketplace, answer.outcome]));
	}

	async function loadRuns() {
		const current = ++runsRead;
		const asked = runPage;
		runsBusy = true;
		try {
			const answer = await api.importRuns({
				offset: (asked - 1) * RUNS_PER_PAGE,
				limit: RUNS_PER_PAGE,
				state: runState === '' ? null : runState,
				source: runSource === '' ? null : runSource,
				order: runOrder
			});
			if (current !== runsRead) return;
			runs = answer.runs;
			runsTotal = answer.total;
			shownRunPage = asked;
			runsUnread = false;
			runsFailedFor = null;
		} catch {
			if (current !== runsRead) return;
			// The page already drawn stays, and so does the page number under
			// it: a failed read is not an empty history, and rows from page
			// one must not be labelled page two because page two was asked
			// for and never arrived.
			runsUnread = true;
			runsFailedFor = asked;
		} finally {
			if (current === runsRead) runsBusy = false;
		}
		runsLoaded = true;
	}

	/** The runs still expecting work, whatever page of the history they are
	 *  on. Read apart so a card's "Open this import" finds the shop's open
	 *  run rather than only the ten most recent. */
	async function loadOpenRuns() {
		const current = ++openRead;
		try {
			const answer = await api.importRuns({ state: 'open', limit: 50 });
			if (current !== openRead) return;
			openRuns = answer.runs;
		} catch {
			// Left as it was: a failed read must not turn an open import into
			// an offer to start a second one.
		}
	}

	function showRuns(next: number) {
		runPage = Math.min(Math.max(1, next), runPages);
		void loadRuns();
	}

	function narrowRuns() {
		runPage = 1;
		void loadRuns();
	}

	function clearRunFilters() {
		runState = '';
		runSource = '';
		runOrder = 'newest';
		narrowRuns();
	}

	async function loadOpenBatch() {
		try {
			openBatch = (await api.imports()).open;
		} catch {
			openBatch = null;
		}
	}

	function sheetRefusalOf(failure: unknown): string {
		if (!(failure instanceof ApiFailure)) {
			return 'That did not reach us. Nothing was uploaded.';
		}
		return failure.message;
	}

	async function downloadTemplate() {
		if (downloading) {
			return;
		}
		downloading = true;
		sheetRefusal = null;
		try {
			saveDocument(await fetchTemplate());
		} catch (failure) {
			sheetRefusal = sheetRefusalOf(failure);
		} finally {
			downloading = false;
		}
	}

	async function sheetChosen(event: Event & { currentTarget: HTMLInputElement }) {
		const file = event.currentTarget.files?.[0];
		event.currentTarget.value = '';
		if (file === undefined || sending) {
			return;
		}
		sending = true;
		sheetRefusal = null;
		try {
			// The key is the batch's identity on the server rather than a token
			// beside it. One is minted per submit, so a second submit is a
			// second batch — which the server refuses while one is open, and
			// the refusal names the one that is.
			const parsed = await uploadSheet(file, crypto.randomUUID());
			await goto(batchHref(parsed.id));
		} catch (failure) {
			// A refusal naming the batch already open is the ordinary case, and
			// the card takes the seller to it rather than telling them to look.
			openBatch = openBatchFrom(failure) ?? openBatch;
			sheetRefusal = sheetRefusalOf(failure);
		} finally {
			sending = false;
			await loadOpenBatch();
		}
	}

	/** Raise a run, then ask this computer to read the shop into it.
	 *
	 *  Two acts in one press, in that order: the run is the row the server,
	 *  this page and the application all address, so it exists before anyone
	 *  is asked to fill it. A computer that then declines leaves a run the
	 *  seller can open and carry on from, which is why the identifier is kept
	 *  rather than discarded with the refusal. */
	async function startImport(inventory: InventoryId) {
		if (starting.has(inventory)) return;
		const epoch = startEpoch;
		const href = page.url.href;
		const org = page.data.session?.org;
		const retryOf = page.url.searchParams.get('source') === inventory
			? page.url.searchParams.get('retry') : null;
		const active = () => epoch === startEpoch && href === page.url.href
			&& org === page.data.session?.org;
		starting = new Set([...starting, inventory]);
		declined = null;
		raised = null;
		try {
			if (org === undefined) {
				declined = 'Your account could not be read. Reload before starting.';
				return;
			}
			const card = cards.find((candidate) => candidate.sites.includes(inventory));
			if (card === undefined) return;
			const local = await sessionStatusHere(invoke, card.marketplace);
			if (!active()) return;
			localSessions = new Map(localSessions).set(card.marketplace, local);
			const blocked = importBlocked(card, shopRefusal, local, inApp);
			if (blocked !== null) { declined = blocked; return; }
			const intent = `teachouse.import.intent:${org}:${inventory}:${retryOf ?? 'new'}`;
			const startKey = localStorage.getItem(intent) ?? crypto.randomUUID();
			localStorage.setItem(intent, startKey);
			const run = await api.createImportRun(inventory, startKey, retryOf);
			if (active()) raised = run.id;
			if (!['complete', 'failed', 'abandoned'].includes(run.state) && inApp) {
				const outcome = await startImportHere(invoke, run.id);
				const refused = startRefusal(outcome);
				if (refused !== null || outcome.kind === 'unavailable') {
					if (active()) declined = refused ?? NEEDS_THE_APP;
					return;
				}
			}
			if (active()) await goto(`/imports/runs/${run.id}`);
			if (localStorage.getItem(intent) === startKey) localStorage.removeItem(intent);
		} catch (failure) {
			if (active()) {
				declined = failure instanceof ApiFailure
					? failure.message
					: 'The start request was not acknowledged. Retry here to reconcile the same attempt.';
			}
		} finally {
			starting = new Set([...starting].filter((source) => source !== inventory));
			if (active()) await loadRuns();
		}
	}
</script>

<div class="page">
	<PageHead
		icon="download"
		title="Import"
		description="Bring your current portfolio to Teachouse from anywhere it is housed."
		guide="importing"
	/>

	{#if connectionsUnread}
		<Banner tone="bad" title="We could not read your marketplaces">{CONNECTIONS_UNREAD}</Banner>
	{:else if nothingHeld}
		<Banner tone="info" title="No marketplace connection is recorded" action={toMarketplaces}>
			{NOTHING_CONNECTED}
		</Banner>
	{/if}

	<section class="sh-card">
		<div class="head">
			<h2>Import from a spreadsheet</h2>
			<span class="badges"><StatusPill tone="flat" label="Read on our server" /></span>
		</div>
		<p>One row per resource in our template, checked before anything is created.</p>
		<ol class="sh-steps">
			<li>Download the template and fill in one row for each resource.</li>
			<li>Upload it and read the report before anything is created.</li>
			<li>Add the files your rows named, then import.</li>
		</ol>

		{#if sheetRefusal !== null}
			<Banner tone="bad" title="Your sheet was not accepted">{sheetRefusal}</Banner>
		{/if}

		<div class="sh-acts">
			<Button
				icon="file-down"
				disabled={downloading}
				reason={downloading ? 'Building the template.' : undefined}
				onclick={() => void downloadTemplate()}
			>
				{downloading ? 'Building the template…' : 'Download the template'}
			</Button>
			{#if uploadRefusal !== null}
				<Button disabled reason={uploadRefusal}>Upload a filled sheet</Button>
			{:else if openBatch === null}
				<!-- A label rather than a Button, because the control has to be the
				     file input's own: a button that then clicks a hidden input is a
				     second control the keyboard reaches separately. -->
				<label class="btn sh-pick" for={sheetInputId}>
					{sending ? 'Reading your sheet…' : 'Upload a filled sheet'}
					<input
						id={sheetInputId}
						type="file"
						accept=".xlsx,.csv"
						disabled={sending}
						onchange={sheetChosen}
					/>
				</label>
			{:else}
				<Button disabled reason={IMPORT_ALREADY_OPEN}>Upload a filled sheet</Button>
				<Button tier="primary" href={batchHref(openBatch)}>Open the import you have</Button>
			{/if}
		</div>
	</section>

	{#if declined !== null}
		<Banner tone="bad" title="The reading did not start">
			{declined}
			{#snippet action()}
				{#if raised !== null}
					<Button tier="outline" small href={`/imports/runs/${raised}`}>
						Open the import
					</Button>
				{/if}
			{/snippet}
		</Banner>
	{/if}

	<div class="import-cards">
		{#each cards as card (card.marketplace)}
			{@const site = card.sites[0]}
			{@const local = localSessions.get(card.marketplace)}
			{@const blocked = importBlocked(card, shopRefusal, local, inApp)}
			<!-- From the open-run read rather than from the history page: an
			     open import that has scrolled off page one is still open. -->
			{@const open = openRuns.find((run) => run.source === site)}
			<section class="import-card">
				<div class="head">
					<h2><MarketplaceMark marketplace={card.marketplace} size={22} /></h2>
					<span class="badges">
						{#if card.unreadable === null}
							{@const badge = standingBadge(card)}
							<StatusPill tone={badge.tone} label={badge.label} />
						{/if}
						<StatusPill
							tone={!inApp ? 'soon' : local?.kind === 'known' ? local.connected ? 'ok' : 'warn' : 'soon'}
							label={!inApp ? 'Waiting for the app' : local?.kind === 'known' ? local.connected ? 'Login on this device' : 'Not signed in here' : 'This device: not known'}
						/>
					</span>
				</div>

				{#if card.unreadable !== null}
					<p class="why">{card.unreadable}</p>
				{:else}
					<p class="quiet">{deviceLine(card)}</p>

					{#if local?.kind === 'known' && !local.connected}
						<Banner tone="warn">
							{card.name} is not signed in on this device.
							{#snippet action()}
								<Button href={CONNECT_HREF}>{CONNECT_LABEL}</Button>
							{/snippet}
						</Banner>
					{/if}

					{#if !inApp}
						<p class="why">{NEEDS_THE_APP}</p>
					{/if}

					<div class="actions">
						{#if open !== undefined}
							<Button tier="primary" href={`/imports/runs/${open.id}`}>Open this import</Button>
						{:else if blocked === null && site !== undefined}
							<Button
								tier="primary"
								icon="download"
								disabled={starting.has(site)}
								reason={starting.has(site) ? 'This import is starting.' : undefined}
								onclick={() => void startImport(site)}
							>
								{starting.has(site) ? 'Starting…' : importLabel(card)}
							</Button>
						{:else}
							<!-- Disabled rather than absent, so the card still shows what
							     the seller would do here and says what stands in the way. -->
							<Button tier="primary" icon="download" disabled reason={blocked ?? undefined}>
								{importLabel(card)}
							</Button>
						{/if}
					</div>
				{/if}
			</section>
		{/each}
	</div>

	<div class="import-files-note">
		<Note icon="lock">{FILES_STAY_ON_YOUR_COMPUTER}</Note>
		<Button tier="outline" small icon="library-big" href="/resources/files">
			See which files are on this computer
		</Button>
	</div>

	<Panel title="Your imports" description="Every import you have run.">
		<div class="import-filters">
			<Field label="Where from" id="imports-source">
				<select id="imports-source" bind:value={runSource} onchange={narrowRuns}>
					<option value="">Anywhere</option>
					<option value="spreadsheet">Spreadsheet</option>
					{#each cards as card (card.marketplace)}
						{#each card.sites as site (site)}
							<option value={site}>{card.name}</option>
						{/each}
					{/each}
				</select>
			</Field>
			<Field label="Status" id="imports-state">
				<select id="imports-state" bind:value={runState} onchange={narrowRuns}>
					<option value="">Any status</option>
					<option value="open">Still running</option>
					<option value="reviewing">Waiting on you</option>
					<option value="complete">Finished</option>
					<option value="failed">Stopped with a problem</option>
					<option value="abandoned">Given up</option>
				</select>
			</Field>
			<Field label="Order by" id="imports-order">
				<select id="imports-order" bind:value={runOrder} onchange={narrowRuns}>
					<option value="newest">Newest first</option>
					<option value="oldest">Oldest first</option>
				</select>
			</Field>
			{#if runsFiltered}
				<Button small tier="quiet" onclick={clearRunFilters}>Clear filters</Button>
			{/if}
		</div>

		<!-- A failed read is a banner over the history, not instead of it: the
		     rows already read are still true, and replacing them with a
		     sentence would take away what the seller came for. -->
		{#if runsUnread}
			<Banner tone="bad" title="We could not read your imports">
				{IMPORTS_UNREAD}
				{runsFailedFor === null
					? ''
					: `Page ${runsFailedFor} did not arrive; below is the page we last read.`}
				{#snippet action()}
					<Button
						tier="outline"
						small
						disabled={runsBusy}
						reason={runsBusy ? 'Reading your imports.' : undefined}
						onclick={() => void loadRuns()}
					>
						Try again
					</Button>
				{/snippet}
			</Banner>
		{/if}
		{#if !runsLoaded}
			<p class="quiet">Loading…</p>
		{:else if rows.length === 0}
			<!-- "Nothing matches what you asked for", "there is nothing left on
			     this page" and "you have never run an import" are three
			     different facts. A seller who filtered, or who deleted the last
			     run on page three, must not be told they have no imports. -->
			{#if runsFiltered}
				<p class="quiet">No import matches these filters. Clear them to see the rest.</p>
			{:else if shownRunPage > 1}
				<p class="quiet">
					There is nothing left on this page. Go back for the imports before it.
				</p>
			{:else}
				<Placeholder
					icon="download"
					headline={NO_IMPORT_YET}
					body="Upload a spreadsheet or choose a marketplace above to start one."
				/>
			{/if}
		{:else}
			<!-- The tick for the page in hand and whatever the seller has ticked
			     elsewhere. Above the rows rather than floating over them,
			     because on a phone a bar pinned to the bottom of the viewport
			     covers the row it is about to act on. -->
			<div class="work-bar">
				<label class="work-pick-all">
					<input
						type="checkbox"
						checked={allPickedHere}
						disabled={pickable.length === 0}
						onchange={pickPage}
					/>
					Select the {countWord(pickable.length, IMPORTS)} on this page
				</label>
				{#if picked.size > 0}
					<span class="work-picked">
						{countWord(picked.size, IMPORTS)} selected{pickedElsewhere > 0
							? `, ${pickedElsewhere} of them on another page`
							: ''}
					</span>
					<div class="work-bar-acts">
						<Button small tier="quiet" onclick={() => (picked = new Map())}>
							Clear selection
						</Button>
						<Button small danger onclick={() => (deleting = pickedItems)}>
							Delete {countWord(picked.size, IMPORTS)}
						</Button>
					</div>
				{/if}
			</div>

			{#each rows as row (row.id)}
				{@const going = retainedBadge(row.deletion)}
				{@const refusal = deleteRefusal(row.deletion)}
				<!-- A row rather than one whole-row anchor: it carries a tick and
				     a Delete, and a control nested inside a link is reached by
				     the keyboard as part of the link and activates both. The
				     title is the link, which is what the seller is aiming at. -->
				<div class="import-row">
					<span class="pick">
						<input
							type="checkbox"
							checked={picked.has(row.id)}
							disabled={refusal !== null}
							title={refusal ?? undefined}
							aria-label={`Select the import ${importRunLabel(row, Date.now())}`}
							onchange={(event) => pick(row, event.currentTarget.checked)}
						/>
					</span>
					<span class="who">
						<a class="t" href={row.href}>
							{#if row.source !== null}
								<MarketplaceMark inventory={row.source} />
							{/if}
							{row.name}
						</a>
						<span class="w">{row.line}</span>
					</span>
					<!-- Its own grid column of its natural width. It used to sit in
					     a fixed 152px slot, which on a wide screen was narrower
					     than the longest stage word and ran the label under the
					     shop's name. -->
					<span class="mark">
						<StatusPill tone={going?.tone ?? row.tone} label={going?.label ?? row.label} />
					</span>
					<span class="at">{agoLabel(row.created_at, Date.now())}</span>
					<span class="act">
						<Button
							small
							danger
							disabled={refusal !== null}
							reason={refusal ?? undefined}
							onclick={() =>
								(deleting = [{ id: row.id, label: importRunLabel(row, Date.now()) }])}
						>
							Delete
						</Button>
					</span>
				</div>
			{/each}
		{/if}

		{#if runsLoaded && (runsTotal > 0 || shownRunPage > 1)}
			<Pagination
				page={shownRunPage}
				hasNext={shownRunPage < runPages}
				busy={runsBusy}
				label="Your imports"
				summary={`${pageSummary((shownRunPage - 1) * RUNS_PER_PAGE, rows.length, runsTotal, 'imports')} · Page ${shownRunPage} of ${runPages}`}
				onprevious={() => showRuns(shownRunPage - 1)}
				onnext={() => showRuns(shownRunPage + 1)}
			/>
		{/if}
	</Panel>
</div>

{#snippet toMarketplaces()}
	<Button tier="outline" small href={CONNECT_HREF}>{CONNECT_LABEL}</Button>
{/snippet}

<!-- One dialog for a row's own Delete and for the selection's: the sentence
     a seller has to read is the same sentence, and two dialogs is where the
     two of them drift apart. -->
<WorkDeleteDialog
	open={deleting !== null}
	items={deleting ?? []}
	noun={IMPORTS}
	remove={api.deleteImportRun}
	onClose={() => (deleting = null)}
	onsettled={settled}
/>
