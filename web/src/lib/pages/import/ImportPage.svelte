<script lang="ts">
	import { goto } from '$app/navigation';
	import {
		ApiFailure,
		api,
		type ConnectionView,
		type ImportBatchView,
		type SyncRequestHead
	} from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import { anyConnectionStands } from '$lib/connection-standing';
	import Button from '$lib/Button.svelte';
	import { agoLabel } from '$lib/elapsed';
	import Field from '$lib/Field.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { readState } from '$lib/pages/automations/read-state';
	import { saveDocument } from '$lib/pages/export/download';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import { SHORT_NAME, platformTitle } from '$lib/platforms';
	import StatusPill from '$lib/StatusPill.svelte';
	import {
		FILES_STAY_ON_YOUR_COMPUTER,
		siteChoiceQuestion,
		siteLabel
	} from '$lib/sync-request';
	import type { InventoryId } from '$lib/generated/vocab';
	import {
		CONNECTIONS_UNREAD,
		CONNECT_HREF,
		CONNECT_LABEL,
		HANDOFF_LABEL,
		IMPORTS_UNREAD,
		IMPORT_IS_A_MIGRATION,
		NOTHING_CONNECTED,
		NO_IMPORT_YET,
		WHAT_AN_IMPORT_IS,
		deviceLine,
		handoffBlocked,
		importCards,
		importRows,
		migrationHref,
		notConnected,
		standingBadge,
		type ImportCard
	} from './import-view';
	import { fetchTemplate, openBatchFrom, uploadSheet } from './api';
	import { IMPORT_ALREADY_OPEN, batchHref, batchRows, listCopy } from './sheet-view';
	import './import.css';
	import './sheet.css';

	// Null until the list has actually been read. An empty array is a seller
	// with no marketplace, which is a claim; not having read the list is not.
	let connections = $state<ConnectionView[] | null>(null);
	let connectionsUnread = $state(false);

	let requests = $state<SyncRequestHead[]>([]);
	let requestsLoaded = $state(false);
	let requestsUnread = $state(false);

	let chosen = $state<Record<string, InventoryId>>({});

	// The spreadsheet import, which is a second way in rather than a second
	// view of the same thing: these batches and the migrate requests below
	// address different identifier spaces and settle in different words.
	let batches = $state<ImportBatchView[]>([]);
	let batchesLoaded = $state(false);
	let batchesUnread = $state(false);
	// Null until the listing has been read. `open` is the server's own answer
	// rather than a search of the list, so the card's reason and the index's
	// predicate stay one thing.
	let openBatch = $state<string | null>(null);
	let sheetRefusal = $state<string | null>(null);
	let sending = $state(false);
	let downloading = $state(false);

	const base = $props.id();
	const sheetInputId = `${base}-sheet`;

	const cards = $derived(importCards(connections));

	/** Whether the seller has no marketplace connected at all.
	 *
	 *  Over every connection rather than over this screen's own cards, because
	 *  the sentence claims about all of them. The cards are the device branch
	 *  alone, so a seller connected only on the server branch would be told
	 *  they have nothing, which is false.
	 *
	 *  Only once the list has been read: a null list is not a seller with no
	 *  shop, and raising this on it would tell them there is nothing when all
	 *  we know is that we could not ask. */
	const nothingHeld = $derived(
		!connectionsUnread && connections !== null && !anyConnectionStands(connections)
	);
	const rows = $derived(importRows(requests));
	const sheets = $derived(readState(batchesLoaded, batchesUnread, batchRows(batches)));
	const sheetsSay = $derived(listCopy(sheets));

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
		void loadRequests();
		void loadBatches();
	});

	async function loadRequests() {
		try {
			requests = (await api.syncRequests()).requests;
			requestsUnread = false;
		} catch {
			requests = [];
			requestsUnread = true;
		}
		requestsLoaded = true;
	}

	function siteOf(card: ImportCard): InventoryId | null {
		return chosen[card.marketplace] ?? card.preselected;
	}

	async function loadBatches() {
		try {
			const held = await api.imports();
			batches = held.imports;
			openBatch = held.open;
			batchesUnread = false;
		} catch {
			batches = [];
			openBatch = null;
			batchesUnread = true;
		}
		batchesLoaded = true;
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
			await loadBatches();
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
	<p class="import-lead">{IMPORT_IS_A_MIGRATION}</p>

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
			{#if openBatch === null}
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

	<div class="import-cards">
		{#each cards as card (card.marketplace)}
			{@const blocked = handoffBlocked(card)}
			{@const site = siteOf(card)}
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
					{#if card.sites.length > 1}
						<Field
							label={siteChoiceQuestion(card.sites)}
							id={`import-site-${card.marketplace}`}
						>
							<select
								id={`import-site-${card.marketplace}`}
								value={site}
								onchange={(event) =>
									(chosen[card.marketplace] = event.currentTarget.value as InventoryId)}
							>
								{#each card.sites as option (option)}
									<option value={option} title={platformTitle(option)}>
										{SHORT_NAME[option]} — {siteLabel(option)}
									</option>
								{/each}
							</select>
						</Field>
					{/if}

					<p class="quiet">{deviceLine(card)}</p>

					{#if card.standing === 'absent'}
						<Banner tone="bad">
							{notConnected(card)}
							{#snippet action()}
								<Button href={CONNECT_HREF}>{CONNECT_LABEL}</Button>
							{/snippet}
						</Banner>
					{/if}

					<div class="actions">
						{#if blocked === null && site !== null}
							<Button tier="primary" icon="arrow-right-left" href={migrationHref(site)}>
								{HANDOFF_LABEL}
							</Button>
						{:else}
							<!-- Disabled rather than absent, so the card still shows what
							     the seller would do here and says what stands in the way. -->
							<Button
								tier="primary"
								icon="arrow-right-left"
								disabled
								reason={blocked ?? undefined}
							>
								{HANDOFF_LABEL}
							</Button>
						{/if}
					</div>
				{/if}
			</section>
		{/each}
	</div>

	<p class="foot-note">{FILES_STAY_ON_YOUR_COMPUTER}</p>

	<Panel
		title="Spreadsheet imports"
		description="Every sheet you have uploaded, newest first."
	>
		{#if sheetsSay !== null}
			<p class="quiet">{sheetsSay.title}</p>
			{#if sheetsSay.body !== ''}<p class="quiet">{sheetsSay.body}</p>{/if}
		{:else if sheets.kind === 'rows'}
			{#each sheets.rows as sheet (sheet.id)}
				<!-- The badge and the line stay inside one link, for the reason the
				     migrate list beside this one states: two of the labels are told
				     apart by the sentence that follows them. -->
				<a class="sh-listed" href={sheet.href}>
					<span class="mark"><StatusPill tone={sheet.tone} label={sheet.label} /></span>
					<span class="who">
						<span class="t">{sheet.name}</span>
						<span class="w">{sheet.line}</span>
					</span>
					<span class="at">{agoLabel(sheet.createdAt, Date.now())}</span>
				</a>
			{/each}
		{/if}
	</Panel>

	<Panel
		title="Your imports"
		description="Every import you have run, newest first."
	>
		{#if requestsUnread}
			<p class="quiet">{IMPORTS_UNREAD}</p>
		{:else if !requestsLoaded}
			<p class="quiet">Loading…</p>
		{:else if rows.length === 0}
			<Placeholder
				icon="download"
				headline={NO_IMPORT_YET}
				body="Choose a marketplace above to start one."
			/>
		{:else}
			{#each rows as row (row.request)}
				<!-- The badge and the line must stay inside one link. Two of the stage
				     labels are "Nothing to import" and "Nothing imported", which a
				     reader who gets no colour tells apart only by the sentence that
				     follows; announced as one link, the pairing resolves for them as
				     the tone resolves it for everyone else. Splitting the badge out of
				     this anchor, or showing the label without its line anywhere, makes
				     those two labels indistinguishable and needs different words
				     upstream rather than a change here. -->
				<a class="import-row" href={`/sync/requests/${row.request}`}>
					<span class="mark"><StatusPill tone={row.tone} label={row.label} /></span>
					<span class="who">
						<span class="t">
							<MarketplaceMark inventory={row.source} /> →
							<MarketplaceMark inventory={row.target} />
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
