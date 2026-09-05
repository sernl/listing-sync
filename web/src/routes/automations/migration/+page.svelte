<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import ActivityLog from '$lib/ActivityLog.svelte';
	import {
		ApiFailure,
		api,
		type ConnectionView,
		type SyncRequestHead
	} from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { APP_TOO_OLD, desktopInvoker, startImportHere } from '$lib/desktop';
	import Field from '$lib/Field.svelte';
	import type { InventoryId, Marketplace } from '$lib/generated/vocab';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { MARKETPLACE_NAME, platformTitle } from '$lib/platforms';
	import StatusPill from '$lib/StatusPill.svelte';
	import {
		AUTHORSHIP_FIRST,
		AUTHORSHIP_HREF,
		FILES_STAY_ON_YOUR_COMPUTER,
		mayMigrate,
		migrateBody,
		migrateSource,
		siteChoiceQuestion,
		siteLabel,
		sourcesOn,
		targetAuthorship
	} from '$lib/sync-request';
	import MarketplaceList from '$lib/pages/automations/MarketplaceList.svelte';
	import { heldSelection, marketplaceRows } from '$lib/pages/automations/marketplace-list';
	import {
		ALREADY_RUNNING_TITLE,
		NO_MIGRATION_YET,
		WHAT_A_MIGRATION_IS,
		migrationCounts,
		migrationLog,
		migrationRows,
		openFromSource,
		seededSource
	} from '$lib/pages/automations/migration';
	import '$lib/pages/automations/automations.css';

	let requests = $state<SyncRequestHead[]>([]);
	let requestsLoaded = $state(false);
	// A migration list that could not be read is not a seller who has brought
	// no shop across. Told the second when the first is true, they start the
	// migration again.
	let requestsUnread = $state(false);

	let connections = $state<ConnectionView[]>([]);
	// A marketplace list that could not be read is not a seller with no
	// marketplace: the card says which of the two it is looking at rather than
	// hiding the action and letting an outage read as "you have no shop".
	let connectionsUnread = $state(false);

	let held = $state<Marketplace | null>(null);
	let starting = $state(false);
	let refusal = $state<string | null>(null);
	// The site the seller picked, or null while they have not picked one and
	// the first site the connection offers stands. Not remembered between
	// visits: nothing on the wire records which site is theirs, so remembering
	// it would mean this console inventing a preference it cannot confirm.
	let chosenSite = $state<InventoryId | null>(null);
	// Set only when the request was created and this computer then declined to
	// run it. The request exists and must stay reachable, so the card offers it
	// rather than leaving the seller on a refusal with nowhere to go.
	let created = $state<string | null>(null);
	let logQuery = $state('');

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
	// The seller's pick, a hand-over from another screen, or this page's own
	// default, in that order. `sites` is `sourcesOn`, so an unknown or
	// unofferable seed falls back to the default rather than emptying the
	// field.
	const source = $derived(
		seededSource(chosenSite, page.url.searchParams.get('source'), sites)
	);

	// A migration already reading this shop. While there is one, the start
	// control is replaced by a banner naming it: the same shop migrated twice
	// drafts every listing on TPT twice, and the seller would have no way to
	// tell the two runs apart afterwards.
	const alreadyRunning = $derived(openFromSource(requests, source));

	/** The picked option read back as an inventory, checked against the list the
	 *  options were drawn from rather than asserted: a cast would be the only
	 *  thing claiming the value is one of them. */
	function siteFrom(value: string): InventoryId | null {
		return sites.find((site) => site === value) ?? null;
	}

	const rows = $derived(marketplaceRows(connections, migrationCounts(requests)));
	// The resolved selection rather than the raw click, so the highlighted row
	// and the card beside it cannot name different marketplaces.
	const selected = $derived(heldSelection(rows, held));
	// `Date.now()` inside the derivation rather than captured at init, so a
	// label recomputes with its list instead of freezing at mount.
	const past = $derived(migrationRows(requests, Date.now()));
	const log = $derived(migrationLog(requests, Date.now()));

	async function load() {
		try {
			const view = await api.syncRequests();
			requests = view.requests;
			requestsUnread = false;
		} catch {
			requests = [];
			requestsUnread = true;
		}
		requestsLoaded = true;
	}

	$effect(() => {
		void load();
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
		icon="arrow-right-left"
		title="Marketplace Migration"
		description="Move a whole shop from one marketplace to another, once."
	/>

	<div class="auto-body">
		<MarketplaceList {rows} {selected} onselect={(marketplace) => (held = marketplace)}
			empty="Connect a marketplace and it appears here." />

		<div class="auto-right">
			{#if connectionsUnread}
				<Panel title="Bring a shop across">
					<p class="quiet">
						Your marketplaces could not be read, so this page cannot say whether you have a
						shop to bring across. Nothing has been started.
					</p>
				</Panel>
			{:else if from === null}
				<Panel title="Bring a shop across">
					<p class="quiet">
						A migration reads the shop you already sell on, so it needs that marketplace
						connected first.
					</p>
					<div class="set-foot">
						<Button href="/marketplaces" tier="outline">Connect a marketplace</Button>
					</div>
				</Panel>
			{:else}
				<Panel title="Bring a shop across">
					<p class="migrate-lead">{WHAT_A_MIGRATION_IS}</p>

					<div class="set-grid">
						<Field label="From" id="migrate-from" hint={siteChoiceQuestion(sites)}>
							<select
								id="migrate-from"
								value={source}
								disabled={starting}
								onchange={(event) => (chosenSite = siteFrom(event.currentTarget.value))}
							>
								{#each sites as site (site)}
									<option value={site} title={platformTitle(site)}>{siteLabel(site)}</option>
								{/each}
							</select>
						</Field>
						<Field
							label="To"
							id="migrate-to"
							hint="The one marketplace a migration writes to, so there is nothing to choose."
						>
							<select id="migrate-to" disabled>
								<option>{MARKETPLACE_NAME.Tpt}</option>
							</select>
						</Field>
					</div>

					{#if alreadyRunning !== null}
						<Banner tone="info" title={ALREADY_RUNNING_TITLE}>
							{alreadyRunning.line}
							{#snippet action()}
								<Button href={alreadyRunning.href} tier="outline" small>
									Open this migration
								</Button>
							{/snippet}
						</Banner>
					{:else if !declared}
						<Banner tone="warn" title="Declare authorship first">
							{AUTHORSHIP_FIRST}
							{#snippet action()}
								<Button href={AUTHORSHIP_HREF} tier="primary" small>
									Declare authorship for TPT
								</Button>
							{/snippet}
						</Banner>
					{:else}
						<div class="set-foot">
							<Button
								tier="primary"
								disabled={starting || source === null}
								reason={starting ? 'The migration is starting.' : undefined}
								onclick={() => void start()}
							>
								{starting ? 'Starting…' : 'Start migration'}
							</Button>
						</div>
					{/if}

					<p class="foot-note">{FILES_STAY_ON_YOUR_COMPUTER}</p>

					{#if refusal !== null}
						<Banner tone="bad">{refusal}</Banner>
					{/if}
					{#if created !== null}
						<div class="set-foot">
							<Button href={`/sync/requests/${created}`} tier="outline">Open the import</Button>
						</div>
					{/if}
				</Panel>
			{/if}

			<Panel
				title="Your migrations"
				description="Every shop you have brought across, newest first. A migration that is still waiting for your device has no run of its own yet, so this is where it lives."
			>
				{#if requestsUnread}
					<p class="quiet">
						Your migrations could not be read, so this page cannot list them. Any migration
						you have already started is still running; nothing here has changed it.
					</p>
				{:else if !requestsLoaded}
					<p class="quiet">Loading…</p>
				{:else if past.length === 0}
					<p class="quiet">{NO_MIGRATION_YET}</p>
				{:else}
					{#each past as row (row.request)}
						<a class="auto-row" href={row.href}>
							<StatusPill tone={row.tone} label={row.label} />
							<span class="who">
								<span class="t">{row.title}</span>
								<span class="meta">{row.meta}</span>
							</span>
						</a>
					{/each}
				{/if}
			</Panel>

			<Panel title="Activity log" description="What each migration did, newest first.">
				<ActivityLog
					entries={log}
					bind:query={logQuery}
					empty="No migration has run yet, so there is nothing to log."
				/>
			</Panel>
		</div>
	</div>
</div>
