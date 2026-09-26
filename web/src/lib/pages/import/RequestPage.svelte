<script lang="ts">
	import { untrack } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { ApiFailure, api, type SyncRequestView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { createLedger, type Ledger } from '$lib/ledger';
	import PageHead from '$lib/PageHead.svelte';
	import Note from '$lib/Note.svelte';
	import Panel from '$lib/Panel.svelte';
	import Pagination from '$lib/Pagination.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { platformTitle } from '$lib/platforms';
	import StatusPill from '$lib/StatusPill.svelte';
	import {
		FILES_STAY_ON_YOUR_COMPUTER,
		MIGRATION_HREF,
		MIGRATION_NOT_ON_THIS_COMPUTER_YET,
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
	import {
		deleteRefusal,
		retainedBadge,
		type WorkDeleteOutcome,
		type WorkItem
	} from '$lib/work-delete';
	import WorkDeleteDialog from '$lib/WorkDeleteDialog.svelte';
	import { pillTone } from './import-view';
	import './import.css';

	const requestId = $derived(page.params.id ?? '');

	let view = $state<SyncRequestView | null>(null);
	let listingPage = $state(1);
	// The ordinal each page starts after. Index `n` reaches page `n + 1`; the
	// request's ordinals are the order the seller submitted, so they are
	// already a total order and there is no token to carry.
	let listingAfter = $state<(number | null)[]>([null]);
	let listingsBusy = $state(false);
	// The page a read asked for and did not get, held apart from the page on
	// screen: that one read successfully and has nothing to retry.
	let listingAttempt = $state<{ after: number | null; page: number } | null>(null);
	// Kept apart from `view` deliberately: a read that failed is not a request
	// with nothing in it, and this page exists to hold those two apart.
	let refusal = $state<string | null>(null);
	let live = $state(false);
	let ledger: Ledger | null = null;
	let generation = 0;
	// Which read the seller asked for, if one is in flight, and whether the
	// ledger wanted a refresh while it was. Plain variables: they arbitrate
	// reads and nothing renders them. The generation rather than a flag, so a
	// second Next pressed over the first leaves one navigation in charge.
	let navigatingFor = 0;
	let refreshWanted = false;

	// Nothing here asks a computer to run the work any more. The Teachouse app
	// reads a shop into Resources, which the Import screen owns; moving one
	// onto a second marketplace has no command in this version of the app, so
	// the control below states that rather than offering a press whose only
	// possible answer is a refusal.

	// One page of listings, with the request's own figures beside them. The
	// figures — the stage, the tally, the coverage — are the server's over the
	// whole request, never counted from the page: this request names every
	// listing of a shop, which is why the rows are paged at all, and a
	// headline drawn from twenty-five of five hundred rows would tell a seller
	// their finished migration had barely started.
	//
	// `asked` is the seller's own navigation, and it wins. An Import event
	// arriving while page two was in flight used to call this with page one's
	// ordinal, bump the generation and discard the page-two answer, so
	// pressing Next during a live import left the seller on page one with
	// nothing to show that it had not worked. A refresh now stands aside and
	// is coalesced into one read once the navigation lands.
	async function read(after: number | null, wanted: number, asked: boolean) {
		if (!requestId) {
			return;
		}
		const id = requestId;
		const current = ++generation;
		if (asked) {
			navigatingFor = current;
		}
		listingsBusy = true;
		try {
			const next = await api.syncRequest(id, after);
			if (current !== generation || id !== requestId) return;
			view = next;
			listingPage = wanted;
			listingAfter = [...listingAfter.slice(0, wanted), next.resources_next];
			refusal = null;
			listingAttempt = null;
		} catch (caught) {
			if (current !== generation || id !== requestId) return;
			// The page already on screen stays: this is the same distinction
			// the `refusal`/`view` split has always held, extended to a page.
			refusal =
				caught instanceof ApiFailure ? caught.message : 'It did not load. Please try again.';
			listingAttempt = { after, page: wanted };
		} finally {
			// Only the newest read owns the busy flag; a superseded one
			// clearing it would unlock the pager mid-navigation.
			if (current === generation) {
				listingsBusy = false;
			}
			// Only the navigation still in charge hands the pending refresh
			// on; one superseded by a later Next must not release it early.
			if (asked && navigatingFor === current) {
				navigatingFor = 0;
				if (refreshWanted) {
					refreshWanted = false;
					void refetch();
				}
			}
		}
	}

	function goListings(after: number | null, wanted: number) {
		void read(after, wanted, true);
	}

	function retryListingPage() {
		const attempt = listingAttempt;
		if (attempt !== null) {
			void read(attempt.after, attempt.page, true);
		}
	}

	/** This request, while a Delete is being confirmed for it, or null while
	 *  none is. The same dialog the transfer history uses: one deleted from
	 *  its own page and one deleted from the list are the same act. */
	let deleting = $state<WorkItem[] | null>(null);

	// "Transfer" rather than "migration": this record is a Copy or a Move.
	const TRANSFERS = { one: 'transfer', many: 'transfers' };

	// Return accepted deletions to history, where stopping rows remain visible.
	// Re-read other outcomes: a lost response does not prove nothing changed.
	async function settledDelete(outcome: WorkDeleteOutcome) {
		if (outcome.deleted.length > 0 || outcome.stopping.length > 0) {
			deleting = null;
			await goto(MIGRATION_HREF);
			return;
		}
		await refetch();
	}

	// The page the seller is on, read again. The ledger fires on every listing
	// the device describes, and re-reading page one would walk them off the
	// page they were reading each time the import made progress. The reads are
	// untracked because this is called from an effect that would otherwise
	// depend on state the read itself writes.
	async function refetch() {
		if (navigatingFor !== 0) {
			refreshWanted = true;
			return;
		}
		await read(
			untrack(() => listingAfter[listingPage - 1] ?? null),
			untrack(() => listingPage),
			false
		);
	}

	// The same liveness the run page beside this one uses: one event stream per
	// tab, and a refetch when the ledger moves or resyncs.
	$effect(() => {
		ledger = createLedger((cursor) => new EventSource(`/v1/events/stream?cursor=${cursor}`));
		void refetch();
		let revision = 0;
		const unsubscribe = ledger.subscribe((state) => {
			live = state.connected;
			if (state.revision === revision) return;
			revision = state.revision;
			if ([...state.kinds].some((kind) => kind === 'resync' || kind.startsWith('Import'))) {
				void refetch();
			}
		});
		return () => {
			generation += 1;
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
			icon="arrow-right-left"
			back={{ href: MIGRATION_HREF, label: 'Back to Migrations' }}
			title={`${isDeviceImport(request) ? 'Move' : 'Request'} ${request.request.slice(0, 8)}…`}
			description={`${platformTitle(request.source)} → ${platformTitle(request.target)}`}
		>
			{#snippet aside()}
				{@const going = retainedBadge(request.deletion_status)}
				<StatusPill tone={going?.tone ?? pillTone(shown.tone)} label={going?.label ?? shown.label} />
				<StatusPill tone={live ? 'ok' : 'soon'} label={live ? 'Live' : 'Reconnecting'} />
			{/snippet}
		</PageHead>

		{#if notice !== null}
			<Banner tone="warn" title="Your device needs updating">{notice}</Banner>
		{/if}

		{#if refusal !== null}
			<Banner tone="bad" title="We could not load this just now">
				{refusal} Below is what we last loaded.
			</Banner>
		{/if}

		{#if !isDeviceImport(request)}
			<Panel title="We cannot show this move here">
				<p class="quiet">{NOT_AN_IMPORT}</p>
				<div class="actions">{@render deleteRequest(request)}</div>
			</Panel>
		{:else}
			<Panel title="How this move is going">
				<p class="import-stage">{shown.headline}</p>
				{#if shown.detail !== ''}
					<p class="quiet">{shown.detail}</p>
				{/if}
				{#if canStartHere(request)}
					<div class="actions">
						<!-- Disabled rather than absent, so the page still shows what
						     would happen here and says what stands in the way. -->
						<Button
							tier="primary"
							icon="arrow-right-left"
							disabled
							reason={MIGRATION_NOT_ON_THIS_COMPUTER_YET}
						>
							Run this move on this computer
						</Button>
					</div>
				{/if}
				{#if request.create_job !== null || request.remove_job !== null}
					<div class="actions">
						{#if request.create_job !== null}
							<Button href={`/sync/${request.create_job}`}>See the new listings being made</Button>
						{/if}
						{#if request.remove_job !== null}
							<Button href={`/sync/${request.remove_job}`}>
								See the originals being removed
							</Button>
						{/if}
					</div>
				{/if}
				<div class="actions">{@render deleteRequest(request)}</div>
			</Panel>

			{#if coverage !== null}
				<Panel
					title="Words we could match"
					description="How much of this shop's wording we could match to our own lists."
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
				description="Every listing this move has reached so far, in order."
			>
				{#if refusal !== null}
					<!-- The page below is the last one that read, and the figures
					     above it are from that read too. -->
					<Banner tone="bad" title="We could not load that page of listings" action={retryListings}>
						{refusal}
					</Banner>
				{/if}
				{#if rows.length === 0}
					<p class="quiet">
						{listingPage > 1
							? 'This page is empty. Go back a page to see your listings.'
							: emptyListingsLine(stageOf(request))}
					</p>
				{:else if anyCoverage}
					<Note>Figures below read: {TERM_COVERAGE_LEGEND}.</Note>
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
				{#if rows.length > 0 || listingPage > 1}
					<Pagination
						page={listingPage}
						hasNext={request.resources_next !== null}
						busy={listingsBusy}
						label="Listings"
						summary={`${rows.length} listings on this page`}
						onprevious={() =>
							goListings(listingAfter[listingPage - 2] ?? null, listingPage - 1)}
						onnext={() => goListings(request.resources_next, listingPage + 1)}
					/>
				{/if}
				<Note icon="lock">{FILES_STAY_ON_YOUR_COMPUTER}</Note>
			</Panel>
		{/if}
	{:else if refusal !== null}
		<PageHead
			icon="arrow-right-left"
			back={{ href: MIGRATION_HREF, label: 'Back to Migrations' }}
			title="Migration"
			description="We could not load this migration."
		/>
		<Panel>
			<Placeholder icon="arrow-right-left" headline="We could not load this migration" body={refusal}>
				{#snippet actions()}
					<Button href={MIGRATION_HREF}>Back to Migrations</Button>
				{/snippet}
			</Placeholder>
		</Panel>
	{:else}
		<p class="quiet">Loading…</p>
	{/if}
</div>

<!-- Retries the page that failed rather than the page on screen: that one
     read successfully, and re-reading it would clear the failure without ever
     fetching what the seller asked for. -->
{#snippet retryListings()}
	<Button
		tier="outline"
		small
		disabled={listingsBusy}
		reason={listingsBusy ? 'Loading the page.' : undefined}
		onclick={retryListingPage}
	>
		Try again
	</Button>
{/snippet}

<!-- Delete, offered wherever this request's own actions are. Withdrawn as a
     press once it is already on its way out: the server would accept the
     call and nothing the seller can see would change. -->
{#snippet deleteRequest(request: SyncRequestView)}
	{@const gone = deleteRefusal(request.deletion_status)}
	<Button
		danger
		disabled={gone !== null}
		reason={gone ?? undefined}
		onclick={() =>
			(deleting = [
				{
					id: request.request,
					label: `${platformTitle(request.source)} → ${platformTitle(request.target)} · ${request.request.slice(0, 8)}…`
				}
			])}
	>
		Delete this transfer
	</Button>
{/snippet}

<WorkDeleteDialog
	open={deleting !== null}
	items={deleting ?? []}
	noun={TRANSFERS}
	remove={api.deleteSyncRequest}
	onClose={() => (deleting = null)}
	onsettled={(outcome) => void settledDelete(outcome)}
/>
