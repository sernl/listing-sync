<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import ActivityLog from '$lib/ActivityLog.svelte';
	import {
		ApiFailure,
		api,
		type ConnectionView,
		type SyncRequestHead
	} from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { anyConnectionStands } from '$lib/connection-standing';
	import { APP_TOO_OLD, desktopInvoker, startImportHere } from '$lib/desktop';
	import { dayMonth, migrationsLine } from '$lib/entitlement';
	import { entitlementRead } from '$lib/entitlement-read';
	import Field from '$lib/Field.svelte';
	import type { Marketplace } from '$lib/generated/vocab';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { MARKETPLACE_NAME } from '$lib/platforms';
	import StatusPill from '$lib/StatusPill.svelte';
	import {
		AUTHORSHIP_FIRST,
		AUTHORSHIP_HREF,
		FILES_STAY_ON_YOUR_COMPUTER,
		mayMigrate,
		migrateBody,
		migrateSource,
		sourcesOn,
		targetAuthorship
	} from '$lib/sync-request';
	import MarketplaceList from '$lib/pages/automations/MarketplaceList.svelte';
	import { heldSelection, marketplaceRows } from '$lib/pages/automations/marketplace-list';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import {
		ALREADY_RUNNING_TITLE,
		NO_MIGRATION_YET,
		WHAT_A_MIGRATION_IS,
		migrationCounts,
		migrationLog,
		migrationRows,
		openFromSource
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
	// Set only when the request was created and this computer then declined to
	// run it. The request exists and must stay reachable, so the card offers it
	// rather than leaving the seller on a refusal with nowhere to go.
	let created = $state<string | null>(null);
	let logQuery = $state('');

	// Read once: whether this console runs inside the desktop application does
	// not change while the page is open.
	const invoke = desktopInvoker();

	// One key per source, minted once: the server takes the idempotency key as
	// the request's own identity, so a retry after a failed submit reaches the
	// same request rather than starting a second import of the same shop.
	let keys: Record<string, string> = {};

	const from = $derived(migrateSource(connections));

	/** Whether the seller has no marketplace at all, which is a different fact
	 *  from having none this migration can read.
	 *
	 *  `from` answers only the second: `MIGRATE_SOURCES` is TES and nothing
	 *  else, so it is null for a seller who connected TPT first —
	 *  the ordinary order, since TPT is where a migration writes. Titling a
	 *  banner "No marketplace is connected" off `from` told that seller
	 *  something false. */
	const nothingConnected = $derived(!connectionsUnread && !anyConnectionStands(connections));
	const sites = $derived(from === null ? [] : sourcesOn(from));
	// One marketplace reads out, so there is nothing for the seller to pick.
	const source = $derived(sites[0] ?? null);
	// The console's own gate, and until the server refuses one of its own it is
	// the only gate: a migrate submitted with no declaration on record settles
	// every listing it creates failed and terminal, and declaring afterwards
	// brings none of them back.
	const declared = $derived(mayMigrate(targetAuthorship(connections)));
	// A migration already reading this shop. While there is one, the start
	// control is replaced by a banner naming it: the same shop migrated twice
	// drafts every listing on TPT twice, and the seller would have no way to
	// tell the two runs apart afterwards.
	const alreadyRunning = $derived(openFromSource(requests, source));

	const rows = $derived(marketplaceRows(connections, migrationCounts(requests)));
	// The resolved selection rather than the raw click, so the highlighted row
	// and the card beside it cannot name different marketplaces.
	const selected = $derived(heldSelection(rows, held));
	// `Date.now()` inside the derivation rather than captured at init, so a
	// label recomputes with its list instead of freezing at mount.
	const past = $derived(migrationRows(requests, Date.now()));
	const log = $derived(migrationLog(requests, Date.now()));

	// The monthly migration allowance, and why the submit is refused once it
	// is spent. The line stands above the control whether or not the
	// allowance is spent: a seller about to move forty resources on a
	// twenty-a-month plan needs the figure before they press, not after the
	// server refuses them.
	const plan = createQuery(() => entitlementRead);
	const monthly = $derived(
		plan.data === undefined
			? null
			: migrationsLine(plan.data.usage, plan.data.capabilities)
	);
	const capped = $derived.by(() => {
		const held = plan.data;
		if (held === undefined) {
			return null;
		}
		if (held.capabilities.migrations_per_month === 0) {
			return 'Your plan does not move resources between marketplaces. Upgrade to migrate.';
		}
		return held.usage.migrations_this_month >= held.capabilities.migrations_per_month
			? `Your plan moves ${held.capabilities.migrations_per_month} resources a month and you have used them. It resets ${dayMonth(held.usage.migrations_reset_at)}.`
			: null;
	});

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
			.then((held) => {
				connections = held;
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

	{#if nothingConnected}
		<Banner tone="warn" title="No marketplace is connected" action={toDownloads}>
			A migration reads a shop you already sell on, so there is nothing to move until one is
			connected. Connect it in the Teachouse app on your computer: the app opens the
			marketplace sign-in there and keeps your login on that machine, which is the only
			place it is kept.
		</Banner>
	{/if}

	<div class="auto-body">
		<MarketplaceList {rows} {selected} onselect={(marketplace) => (held = marketplace)}
			empty="Connect a marketplace in the Teachouse app and it appears here." />

		<div class="auto-right">
			{#if connectionsUnread}
				<Panel title="Bring a shop across">
					<p class="quiet">
						We could not read your marketplaces, so we cannot tell whether you have a shop to
						bring across. Nothing has started.
					</p>
				</Panel>
			{:else if nothingConnected}
				<!-- The banner above already states the blocker and offers the one
				     remedy, so this column says what the feature is rather than
				     repeating it: two panels naming the same missing thing read as
				     two different problems. -->
				<Panel title="Bring a shop across">
					<p class="quiet">{WHAT_A_MIGRATION_IS}</p>
				</Panel>
			{:else if from === null}
				<Panel title="Bring a shop across">
					<p class="quiet">
						A migration reads the shop you already sell on, and the one it reads is TES, so
						it needs TES connected. That happens in the Teachouse app on your computer,
						which opens the marketplace sign-in and keeps your login on that machine.
					</p>
					<div class="set-foot">
						<Button href="/marketplaces" tier="outline">Connect on Marketplaces</Button>
					</div>
				</Panel>
			{:else}
				<Panel title="Bring a shop across">
					<p class="migrate-lead">{WHAT_A_MIGRATION_IS}</p>

					<div class="set-grid">
						<Field
							label="From"
							id="migrate-from"
							hint="The one marketplace a migration reads from, so there is nothing to choose."
						>
							<select id="migrate-from" disabled>
								<option>{MARKETPLACE_NAME.Tes}</option>
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
						<Banner tone="warn" title="Declare the copyright holder first">
							{AUTHORSHIP_FIRST}
							{#snippet action()}
								<Button href={AUTHORSHIP_HREF} tier="primary" small>
									Declare copyright for TPT
								</Button>
							{/snippet}
						</Banner>
					{:else}
						<div class="set-foot">
							{#if monthly !== null}
								<p class="foot-note">{monthly}</p>
							{/if}
							<Button
								tier="primary"
								disabled={capped !== null || starting || source === null}
								reason={capped ??
									(starting
										? 'The migration is starting.'
										: source === null
											? 'No marketplace this migration could read is connected.'
											: undefined)}
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
				description="Every shop you have brought across, newest first."
			>
				{#if requestsUnread}
					<p class="quiet">
						Your migrations could not be read, so this page cannot list them. Any migration
						already running is unaffected.
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
								<span class="t">
									<MarketplaceMark inventory={row.source} /> →
									<MarketplaceMark inventory={row.target} />
								</span>
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

{#snippet toDownloads()}
	<Button tier="outline" small href="/marketplaces">Connect on Marketplaces</Button>
{/snippet}
