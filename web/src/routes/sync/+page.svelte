<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import ActivityLog from '$lib/ActivityLog.svelte';
	import {
		ApiFailure,
		api,
		type ActivityLine,
		type ConnectionView,
		type JobHead,
		type MarketplaceSyncSettingView,
		type MultiListedRow
	} from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { entitlementRead, featureOf } from '$lib/entitlement-read';
	import Field from '$lib/Field.svelte';
	import type { InventoryId, Marketplace } from '$lib/generated/vocab';
	import { MARKETPLACE_OF } from '$lib/listings-view';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { AUTHORABLE_PLATFORMS, MARK_SRC, platformTitle } from '$lib/platforms';
	import Toggle from '$lib/Toggle.svelte';
	import MarketplaceList from '$lib/pages/automations/MarketplaceList.svelte';
	import { heldSelection, marketplaceRows } from '$lib/pages/automations/marketplace-list';
	import { columnCopy, readState } from '$lib/pages/automations/read-state';
	import {
		CONNECT_FIRST,
		FILLS_WHAT_THE_PULL_LEFT_EMPTY,
		NOTHING_MULTI_LISTED,
		NO_ACTIVITY_YET,
		NO_RUN_YET,
		PUBLISHED_WITH_CATALOGUE_WORDS,
		activityEntries,
		cadenceOptions,
		heldCadence,
		lastPullLine,
		openQuestionsLabel,
		runRows,
		syncCards,
		templateChoices,
		type SyncCard
	} from '$lib/pages/automations/sync';
	import { templates, type TemplateHead } from '$lib/pages/templates/api';
	import '$lib/pages/automations/automations.css';

	let jobs = $state<JobHead[]>([]);
	let nextCursor = $state<string | null>(null);
	let loaded = $state(false);
	// A run list that could not be read is not a seller who has never synced,
	// exactly as an unread migration list is not a seller with no migration.
	let jobsUnread = $state(false);

	let connections = $state<ConnectionView[]>([]);
	let connectionsLoaded = $state(false);
	let connectionsFailed = $state(false);

	let settings = $state<MarketplaceSyncSettingView[]>([]);
	let settingsUnread = $state(false);

	let multi = $state<MultiListedRow[]>([]);
	let multiUnread = $state(false);

	let activity = $state<ActivityLine[]>([]);
	// The `at` of the oldest line loaded, which is how this log pages: it is
	// ordered by the clock and nothing else, so an opaque token would carry
	// no more than the instant does.
	let activityCursor = $state<number | null>(null);
	let activityUnread = $state(false);

	// The templates a rule may fill a new pull from. Heads only: the picker
	// needs a name and a scope, and the draft is the server's business when it
	// applies the rule.
	let heads = $state<TemplateHead[]>([]);
	let headsUnread = $state(false);

	// Null until the figure has actually been read. A count this page failed to
	// fetch is not a count of zero, so the header control drops the figure
	// rather than claiming one.
	let openQuestions = $state<number | null>(null);

	let held = $state<Marketplace | null>(null);
	let logQuery = $state('');

	/** One card's unsaved position. Its own shape rather than the wire's,
	 *  because a card the server holds no row for still has one. */
	interface PullEdit {
		enabled: boolean;
		interval: number;
		publishTo: InventoryId[];
		/** The template a new pull is filled from, or `''` for none. Empty
		 *  string rather than null because it is a `<select>` value. */
		template: string;
	}

	// What each card holds while it is being edited, keyed by inventory. The
	// stored row is left alone until a Save answers: a card that wrote into
	// the list as the seller typed would show a setting that is not in force.
	let edits = $state<Partial<Record<InventoryId, PullEdit>>>({});
	let saving = $state<InventoryId | null>(null);
	let refusal = $state<string | null>(null);

	const entitlement = createQuery(() => entitlementRead);
	const gate = $derived(featureOf(entitlement.data, 'sync'));
	const rulesGate = $derived(featureOf(entitlement.data, 'auto_publish_rules'));
	// The plan's own floor, which a marketplace with no stored row still has
	// to be held to. Null while the entitlement is unread, which leaves every
	// cadence standing rather than disabling a paying seller's controls for as
	// long as the request takes.
	const planFloor = $derived(entitlement.data?.capabilities.sync_pull_interval_secs ?? null);

	// No count badge on this column. `jobs` holds the pages loaded so far, so a
	// figure drawn from it would mean "runs on the page you have loaded" and
	// would visibly jump when the seller pressed Load more runs. Migration's
	// column keeps its badge, because that list is a single bounded read.
	const rows = $derived(marketplaceRows(connections));
	const read = $derived(readState(connectionsLoaded, connectionsFailed, rows));
	// The resolved selection rather than the raw click, so the highlighted row
	// and the card beside it cannot name different marketplaces.
	const selected = $derived(heldSelection(rows, held));
	const cards = $derived(syncCards(connections, settings));
	// `Date.now()` inside the derivation rather than captured at init, so a
	// label recomputes with its list instead of freezing at mount.
	const runs = $derived(runRows(jobs, Date.now()));
	const log = $derived(activityEntries(activity, Date.now()));

	/** One card's editable state: what the seller has changed, or the stored
	 *  row where they have changed nothing. */
	function editOf(card: SyncCard): PullEdit {
		const stored = card.setting;
		return (
			edits[card.inventory] ?? {
				enabled: stored.enabled,
				interval: heldCadence(stored.interval_secs, stored.minimum_secs ?? planFloor),
				publishTo: [...stored.publish_to],
				template: stored.template_id ?? ''
			}
		);
	}

	function change(inventory: InventoryId, wanted: PullEdit) {
		edits = { ...edits, [inventory]: wanted };
	}

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

	async function loadActivity(cursor?: number | null) {
		try {
			const view = await api.syncActivity(cursor);
			activity = [...activity, ...view.activity];
			activityCursor = view.cursor;
			activityUnread = false;
		} catch {
			activityUnread = true;
		}
	}

	async function loadSettings() {
		try {
			const view = await api.syncSettings();
			settings = view.marketplaces;
			settingsUnread = false;
		} catch {
			settings = [];
			settingsUnread = true;
		}
	}

	$effect(() => {
		void loadPage();
		void loadActivity();
		void templates
			.list()
			.then((listed) => {
				heads = listed;
				headsUnread = false;
			})
			.catch(() => (headsUnread = true));
		void loadSettings();
		void api
			.connections()
			.then((standing) => {
				connections = standing;
				connectionsFailed = false;
			})
			.catch(() => {
				connections = [];
				connectionsFailed = true;
			})
			.finally(() => (connectionsLoaded = true));
		void api
			.multiListed()
			.then((view) => {
				multi = view.multi;
				multiUnread = false;
			})
			.catch(() => (multiUnread = true));
		void api
			.drainStats()
			.then((stats) => (openQuestions = stats.open))
			.catch(() => (openQuestions = null));
	});

	async function save(inventory: InventoryId) {
		const card = cards.find((entry) => entry.inventory === inventory);
		if (card === undefined) {
			return;
		}
		const wanted = editOf(card);
		saving = inventory;
		refusal = null;
		try {
			await api.setSyncSetting(inventory, {
				enabled: wanted.enabled,
				interval_secs: wanted.interval,
				publish_to: wanted.publishTo,
				template_id: wanted.template === '' ? null : wanted.template
			});
			// The stored row is what the card reads from once it is saved, so
			// the local edit is dropped rather than left to shadow it.
			edits = Object.fromEntries(
				Object.entries(edits).filter(([key]) => key !== inventory)
			);
			await loadSettings();
		} catch (caught) {
			refusal =
				caught instanceof ApiFailure
					? caught.message
					: 'That setting could not be saved, so it is as it was.';
		} finally {
			saving = null;
		}
	}
</script>

<div class="page">
	<PageHead
		icon="refresh-cw"
		title="Marketplace Sync"
		description="Keep every marketplace’s copy of a resource up to date."
	>
		{#snippet aside()}
			<Button href="/reconciliation" tier="outline" icon="circle-question-mark">
				{openQuestionsLabel(openQuestions)}
			</Button>
		{/snippet}
	</PageHead>

	{#if gate !== null}
		<!-- Said once with the way out. Every control below carries it too,
		     because a disabled control with no stated reason reads as a
		     fault. -->
		<Banner tone="warn" title="Pulling from your marketplaces is not on your plan" action={toPlans}>
			{gate}
		</Banner>
	{/if}

	<div class="auto-body">
		<MarketplaceList
			{rows}
			{selected}
			onselect={(marketplace) => (held = marketplace)}
			empty={columnCopy(read) ?? ''}
		/>

		<div class="auto-right">
			<Panel
				title="Pull new resources"
				description="We ask your computer to read each shop on this timetable, and anything new arrives in Import."
			>
				{#if settingsUnread}
					<p class="quiet">
						Your pull settings could not be read, so this page is showing none. Nothing has
						been changed, and any pull already set still runs.
					</p>
				{:else}
					{#each cards as card (card.inventory)}
						{@const pull = editOf(card)}
						{@const options = cadenceOptions(card.setting.minimum_secs ?? planFloor)}
						{@const why = card.connected ? gate : CONNECT_FIRST}
						<div class="pull-card" class:off={why !== null}>
							<!-- The mark alone names the card, as every other
							     marketplace heading in this console does: it carries
							     the full name as its accessible name and its tooltip,
							     and a title beside it would print the word the mark is
							     there to replace. -->
							<div class="pull-head">
								<MarketplaceMark inventory={card.inventory} size={24} />
								<span class="meta">
									{lastPullLine(card.setting.last_pull_at, Date.now())}
								</span>
							</div>

							<div class="set-toggle">
								<Toggle
									label="Pull new resources"
									checked={pull.enabled}
									disabled={why !== null}
									onchange={(value) =>
										change(card.inventory, { ...pull, enabled: value })}
								/>
							</div>

							<div class="set-grid">
								<Field
									label="How often"
									id={`cadence-${card.inventory}`}
									hint={options.find((option) => !option.enabled)?.reason ?? undefined}
								>
									<select
										id={`cadence-${card.inventory}`}
										disabled={why !== null}
										value={String(pull.interval)}
										onchange={(event) =>
											change(card.inventory, {
												...pull,
												interval: Number(event.currentTarget.value)
											})}
									>
										{#each options as option (option.secs)}
											<option
												value={String(option.secs)}
												disabled={!option.enabled}
												title={option.reason ?? undefined}
											>
												{option.label}
											</option>
										{/each}
									</select>
								</Field>
							</div>

							<p class="pull-label">Publish new pulls to</p>
							<div class="mk-tiles" role="group" aria-label={`Publish new ${card.name} pulls to`}>
								<!-- The authorable set, not every inventory: a rule
								     publishes by minting a create job, and a marketplace
								     this console cannot author to would take a tick and
								     write nothing. -->
								{#each AUTHORABLE_PLATFORMS.filter((inventory) => inventory !== card.inventory) as target (target)}
									<label class="mk-tile" title={platformTitle(target)}>
										<input
											type="checkbox"
											checked={pull.publishTo.includes(target)}
											disabled={why !== null || rulesGate !== null}
											onchange={(event) =>
												change(card.inventory, {
													...pull,
													publishTo: event.currentTarget.checked
														? [...pull.publishTo, target]
														: pull.publishTo.filter((one) => one !== target)
												})}
										/>
										<img
											class="mk-tile-mark"
											src={MARK_SRC[MARKETPLACE_OF[target]]}
											alt=""
										/>
										<span class="sr-only">
											Publish new {card.name} pulls to {platformTitle(target)}
										</span>
									</label>
								{/each}
							</div>
							{#if rulesGate !== null}
								<p class="foot-note">{rulesGate}</p>
							{/if}
							<p class="foot-note">{PUBLISHED_WITH_CATALOGUE_WORDS}</p>

							<!-- The template the rule fills a new pull from. Under the
							     publish targets rather than above them, because which
							     templates may be chosen is decided by what is ticked
							     there: a template written for one marketplace fills
							     fields nothing carries unless the rule publishes to it,
							     and the server answers 422 for exactly that. -->
							{#if headsUnread}
								<p class="foot-note">
									Your templates could not be read, so this card is offering none. Any
									template already set on this rule still runs.
								</p>
							{:else if heads.length > 0}
								{@const choices = templateChoices(heads, pull.publishTo)}
								<div class="set-grid">
									<Field label="Fill new pulls from template" id={`template-${card.inventory}`}>
										<select
											id={`template-${card.inventory}`}
											disabled={why !== null || rulesGate !== null}
											value={pull.template}
											onchange={(event) =>
												change(card.inventory, {
													...pull,
													template: event.currentTarget.value
												})}
										>
											<option value="">No template</option>
											{#each choices as choice (choice.id)}
												<option
													value={choice.id}
													disabled={choice.reason !== null}
													title={choice.reason ?? undefined}
												>
													{choice.label}{choice.reason === null
														? ''
														: ' — not a target of this rule'}
												</option>
											{/each}
										</select>
									</Field>
								</div>
								<p class="foot-note">{FILLS_WHAT_THE_PULL_LEFT_EMPTY}</p>
							{/if}

							<div class="set-foot">
								<Button
									tier="primary"
									small
									disabled={why !== null || saving === card.inventory}
									reason={why ??
										(saving === card.inventory ? 'The setting is being saved.' : undefined)}
									onclick={() => void save(card.inventory)}
								>
									{saving === card.inventory ? 'Saving…' : 'Save'}
								</Button>
								{#if why !== null}
									<p class="foot-note">{why}</p>
								{/if}
							</div>
						</div>
					{/each}
				{/if}

				{#if refusal !== null}
					<Banner tone="bad">{refusal}</Banner>
				{/if}
			</Panel>

			<Panel
				title="Listed on more than one marketplace"
				description="Where a change here has more than one copy to carry out to."
			>
				{#if multiUnread}
					<p class="quiet">This list could not be read, so it is showing none.</p>
				{:else if multi.length === 0}
					<p class="quiet">{NOTHING_MULTI_LISTED}</p>
				{:else}
					{#each multi as row (row.product)}
						<a class="multi-row" href={`/resources/${row.product}`}>
							<span class="t">{row.title}</span>
							<span class="multi-marks">
								{#each row.inventories as inventory (inventory)}
									<MarketplaceMark {inventory} size={16} />
								{/each}
							</span>
						</a>
					{/each}
				{/if}
			</Panel>

			<Panel
				title="Runs"
				description="Every update we have sent, with live progress while one is running."
			>
				{#if !loaded}
					<p class="quiet">Loading…</p>
				{:else if jobsUnread && jobs.length === 0}
					<p class="quiet">
						Your runs could not be read, so this page cannot list them. Anything already
						running is unaffected.
					</p>
				{:else if runs.length === 0}
					<Placeholder icon="refresh-cw" headline="No sync has run yet" body={NO_RUN_YET} />
				{:else}
					{#each runs as run (run.job)}
						<a class="auto-row" href={run.href}>
							<span class="who">
								<span class="t"><MarketplaceMark inventory={run.inventory} /></span>
								<span class="meta">{run.meta}</span>
							</span>
							<span class="when">{new Date(run.at).toLocaleString()}</span>
						</a>
					{/each}
					{#if jobsUnread}
						<p class="quiet">More runs could not be read. The ones above are what loaded.</p>
					{/if}
					{#if nextCursor}
						<div class="set-foot">
							<Button tier="outline" onclick={() => void loadPage(nextCursor)}>
								Load more runs
							</Button>
						</div>
					{/if}
				{/if}
			</Panel>

			<Panel title="Activity log" description="What each pull and each send did, newest first.">
				{#if activityUnread && activity.length === 0}
					<p class="quiet">
						The activity log could not be read, so it is showing nothing rather than a guess.
					</p>
				{:else}
					<ActivityLog entries={log} bind:query={logQuery} empty={NO_ACTIVITY_YET} />
					{#if activityUnread}
						<p class="quiet">More of the log could not be read. The lines above are what loaded.</p>
					{/if}
					{#if activityCursor}
						<div class="set-foot">
							<Button tier="outline" onclick={() => void loadActivity(activityCursor)}>
								Load more
							</Button>
						</div>
					{/if}
				{/if}
			</Panel>
		</div>
	</div>
</div>

{#snippet toPlans()}
	<Button tier="primary" small href="/settings/subscription">See plans</Button>
{/snippet}
