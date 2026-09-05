<script lang="ts">
	import { page } from '$app/state';
	import { ApiFailure, api, type SyncRequestView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { desktopInvoker, startImportHere } from '$lib/desktop';
	import { createLedger, type Ledger } from '$lib/ledger';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { platformTitle } from '$lib/platforms';
	import StatusPill from '$lib/StatusPill.svelte';
	import {
		FILES_STAY_ON_YOUR_COMPUTER,
		NOT_AN_IMPORT,
		TERM_COVERAGE_LEGEND,
		canStartHere,
		coverageOf,
		coverageRows,
		deviceUpdateNotice,
		emptyListingsLine,
		isDeviceImport,
		presentStage,
		resourceRows,
		stageOf,
		termCoverageStrip
	} from '$lib/sync-request';
	import { MIGRATION_HREF, pillTone, startRefusal } from './import-view';
	import './import.css';

	const requestId = $derived(page.params.id ?? '');

	let view = $state<SyncRequestView | null>(null);
	// Kept apart from `view` deliberately: a read that failed is not a request
	// with nothing in it, and this page exists to hold those two apart.
	let refusal = $state<string | null>(null);
	let live = $state(false);
	let ledger: Ledger | null = null;

	// Read once: whether this console is running inside the desktop application
	// does not change while the page is open.
	const invoke = desktopInvoker();
	let starting = $state(false);
	// The application's own words when it refuses, kept apart from `refusal`
	// above: one is this page failing to read the request, the other is this
	// computer declining to run it, and they are different facts.
	let declined = $state<string | null>(null);

	async function startHere() {
		if (invoke === null) {
			return;
		}
		starting = true;
		declined = null;
		const outcome = await startImportHere(invoke, requestId);
		starting = false;
		declined = startRefusal(outcome);
		if (declined !== null) {
			return;
		}
		// Started: the request moves to draining on the server, and the ledger
		// will say so, but a read now means the page does not sit on the old
		// state waiting for the first page to land.
		await refetch();
	}

	async function refetch() {
		if (!requestId) {
			return;
		}
		try {
			view = await api.syncRequest(requestId);
			refusal = null;
		} catch (caught) {
			refusal =
				caught instanceof ApiFailure ? caught.message : 'This import could not be read.';
		}
	}

	// The same liveness the run page beside this one uses: one event stream per
	// tab, and a refetch when the ledger moves or resyncs.
	$effect(() => {
		void refetch();
		ledger = createLedger((cursor) => new EventSource(`/v1/events/stream?cursor=${cursor}`));
		const unsubscribe = ledger.subscribe((state) => {
			live = state.connected;
			if (state.events.length > 0 || state.resyncs > 0) {
				void refetch();
			}
		});
		return () => {
			unsubscribe();
			ledger?.close();
		};
	});
</script>

<div class="page">
	{#if view}
		{@const request = view}
		{@const shown = presentStage(stageOf(request))}
		{@const notice = deviceUpdateNotice(request)}
		{@const coverage = coverageOf(request)}
		{@const rows = resourceRows(request)}
		{@const anyCoverage = rows.some((row) => row.coverage !== null)}
		<PageHead
			icon="download"
			back={{ href: MIGRATION_HREF, label: 'Back to Marketplace Migration' }}
			title={`${isDeviceImport(request) ? 'Import' : 'Request'} ${request.request.slice(0, 8)}…`}
			description={`${platformTitle(request.source)} → ${platformTitle(request.target)}`}
		>
			{#snippet aside()}
				<StatusPill tone={pillTone(shown.tone)} label={shown.label} />
				<StatusPill tone={live ? 'ok' : 'soon'} label={live ? 'Live' : 'Reconnecting'} />
			{/snippet}
		</PageHead>

		{#if notice !== null}
			<Banner tone="warn" title="Your device needs updating">{notice}</Banner>
		{/if}

		{#if refusal !== null}
			<Banner tone="bad" title="This import could not be read just now">
				{refusal} What is shown below is the last state that was read.
			</Banner>
		{/if}

		{#if !isDeviceImport(request)}
			<Panel title="Not an import">
				<p class="quiet">{NOT_AN_IMPORT}</p>
			</Panel>
		{:else}
			<Panel title="Where this import stands">
				<p class="import-stage">{shown.headline}</p>
				{#if shown.detail !== ''}
					<p class="quiet">{shown.detail}</p>
				{/if}
				{#if canStartHere(request) && invoke !== null}
					<div class="actions">
						<Button
							tier="primary"
							icon="download"
							disabled={starting}
							reason={starting ? 'This computer is starting the import.' : undefined}
							onclick={() => void startHere()}
						>
							{starting ? 'Starting…' : 'Start the import on this computer'}
						</Button>
					</div>
				{/if}
				{#if declined !== null}
					<Banner tone="bad" title="This computer did not start the import">{declined}</Banner>
				{/if}
				{#if request.create_job !== null || request.remove_job !== null}
					<div class="actions">
						{#if request.create_job !== null}
							<Button href={`/sync/${request.create_job}`}>Open the run that creates them</Button>
						{/if}
						{#if request.remove_job !== null}
							<Button href={`/sync/${request.remove_job}`}>
								Open the run that removes the originals
							</Button>
						{/if}
					</div>
				{/if}
			</Panel>

			{#if coverage !== null}
				<Panel
					title="Coverage"
					description="What the import measured against our own taxonomy. A zero here is a measurement, not a missing one."
				>
					<ul class="import-figures">
						{#each coverageRows(coverage) as figure (figure.label)}
							<li><span class="k">{figure.label}</span><span class="v">{figure.value}</span></li>
						{/each}
					</ul>
				</Panel>
			{/if}

			<Panel
				title="Listings"
				description="Each listing the import has reached, in the order it read them. A skipped one carries the reason it was given, in the words it was given in."
			>
				{#if rows.length === 0}
					<p class="quiet">{emptyListingsLine(stageOf(request))}</p>
				{:else if anyCoverage}
					<p class="foot-note">Figures below read: {TERM_COVERAGE_LEGEND}.</p>
				{/if}
				{#each rows as row (row.ordinal)}
					<div class="import-listing">
						<span class="mark"><StatusPill tone={pillTone(row.tone)} label={row.label} /></span>
						<span class="what">
							<span class="mono" title={row.locator}>{row.locator}</span>
							{#if row.reason !== ''}
								<span class="w">{row.reason}</span>
							{/if}
							{#if row.coverage !== null}
								<span class="w mono" title={TERM_COVERAGE_LEGEND}
									>{termCoverageStrip(row.coverage)}</span
								>
							{/if}
						</span>
						<span class="ord">#{row.ordinal}</span>
					</div>
				{/each}
				<p class="foot-note">{FILES_STAY_ON_YOUR_COMPUTER}</p>
			</Panel>
		{/if}
	{:else if refusal !== null}
		<PageHead
			icon="download"
			back={{ href: MIGRATION_HREF, label: 'Back to Marketplace Migration' }}
			title="Import"
			description="This import could not be read."
		/>
		<Panel>
			<Placeholder icon="download" headline="This import could not be read" body={refusal}>
				{#snippet actions()}
					<Button href={MIGRATION_HREF}>Back to Marketplace Migration</Button>
				{/snippet}
			</Placeholder>
		</Panel>
	{:else}
		<p class="quiet">Loading the import…</p>
	{/if}
</div>
