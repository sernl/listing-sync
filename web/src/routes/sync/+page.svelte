<script lang="ts">
	import ActivityLog from '$lib/ActivityLog.svelte';
	import { api, type ConnectionView, type JobHead } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import type { Marketplace } from '$lib/generated/vocab';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Toggle from '$lib/Toggle.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import MarketplaceList from '$lib/pages/automations/MarketplaceList.svelte';
	import { heldSelection, marketplaceRows } from '$lib/pages/automations/marketplace-list';
	import { columnCopy, panelCopy, readState } from '$lib/pages/automations/read-state';
	import {
		CADENCE_OPTIONS,
		NO_RUN_YET,
		SCHEDULE_BODY,
		SCHEDULE_NOT_SETTABLE,
		SCHEDULE_TITLE,
		SETTINGS_ARE_A_PREVIEW,
		openQuestionsLabel,
		runLog,
		runRows
	} from '$lib/pages/automations/sync';
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

	// Null until the figure has actually been read. A count this page failed to
	// fetch is not a count of zero, so the header control drops the figure
	// rather than claiming one.
	let openQuestions = $state<number | null>(null);

	let held = $state<Marketplace | null>(null);
	let logQuery = $state('');

	// No count badge on this column. `jobs` holds the pages loaded so far, so a
	// figure drawn from it would mean "runs on the page you have loaded" and
	// would visibly jump when the seller pressed Load more runs. Migration's
	// column keeps its badge, because that list is a single bounded read.
	const rows = $derived(marketplaceRows(connections));
	const read = $derived(readState(connectionsLoaded, connectionsFailed, rows));
	const copy = $derived(panelCopy(read, 'Sync'));
	// The resolved selection rather than the raw click, so the highlighted row
	// and the card beside it cannot name different marketplaces.
	const selected = $derived(heldSelection(rows, held));
	const shown = $derived(rows.find((row) => row.marketplace === selected) ?? null);
	// `Date.now()` inside the derivation rather than captured at init, so a
	// label recomputes with its list instead of freezing at mount.
	const runs = $derived(runRows(jobs, Date.now()));
	const log = $derived(runLog(jobs, Date.now()));

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

	$effect(() => {
		void loadPage();
		void api
			.connections()
			.then((view) => {
				connections = view.connections;
				connectionsFailed = false;
			})
			.catch(() => {
				connections = [];
				connectionsFailed = true;
			})
			.finally(() => (connectionsLoaded = true));
		void api
			.drainStats()
			.then((stats) => (openQuestions = stats.open))
			.catch(() => (openQuestions = null));
	});
</script>

<div class="page">
	<PageHead
		icon="refresh-cw"
		title="Marketplace Sync"
		description="Keep each marketplace’s copy of a resource agreeing with your catalogue."
	>
		{#snippet aside()}
			<Button href="/reconciliation" tier="outline" icon="circle-question-mark">
				{openQuestionsLabel(openQuestions)}
			</Button>
		{/snippet}
	</PageHead>

	<div class="auto-body">
		<MarketplaceList {rows} {selected} onselect={(marketplace) => (held = marketplace)}
			empty={columnCopy(read) ?? ''} />

		<div class="auto-right">
			{#if copy !== null}
				<Panel title={copy.title}>
					{#if copy.body}<p class="quiet">{copy.body}</p>{/if}
				</Panel>
			{:else if shown !== null}
				<Panel title={shown.name} description={SETTINGS_ARE_A_PREVIEW}>
					<Banner tone="info" title={SCHEDULE_TITLE}>{SCHEDULE_BODY}</Banner>

					<div class="set-grid">
						<div class="set-toggle">
							<Toggle label="Include this marketplace" checked={false} disabled />
						</div>

						<Field label="Check for changes" id="sync-cadence" hint={SCHEDULE_NOT_SETTABLE}>
							<select id="sync-cadence" disabled>
								{#each CADENCE_OPTIONS as option (option)}
									<option>{option}</option>
								{/each}
							</select>
						</Field>

						<div class="set-toggle">
							<Toggle
								label="Ask me before changing a listing that is already live"
								checked={false}
								disabled
							/>
						</div>
					</div>

					<div class="set-foot">
						<Button tier="primary" disabled reason={SCHEDULE_NOT_SETTABLE}>
							Save schedule
						</Button>
					</div>
				</Panel>
			{/if}

			<Panel
				title="Runs"
				description="Queued and completed runs, with live progress while one is under way."
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
								<span class="t">{run.title}</span>
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

			<Panel title="Activity log" description="What each run did, newest first.">
				<ActivityLog
					entries={log}
					bind:query={logQuery}
					empty="No sync has run yet, so there is nothing to log."
				/>
			</Panel>
		</div>
	</div>
</div>
