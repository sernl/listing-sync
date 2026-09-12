<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import ActivityLog from '$lib/ActivityLog.svelte';
	import {
		ApiFailure,
		allPages,
		api,
		type ConnectionView,
		type Disposition,
		type MigrationPlanView,
		type ProductHead,
		type SyncRequestHead
	} from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { anyConnectionStands } from '$lib/connection-standing';
	import { migrationsReason } from '$lib/entitlement';
	import { entitlementRead } from '$lib/entitlement-read';
	import { external } from '$lib/external';
	import Field from '$lib/Field.svelte';
	import type { InventoryId, Marketplace } from '$lib/generated/vocab';
	import { formatPrice, normaliseQuery } from '$lib/listings-view';
	import {
		DISPOSITIONS,
		DISPOSITION_LINE,
		DISPOSITION_WORD,
		VERDICT_TONE,
		VERDICT_WORD,
		capSentence,
		confirmLabel,
		countsLine,
		migrationBody,
		migrationSources,
		migrationTargets,
		pairReason,
		productsFromUrl
	} from '$lib/migration-plan';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { SHORT_NAME } from '$lib/platforms';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import {
		AUTHORSHIP_FIRST,
		AUTHORSHIP_HREF,
		FILES_STAY_ON_YOUR_COMPUTER,
		mayMigrate,
		targetAuthorship
	} from '$lib/sync-request';
	import MarketplaceList from '$lib/pages/automations/MarketplaceList.svelte';
	import { heldSelection, marketplaceRows } from '$lib/pages/automations/marketplace-list';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import {
		NO_MIGRATION_YET,
		WHAT_A_MIGRATION_IS,
		migrationCounts,
		migrationLog,
		migrationRows
	} from '$lib/pages/automations/migration';
	import '$lib/pages/automations/automations.css';

	let requests = $state<SyncRequestHead[]>([]);
	let requestsLoaded = $state(false);
	// A migration list that could not be read is not a seller who has brought
	// no shop across. Told the second when the first is true, they start the
	// migration again.
	let requestsUnread = $state(false);

	// A marketplace list that could not be read is not a seller with no
	// marketplace: the card says which of the two it is looking at rather than
	// hiding the action and letting an outage read as "you have no shop".
	let connections = $state<ConnectionView[]>([]);
	let connectionsUnread = $state(false);

	let held = $state<Marketplace | null>(null);
	let logQuery = $state('');

	// The pair, and the two ends are independent: every marketplace is offered
	// at both, disabled with its reason where it cannot serve. The defaults are
	// the one pair that runs today, so a seller who changes nothing is on the
	// path that works.
	let source = $state<InventoryId>('Tes');
	let target = $state<InventoryId>('Tpt');
	let disposition = $state<Disposition>('sync');

	// A selection arriving from the Resources board opens the tick list on what
	// it named; a bare visit opens on the whole shop, which is what a seller
	// bringing a marketplace across came for.
	const preselected = productsFromUrl(page.url.searchParams);
	let all = $state(preselected.length === 0);
	let ticked = $state<Set<string>>(new Set(preselected));
	let box = $state('');

	let plan = $state<MigrationPlanView | null>(null);
	let previewing = $state(false);
	let planFailure = $state<string | null>(null);
	let confirming = $state(false);
	let refusal = $state<string | null>(null);
	// Minted at the confirm rather than at mount: the server takes the key as
	// the request's own identity, so it has to change when the selection does
	// and stay put across a failed submit's retry.
	let key = $state<string | null>(null);

	const sources = migrationSources();
	const targets = migrationTargets();

	const catalogue = createQuery(() => ({
		queryKey: queryKeys.catalogue(null),
		queryFn: () =>
			allPages(
				(cursor) => api.products(cursor, null),
				(view) => view.products
			)
	}));
	const mappings = createQuery(() => ({
		queryKey: queryKeys.mappings,
		queryFn: () => api.mappings().then((view) => view.mappings)
	}));

	const products = $derived<ProductHead[]>(catalogue.data ?? []);
	const marksOf = $derived.by(() => {
		const index = new Map<string, InventoryId[]>();
		for (const mapping of mappings.data ?? []) {
			index.set(mapping.product, [...(index.get(mapping.product) ?? []), mapping.inventory]);
		}
		return index;
	});
	const query = $derived(normaliseQuery(box).toLocaleLowerCase());
	const shown = $derived(
		query.length === 0
			? products
			: products.filter((product) => product.title.toLocaleLowerCase().includes(query))
	);
	const allShownTicked = $derived(
		shown.length > 0 && shown.every((product) => ticked.has(product.id))
	);

	const nothingConnected = $derived(!connectionsUnread && !anyConnectionStands(connections));
	const pairRefusal = $derived(pairReason(source, target));
	// The console's own gate, and until the server refuses one of its own it is
	// the only gate: a migration submitted with no declaration on record settles
	// every listing it creates failed and terminal, and declaring afterwards
	// brings none of them back.
	const declared = $derived(mayMigrate(targetAuthorship(connections, target)));
	// A migration already reading this shop. While there is one, the confirm is
	// replaced by a banner naming it: the same shop migrated twice drafts every
	// listing on the target twice, and the seller would have no way to tell the
	// two runs apart afterwards.

	const rows = $derived(marketplaceRows(connections, migrationCounts(requests)));
	// The resolved selection rather than the raw click, so the highlighted row
	// and the card beside it cannot name different marketplaces.
	const selected = $derived(heldSelection(rows, held));
	// `Date.now()` inside the derivation rather than captured at init, so a
	// label recomputes with its list instead of freezing at mount.
	const past = $derived(migrationRows(requests, Date.now()));
	const log = $derived(migrationLog(requests, Date.now()));

	const body = $derived(
		migrationBody({ source, target, disposition, all, products: [...ticked] })
	);

	// The allowance before a preview has been taken, read off the entitlement
	// the shell already holds. It stands whether or not it refuses: a seller
	// about to move forty resources on a twenty-a-month plan needs the figure
	// before they press, not after the server refuses them.
	const entitlement = createQuery(() => entitlementRead);
	const allowance = $derived(
		entitlement.data === undefined
			? null
			: plan === null
				? migrationsReason(entitlement.data.capabilities, entitlement.data.usage, 0)
				: capSentence(plan.cap, plan.counts.will_create)
	);

	const confirmRefusal = $derived.by(() => {
		if (pairRefusal !== null) {
			return pairRefusal;
		}
		if (plan === null) {
			return 'Preview the migration first, so you can see what it would do.';
		}
		if (plan.counts.will_create === 0) {
			return `Nothing here would be created: every resource you chose is already on ${SHORT_NAME[target]} or blocked.`;
		}
		return allowance?.refusal ?? null;
	});

	// A changed pair, disposition or selection makes the preview on screen a
	// statement about something the seller is no longer asking for, so it is
	// dropped rather than left to be confirmed. The key goes with it: a
	// different selection is a different request.
	$effect(() => {
		void JSON.stringify(body);
		plan = null;
		planFailure = null;
		refusal = null;
		key = null;
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

	function tick(product: string, value: boolean) {
		const next = new Set(ticked);
		if (value) {
			next.add(product);
		} else {
			next.delete(product);
		}
		ticked = next;
	}

	function toggleAllShown() {
		ticked = allShownTicked ? new Set() : new Set(shown.map((product) => product.id));
	}

	async function preview() {
		previewing = true;
		planFailure = null;
		try {
			plan = await api.migrationPlan(body);
		} catch (caught) {
			plan = null;
			planFailure =
				caught instanceof ApiFailure
					? caught.message
					: 'The preview could not be taken, so nothing is shown rather than a guess.';
		} finally {
			previewing = false;
		}
	}

	async function confirm() {
		key ??= crypto.randomUUID();
		confirming = true;
		refusal = null;
		try {
			const ack = await api.createMigration(body, key);
			await goto(`/sync/requests/${ack.request}`);
		} catch (caught) {
			refusal =
				caught instanceof ApiFailure ? caught.message : 'The migration could not be raised.';
		} finally {
			confirming = false;
		}
	}
</script>

<div class="page">
	<PageHead
		icon="arrow-right-left"
		title="Migrations"
		description="Copy or move resources from one marketplace to another."
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
				<Panel title="Set up">
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
				<Panel title="Set up">
					<p class="quiet">{WHAT_A_MIGRATION_IS}</p>
				</Panel>
			{:else}
				<Panel title="Set up">
					<p class="migrate-lead">{WHAT_A_MIGRATION_IS}</p>

					<div class="set-grid">
						<Field label="From" id="migrate-from">
							<select id="migrate-from" bind:value={source}>
								{#each sources as side (side.inventory)}
									<option
										value={side.inventory}
										disabled={!side.enabled}
										title={side.reason ?? undefined}
									>
										{side.label}
									</option>
								{/each}
							</select>
						</Field>
						<Field label="To" id="migrate-to">
							<select id="migrate-to" bind:value={target}>
								{#each targets as side (side.inventory)}
									<option
										value={side.inventory}
										disabled={!side.enabled}
										title={side.reason ?? undefined}
									>
										{side.label}
									</option>
								{/each}
							</select>
						</Field>
					</div>

					{#if pairRefusal !== null}
						<!-- Under the selects rather than only in the option's title: a
						     `title` reaches neither a thumb nor a screen reader, and the
						     pair a seller has already chosen is the one they need the
						     reason for. -->
						<p class="pair-refusal">{pairRefusal}</p>
					{/if}

					<div class="disp-choice" role="radiogroup" aria-label="Copy or move">
						{#each DISPOSITIONS as option (option)}
							<button
								type="button"
								class="disp"
								class:on={disposition === option}
								role="radio"
								aria-checked={disposition === option}
								onclick={() => (disposition = option)}
							>
								<span class="disp-word">{DISPOSITION_WORD[option]}</span>
								<span class="disp-line">{DISPOSITION_LINE[option]}</span>
							</button>
						{/each}
					</div>
				</Panel>

				<Panel title="What to move">
					<div class="scope-choice" role="radiogroup" aria-label="What to move">
						<button
							type="button"
							class="disp"
							class:on={all}
							role="radio"
							aria-checked={all}
							onclick={() => (all = true)}
						>
							<span class="disp-word">All resources on {SHORT_NAME[source]}</span>
						</button>
						<button
							type="button"
							class="disp"
							class:on={!all}
							role="radio"
							aria-checked={!all}
							onclick={() => (all = false)}
						>
							<span class="disp-word">Choose</span>
						</button>
					</div>

					{#if !all}
						{#if catalogue.isPending}
							<p class="quiet">Loading your resources…</p>
						{:else if catalogue.isError}
							<p class="quiet">
								Your resources could not be read, so there is nothing to tick. Moving all of
								{SHORT_NAME[source]} does not need this list and still works.
							</p>
						{:else if products.length === 0}
							<p class="quiet">
								You have no resources yet, so there is nothing to tick. Import brings your
								existing shop across first.
							</p>
						{:else}
							<div class="pick-head">
								<input
									type="search"
									aria-label="Search your resources"
									placeholder="Search resources"
									bind:value={box}
								/>
								<label class="pick-all">
									<input
										type="checkbox"
										checked={allShownTicked}
										onchange={toggleAllShown}
									/>
									Select the {shown.length} shown
								</label>
								<span class="pick-count">{ticked.size} chosen</span>
							</div>

							{#if shown.length === 0}
								<p class="quiet">Nothing matches that search.</p>
							{:else}
								<div class="pick-list">
									{#each shown as product (product.id)}
										<label class="pick-row">
											<input
												type="checkbox"
												checked={ticked.has(product.id)}
												onchange={(event) =>
													tick(product.id, event.currentTarget.checked)}
											/>
											<span class="pick-title">{product.title}</span>
											<span class="pick-marks">
												{#each marksOf.get(product.id) ?? [] as inventory (inventory)}
													<MarketplaceMark {inventory} size={16} />
												{/each}
											</span>
											<span class="pick-price">{formatPrice(product.price)}</span>
										</label>
									{/each}
								</div>
							{/if}
						{/if}
					{/if}
				</Panel>

				<Panel title="Preview" description="What this would do, resource by resource.">
					<div class="set-foot preview-foot">
						<Button
							tier="outline"
							disabled={previewing || pairRefusal !== null}
							reason={pairRefusal ?? (previewing ? 'The preview is being taken.' : undefined)}
							onclick={() => void preview()}
						>
							{previewing ? 'Previewing…' : 'Preview'}
						</Button>
						{#if allowance !== null}
							<p class="foot-note">{allowance.line}</p>
						{/if}
					</div>

					{#if planFailure !== null}
						<Banner tone="bad">{planFailure}</Banner>
					{:else if plan === null}
						<p class="quiet">
							Nothing is queued by a preview. It says, for each resource, whether it would be
							created on {SHORT_NAME[target]}, is already there, or is blocked and why.
						</p>
					{:else}
						{#if !plan.pair.allowed && plan.pair.reason !== null}
							<Banner tone="warn" title="This pair cannot be migrated between">
								{plan.pair.reason}
							</Banner>
						{/if}
						{#if plan.rows.length === 0}
							<p class="quiet">
								This selection names no resources, so there is nothing to preview.
							</p>
						{:else}
							<div class="plan-rows">
								{#each plan.rows as resource (resource.product)}
									<div class="plan-row">
										<span class="plan-title">{resource.title}</span>
										<StatusPill
											tone={VERDICT_TONE[resource.verdict]}
											label={VERDICT_WORD[resource.verdict]}
										/>
										<span class="plan-why">
											{#if resource.remote !== null}
												<a
													href={resource.remote}
													target="_blank"
													rel="noreferrer noopener"
													use:external
												>
													Open the listing on {SHORT_NAME[target]}
												</a>
											{/if}
											{#if resource.reason !== null}
												<span class="block">{resource.reason}</span>
											{/if}
											{#if resource.price_note !== null}
												<span class="block quiet">{resource.price_note}</span>
											{/if}
										</span>
									</div>
								{/each}
							</div>
							<p class="foot-note">{countsLine(plan.counts)}</p>
						{/if}
					{/if}

					{#if pairRefusal === null && !declared}
						<!-- Only once the pair itself stands. A declaration is the writing
						     marketplace's requirement, and asking for one on a pair that
						     cannot be migrated between at all named the wrong cause: the
						     seller would go and declare, come back, and still be refused
						     by the sentence under the select. -->
						<Banner tone="warn" title="Declare the copyright holder first">
							{AUTHORSHIP_FIRST}
							{#snippet action()}
								<Button href={AUTHORSHIP_HREF} tier="primary" small>
									Declare copyright for {SHORT_NAME[target]}
								</Button>
							{/snippet}
						</Banner>
					{:else}
						<div class="set-foot">
							<Button
								tier="primary"
								disabled={confirmRefusal !== null || confirming}
								reason={confirmRefusal ??
									(confirming ? 'The migration is starting.' : undefined)}
								onclick={() => void confirm()}
							>
								{confirming
									? 'Starting…'
									: confirmLabel(disposition, plan?.counts.will_create ?? 0)}
							</Button>
						</div>
					{/if}

					<p class="foot-note">{FILES_STAY_ON_YOUR_COMPUTER}</p>

					{#if refusal !== null}
						<Banner tone="bad">{refusal}</Banner>
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
