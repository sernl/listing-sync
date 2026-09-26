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
	import Explain from '$lib/Explain.svelte';
	import FlowActionBar from '$lib/FlowActionBar.svelte';
	import FlowDiagram, { type FlowEnd } from '$lib/FlowDiagram.svelte';
	import FlowStep from '$lib/FlowStep.svelte';
	import Icon from '$lib/Icon.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Pagination from '$lib/Pagination.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { saveDocument } from '$lib/pages/export/download';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import Stepper, { type StepMark } from '$lib/Stepper.svelte';
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
		type ImportCard,
		type ImportRow,
		type PillTone
	} from './import-view';
	import { fetchTemplate, openBatchFrom, uploadSheet } from './api';
	import { IMPORT_ALREADY_OPEN, batchHref } from './sheet-view';
	import { READING_HAPPENS_ON_YOUR_COMPUTER, pageCount, pageSummary } from './run-view';
	import './import.css';

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
	// The template's own failure, apart from the upload's: the two controls
	// sit in different steps, and each says what went wrong where it was
	// pressed.
	let templateRefusal = $state<string | null>(null);
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
		templateRefusal = null;
		try {
			saveDocument(await fetchTemplate());
		} catch (failure) {
			templateRefusal = sheetRefusalOf(failure);
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
				declined = 'We could not load your account. Reload the page, then start.';
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
					: 'We did not hear back. Try again here and it will pick up the same import.';
			}
		} finally {
			starting = new Set([...starting].filter((source) => source !== inventory));
			if (active()) await loadRuns();
		}
	}

	// ------------------------------------------------------------ the flow

	/** Where an import comes from: a marketplace's card, or the sheet. */
	type Source = Marketplace | 'sheet';

	/** The seller's own pick, or null while they have not made one. */
	let picked_source = $state<Source | null>(null);

	/** Where the page starts before a pick: the shop a retry link names, else
	 *  the first shop the seller has connected, else the sheet — the one way
	 *  in that needs no connection. */
	const firstSource = $derived.by((): Source => {
		const asked = page.url.searchParams.get('source');
		const named = cards.find((card) => asked !== null && card.sites.some((site) => site === asked));
		if (named !== undefined) return named.marketplace;
		const held = cards.find((card) => card.unreadable === null && card.standing === 'held');
		return held?.marketplace ?? 'sheet';
	});
	const source = $derived(picked_source ?? firstSource);
	const chosenCard = $derived(
		source === 'sheet' ? null : (cards.find((card) => card.marketplace === source) ?? null)
	);

	/** The open run for a card's shop, from the open-run read rather than the
	 *  history page: an open import that has scrolled off page one is still
	 *  open. */
	function openRunOf(card: ImportCard): ImportRunHead | undefined {
		const site = card.sites[0];
		return openRuns.find((run) => run.source === site);
	}

	/** The one pill a tile carries: the thing most worth knowing before
	 *  choosing it. */
	function tilePill(card: ImportCard): { tone: PillTone; label: string } {
		if (card.unreadable !== null) return { tone: 'soon', label: 'Not yet' };
		if (openRunOf(card) !== undefined) return { tone: 'run', label: 'Import running' };
		const local = localSessions.get(card.marketplace);
		if (inApp && local?.kind === 'known' && !local.connected) {
			return { tone: 'warn', label: 'Not signed in here' };
		}
		return standingBadge(card);
	}

	const sheetPill = $derived<{ tone: PillTone; label: string }>(
		uploadRefusal !== null
			? { tone: 'soon', label: 'Not on your plan' }
			: openBatch !== null
				? { tone: 'run', label: 'Import open' }
				: { tone: 'flat', label: 'Checked online' }
	);

	/** This device's own sign-in, as the start step states it beside the
	 *  line about keeping the app open. */
	function devicePill(card: ImportCard): { tone: PillTone; label: string } {
		const local = localSessions.get(card.marketplace);
		if (!inApp) return { tone: 'soon', label: 'Waiting for the app' };
		if (local?.kind !== 'known') return { tone: 'soon', label: 'Sign-in here unknown' };
		return local.connected
			? { tone: 'ok', label: 'Signed in on this device' }
			: { tone: 'warn', label: 'Not signed in here' };
	}

	const diagramFrom = $derived<FlowEnd>(
		chosenCard !== null && chosenCard.sites[0] !== undefined
			? { inventory: chosenCard.sites[0] }
			: { icon: 'layout-list', label: 'Your sheet' }
	);
	const CATALOGUE: FlowEnd = { icon: 'library-big', label: 'Catalogue' };

	const startHeld = $derived(
		chosenCard === null ? openBatch !== null : openRunOf(chosenCard) !== undefined
	);
	const sourceWord = $derived(chosenCard?.name ?? 'Spreadsheet');

	const steps = $derived<StepMark[]>([
		{ id: 'where', label: 'Where from', done: true },
		{ id: 'start', label: 'Start', done: startHeld },
		{ id: 'imports', label: 'Your imports', done: false }
	]);

	let whereOpen = $state(true);
	let startOpen = $state(true);
	let importsOpen = $state(true);

	/** The history's filters as chips: the value each sends to the server
	 *  and the word it shows. */
	const sourceChips = $derived([
		{ value: '', label: 'Anywhere' },
		{ value: 'spreadsheet', label: 'Spreadsheet' },
		...cards.flatMap((card) => card.sites.map((site) => ({ value: site as string, label: card.name })))
	]);
	const STATE_CHIPS: readonly { value: ImportRunFilterState | ''; label: string }[] = [
		{ value: '', label: 'Any status' },
		{ value: 'open', label: 'Still running' },
		{ value: 'reviewing', label: 'Waiting on you' },
		{ value: 'complete', label: 'Finished' },
		{ value: 'failed', label: 'Stopped with a problem' },
		{ value: 'abandoned', label: 'Cancelled' }
	];

	function chooseSource(value: string) {
		runSource = value;
		narrowRuns();
	}

	function chooseState(value: ImportRunFilterState | '') {
		runState = value;
		narrowRuns();
	}

	function flipOrder() {
		runOrder = runOrder === 'newest' ? 'oldest' : 'newest';
		narrowRuns();
	}
</script>

<div class="page flow-page has-bar">
	<PageHead
		icon="download"
		title="Import"
		description="Bring your resources into Teachouse."
		guide="importing"
	/>

	<div class="flow">
		{#if connectionsUnread}
			<Banner tone="bad" title="We could not load your marketplaces">{CONNECTIONS_UNREAD}</Banner>
		{:else if nothingHeld}
			<Banner tone="info" title="No marketplace connected yet" action={toMarketplaces}>
				{NOTHING_CONNECTED}
			</Banner>
		{/if}

		<Stepper {steps} label="Import steps" />

		<FlowStep
			n={1}
			id="where"
			title="Where from"
			hint="Choose where your resources are now."
			summary="{sourceWord} → Teachouse catalogue"
			done
			bind:open={whereOpen}
		>
			<div class="imp-tiles" role="radiogroup" aria-label="Where from">
				{#each cards as card (card.marketplace)}
					{@const pill = tilePill(card)}
					<div class="imp-tile" class:on={source === card.marketplace}>
						<button
							type="button"
							role="radio"
							class="imp-tile-pick"
							aria-checked={source === card.marketplace}
							disabled={card.unreadable !== null}
							title={card.unreadable ?? undefined}
							onclick={() => (picked_source = card.marketplace)}
						>
							<span class="imp-tile-mark"><MarketplaceMark marketplace={card.marketplace} size={28} /></span>
							<span class="imp-tile-name">{card.name}</span>
							<StatusPill tone={pill.tone} label={pill.label} />
						</button>
					</div>
				{/each}
				<div class="imp-tile" class:on={source === 'sheet'}>
					<button
						type="button"
						role="radio"
						class="imp-tile-pick"
						aria-checked={source === 'sheet'}
						onclick={() => (picked_source = 'sheet')}
					>
						<span class="imp-tile-mark sheet"><Icon name="layout-list" size={22} /></span>
						<span class="imp-tile-name">Spreadsheet</span>
						<StatusPill tone={sheetPill.tone} label={sheetPill.label} />
					</button>
					<div class="imp-tile-more">
						<Button
							small
							tier="quiet"
							icon="file-down"
							disabled={downloading}
							reason={downloading ? 'Getting the template ready.' : undefined}
							onclick={() => void downloadTemplate()}
						>
							{downloading ? 'Getting it ready…' : 'Download the template'}
						</Button>
					</div>
				</div>
			</div>

			{#if templateRefusal !== null}
				<Banner tone="bad" title="The template did not download">{templateRefusal}</Banner>
			{/if}

			<FlowDiagram
				from={diagramFrom}
				to={[CATALOGUE]}
				rule={chosenCard === null ? 'Checked first' : 'You pick'}
				label="From {sourceWord} into your Teachouse catalogue"
			/>
		</FlowStep>

		<FlowStep
			n={2}
			id="start"
			title="Start"
			hint={chosenCard === null ? 'Upload your filled template.' : `Read your ${chosenCard.name} shop.`}
			summary={startHeld ? 'An import is open' : 'Not started'}
			done={startHeld}
			action={startAction}
			actionInBar
			bind:open={startOpen}
		>
			{#snippet aside()}
				<Explain title="How importing works" label="How importing works">
					{#if chosenCard === null}
						<ol class="imp-explain-steps">
							<li>Download the template and fill in one row for each resource.</li>
							<li>Upload it and read our check. Nothing is added yet.</li>
							<li>Add the files your sheet lists, then import.</li>
						</ol>
					{:else}
						<p>{READING_HAPPENS_ON_YOUR_COMPUTER}</p>
						<p>You choose which resources come in, then confirm before anything is added.</p>
						<p>{NEEDS_THE_APP}</p>
					{/if}
					<p>{FILES_STAY_ON_YOUR_COMPUTER}</p>
					<p><a href="/resources/files">See which files are on this computer</a></p>
				</Explain>
			{/snippet}

			{#if chosenCard === null}
				<p class="imp-line">Fill in the template, then upload it. We check it before adding anything.</p>
				{#if uploadRefusal !== null}
					<p class="flow-warn">{uploadRefusal}</p>
				{:else if openBatch !== null}
					<p class="flow-warn">{IMPORT_ALREADY_OPEN}</p>
				{/if}
				{#if sheetRefusal !== null}
					<Banner tone="bad" title="Your sheet was not accepted">{sheetRefusal}</Banner>
				{/if}
			{:else if chosenCard.unreadable !== null}
				<p class="flow-warn">{chosenCard.unreadable}</p>
			{:else}
				{@const card = chosenCard}
				{@const device = devicePill(card)}
				{@const blocked = importBlocked(card, shopRefusal, localSessions.get(card.marketplace), inApp)}
				<p class="imp-line">
					<span>{deviceLine(card)}</span>
					<StatusPill tone={device.tone} label={device.label} />
				</p>
				{#if blocked !== null && openRunOf(card) === undefined}
					<p class="flow-warn">
						{blocked}
						{#if localSessions.get(card.marketplace)?.kind === 'known'}
							<Button small tier="outline" href={CONNECT_HREF}>{CONNECT_LABEL}</Button>
						{/if}
					</p>
				{/if}
			{/if}

			{#if declined !== null}
				<Banner tone="bad" title="Your import did not start">
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

			<!-- The sheet's one file input. Every upload control on the page is a
			     label for it, so the keyboard reaches one control and the phone's
			     action bar and the step's own button open the same picker. -->
			<input
				id={sheetInputId}
				class="imp-file"
				type="file"
				accept=".xlsx,.csv"
				disabled={sending || uploadRefusal !== null || openBatch !== null}
				onchange={sheetChosen}
			/>
		</FlowStep>

		<FlowStep
			n={3}
			id="imports"
			title="Your imports"
			summary={runsTotal === 0 ? 'None yet' : countWord(runsTotal, IMPORTS)}
			footer={runsLoaded && rows.length > 0 ? workBar : undefined}
			bind:open={importsOpen}
		>
			<div class="imp-filters">
				<div class="imp-chips" role="group" aria-label="Where from">
					{#each sourceChips as chip (chip.value)}
						<button
							type="button"
							class="imp-chip"
							aria-pressed={runSource === chip.value}
							onclick={() => chooseSource(chip.value)}
						>
							{chip.label}
						</button>
					{/each}
				</div>
				<div class="imp-chips" role="group" aria-label="Status">
					{#each STATE_CHIPS as chip (chip.value)}
						<button
							type="button"
							class="imp-chip"
							aria-pressed={runState === chip.value}
							onclick={() => chooseState(chip.value)}
						>
							{chip.label}
						</button>
					{/each}
					<button type="button" class="imp-chip order" onclick={flipOrder} aria-label="Order: {runOrder === 'newest' ? 'newest first' : 'oldest first'}. Press to flip.">
						{runOrder === 'newest' ? 'Newest first' : 'Oldest first'} ⇅
					</button>
					{#if runsFiltered}
						<Button small tier="quiet" icon="x" onclick={clearRunFilters}>Clear</Button>
					{/if}
				</div>
			</div>

			<!-- A failed read is a banner over the history, not instead of it: the
			     rows already read are still true. -->
			{#if runsUnread}
				<Banner tone="bad" title="We could not load your imports">
					{IMPORTS_UNREAD}
					{runsFailedFor === null
						? ''
						: `We could not load page ${runsFailedFor}. Below is the last page we loaded.`}
					{#snippet action()}
						<Button
							tier="outline"
							small
							disabled={runsBusy}
							reason={runsBusy ? 'Loading your imports.' : undefined}
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
				<!-- "Nothing matches", "nothing left on this page" and "never ran an
				     import" are three different facts. -->
				{#if runsFiltered}
					<p class="quiet">No import matches these filters.</p>
				{:else if shownRunPage > 1}
					<p class="quiet">This page is empty. Go back a page.</p>
				{:else}
					<Placeholder
						icon="download"
						headline={NO_IMPORT_YET}
						body="Choose where from above to start one."
					/>
				{/if}
			{:else}
				<div class="flow-table-wrap">
					<table class="flow-table imp-table">
						<thead class="imp-head">
							<tr>
								<th><span class="sr-only">Select</span></th>
								<th>Import</th>
								<th class="imp-counts-col">Counts</th>
								<th>Status</th>
								<th class="imp-when">When</th>
								<th><span class="sr-only">Actions</span></th>
							</tr>
						</thead>
						<tbody>
							{#each rows as row (row.id)}
								{@const going = retainedBadge(row.deletion)}
								{@const refusal = deleteRefusal(row.deletion)}
								<tr>
									<td class="marks imp-pick">
										<input
											type="checkbox"
											checked={picked.has(row.id)}
											disabled={refusal !== null}
											title={refusal ?? undefined}
											aria-label={`Select the import ${importRunLabel(row, Date.now())}`}
											onchange={(event) => pick(row, event.currentTarget.checked)}
										/>
									</td>
									<td class="imp-main">
										<a class="imp-name" href={row.href}>
											{#if row.source !== null}
												<MarketplaceMark inventory={row.source} size={18} />
											{:else}
												<span class="imp-sheet-mark" aria-hidden="true"><Icon name="layout-list" size={15} /></span>
											{/if}
											<span class="imp-name-word">{row.name}</span>
										</a>
										<span class="imp-counts-inline">{row.line}</span>
									</td>
									<td class="imp-counts-col">{row.line}</td>
									<td class="marks imp-state">
										<StatusPill tone={going?.tone ?? row.tone} label={going?.label ?? row.label} />
									</td>
									<td class="marks imp-when">{agoLabel(row.created_at, Date.now())}</td>
									<td class="marks imp-act">
										<Button
											small
											tier="quiet"
											danger
											icon="trash-2"
											label="Delete"
											disabled={refusal !== null}
											reason={refusal ?? undefined}
											onclick={() =>
												(deleting = [{ id: row.id, label: importRunLabel(row, Date.now()) }])}
										>
											Delete
										</Button>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
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
		</FlowStep>
	</div>
</div>

<FlowActionBar>
	{@render startAction()}
</FlowActionBar>

<!-- The start step's one primary control, for whichever source is chosen.
     Drawn at the step's top right, and again in the phone's action bar. -->
{#snippet startAction()}
	{#if chosenCard === null}
		{#if uploadRefusal !== null}
			<Button tier="primary" icon="upload" disabled reason={uploadRefusal}>Upload your sheet</Button>
		{:else if openBatch !== null}
			<Button tier="primary" href={batchHref(openBatch)}>Open your current import</Button>
		{:else}
			<!-- A label for the step's one file input rather than a button that
			     clicks it, so the file input stays the accessible control. -->
			<label class="cta imp-upload" for={sheetInputId} aria-disabled={sending}>
				<Icon name="upload" size={16} />
				<span class="btn-word">{sending ? 'Reading your sheet…' : 'Upload your sheet'}</span>
			</label>
		{/if}
	{:else}
		{@const card = chosenCard}
		{@const site = card.sites[0]}
		{@const open = openRunOf(card)}
		{@const blocked = importBlocked(card, shopRefusal, localSessions.get(card.marketplace), inApp)}
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
			<!-- Disabled rather than absent, so the step still shows what the
			     seller would do here; the line above it says what stands in the
			     way. -->
			<Button tier="primary" icon="download" disabled reason={blocked ?? undefined}>
				{importLabel(card)}
			</Button>
		{/if}
	{/if}
{/snippet}

<!-- Select-all and Delete, stuck to the bottom of the history while it
     scrolls, so the act stays in reach of the rows it acts on. -->
{#snippet workBar()}
	<div class="work-bar imp-work-bar">
		<label class="work-pick-all">
			<input
				type="checkbox"
				checked={allPickedHere}
				disabled={pickable.length === 0}
				onchange={pickPage}
			/>
			Select all {pickable.length} here
		</label>
		{#if picked.size > 0}
			<span class="work-picked">
				{countWord(picked.size, IMPORTS)} selected{pickedElsewhere > 0
					? `, ${pickedElsewhere} on another page`
					: ''}
			</span>
			<div class="work-bar-acts">
				<Button small tier="quiet" icon="circle-x" onclick={() => (picked = new Map())}>
					Clear
				</Button>
				<Button small danger icon="trash-2" onclick={() => (deleting = pickedItems)}>
					Delete {countWord(picked.size, IMPORTS)}
				</Button>
			</div>
		{/if}
	</div>
{/snippet}

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
