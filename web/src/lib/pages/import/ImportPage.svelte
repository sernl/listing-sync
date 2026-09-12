<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { ApiFailure, api, type ConnectionView, type ImportRunHead } from '$lib/api';
	import type { InventoryId } from '$lib/generated/vocab';
	import Banner from '$lib/Banner.svelte';
	import { anyConnectionStands } from '$lib/connection-standing';
	import Button from '$lib/Button.svelte';
	import { desktopInvoker, startImportHere } from '$lib/desktop';
	import { agoLabel } from '$lib/elapsed';
	import { entitlementRead, featureOf } from '$lib/entitlement-read';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { saveDocument } from '$lib/pages/export/download';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { FILES_STAY_ON_YOUR_COMPUTER } from '$lib/sync-request';
	import {
		CONNECTIONS_UNREAD,
		CONNECT_HREF,
		CONNECT_LABEL,
		IMPORTS_UNREAD,
		NEEDS_THE_APP,
		NOTHING_CONNECTED,
		NO_IMPORT_YET,
		WHAT_AN_IMPORT_IS,
		deviceLine,
		importBlocked,
		importCards,
		importLabel,
		importRows,
		notConnected,
		standingBadge,
		startRefusal
	} from './import-view';
	import { fetchTemplate, openBatchFrom, uploadSheet } from './api';
	import { IMPORT_ALREADY_OPEN, batchHref } from './sheet-view';
	import './import.css';
	import './sheet.css';

	// Null until the list has actually been read. An empty array is a seller
	// with no marketplace, which is a claim; not having read the list is not.
	let connections = $state<ConnectionView[] | null>(null);
	let connectionsUnread = $state(false);

	// Both ways in, in one list. A seller who read a shop on Monday and a
	// spreadsheet on Tuesday has made two imports, not one of each: the two
	// tables behind them are ours rather than theirs.
	let runs = $state<ImportRunHead[]>([]);
	let runsLoaded = $state(false);
	let runsUnread = $state(false);

	// The open batch, so the spreadsheet card's upload control states why it
	// is unavailable rather than losing its button. The server's own answer
	// rather than a search of a list.
	let openBatch = $state<string | null>(null);
	let sheetRefusal = $state<string | null>(null);
	let sending = $state(false);
	let downloading = $state(false);

	// Which marketplace is being started, so one card's spinner never claims
	// the other's. Null while nothing is starting.
	let starting = $state<string | null>(null);
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

	$effect(() => {
		void api
			.connections()
			.then((held) => {
				connections = held;
				connectionsUnread = false;
			})
			.catch(() => {
				connections = null;
				connectionsUnread = true;
			});
		void loadRuns();
		void loadOpenBatch();
	});

	async function loadRuns() {
		try {
			runs = (await api.importRuns()).runs;
			runsUnread = false;
		} catch {
			runs = [];
			runsUnread = true;
		}
		runsLoaded = true;
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
		if (starting !== null) {
			return;
		}
		starting = inventory;
		declined = null;
		raised = null;
		try {
			const run = await api.createImportRun(inventory);
			raised = run.id;
			const outcome = await startImportHere(invoke, run.id);
			const refused = startRefusal(outcome);
			if (refused !== null) {
				declined = refused;
				return;
			}
			if (outcome.kind === 'unavailable') {
				declined = NEEDS_THE_APP;
				return;
			}
			await goto(`/imports/runs/${run.id}`);
		} catch (failure) {
			declined =
				failure instanceof ApiFailure
					? failure.message
					: 'That did not reach us. No import was started.';
		} finally {
			starting = null;
			await loadRuns();
		}
	}
</script>

<div class="page">
	<PageHead
		icon="download"
		title="Import"
		description="Bring your current portfolio to Teachouse from anywhere it is housed."
	/>

	<p class="import-lead">{WHAT_AN_IMPORT_IS}</p>

	{#if connectionsUnread}
		<Banner tone="bad" title="We could not read your marketplaces">{CONNECTIONS_UNREAD}</Banner>
	{:else if nothingHeld}
		<Banner tone="warn" title="No marketplace is connected" action={toMarketplaces}>
			{NOTHING_CONNECTED}
		</Banner>
	{/if}

	<section class="sh-card">
		<div class="head">
			<h2>Import from a spreadsheet</h2>
			<span class="badges"><StatusPill tone="flat" label="Read on our server" /></span>
		</div>
		<p>
			One row per resource in our template. We check every row and show you the result before
			anything is created. No marketplace login is needed.
		</p>
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
			{@const blocked = importBlocked(card, shopRefusal)}
			<section class="import-card">
				<div class="head">
					<h2><MarketplaceMark marketplace={card.marketplace} size={22} /></h2>
					<span class="badges">
						{#if card.unreadable === null}
							{@const badge = standingBadge(card)}
							<StatusPill tone={badge.tone} label={badge.label} />
						{/if}
						<StatusPill tone="flat" label="On your device" />
					</span>
				</div>

				{#if card.unreadable !== null}
					<p class="why">{card.unreadable}</p>
				{:else}
					<p class="quiet">{deviceLine(card)}</p>

					{#if card.standing === 'absent'}
						<Banner tone="bad">
							{notConnected(card)}
							{#snippet action()}
								<Button href={CONNECT_HREF}>{CONNECT_LABEL}</Button>
							{/snippet}
						</Banner>
					{/if}

					{#if invoke === null}
						<p class="why">{NEEDS_THE_APP}</p>
					{/if}

					<div class="actions">
						{#if blocked === null && site !== undefined}
							<Button
								tier="primary"
								icon="download"
								disabled={starting !== null}
								reason={starting !== null ? 'An import is starting.' : undefined}
								onclick={() => void startImport(site)}
							>
								{starting === site ? 'Starting…' : importLabel(card)}
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

	<p class="foot-note">{FILES_STAY_ON_YOUR_COMPUTER}</p>

	<Panel title="Your imports" description="Every import you have run, newest first.">
		{#if runsUnread}
			<p class="quiet">{IMPORTS_UNREAD}</p>
		{:else if !runsLoaded}
			<p class="quiet">Loading…</p>
		{:else if rows.length === 0}
			<Placeholder
				icon="download"
				headline={NO_IMPORT_YET}
				body="Upload a spreadsheet or choose a marketplace above to start one."
			/>
		{:else}
			{#each rows as row (row.id)}
				<!-- The badge and the line stay inside one link. Two of the labels
				     are told apart by the sentence that follows them, and announced
				     as one link the pairing resolves for a reader who gets no
				     colour as the tone resolves it for everyone else. -->
				<a class="import-row" href={row.href}>
					<span class="mark"><StatusPill tone={row.tone} label={row.label} /></span>
					<span class="who">
						<span class="t">
							{#if row.source !== null}
								<MarketplaceMark inventory={row.source} />
							{:else}
								{row.name}
							{/if}
						</span>
						<span class="w">{row.line}</span>
					</span>
					<span class="at">{agoLabel(row.created_at, Date.now())}</span>
				</a>
			{/each}
		{/if}
	</Panel>
</div>

{#snippet toMarketplaces()}
	<Button tier="outline" small href={CONNECT_HREF}>{CONNECT_LABEL}</Button>
{/snippet}
