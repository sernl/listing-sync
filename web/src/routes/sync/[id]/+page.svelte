<script lang="ts">
	import { untrack } from 'svelte';
	import { page } from '$app/state';
	import { ApiFailure, api, type ItemDetail, type ItemView, type JobView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { agoLabel } from '$lib/elapsed';
	import { gateLabel } from '$lib/gates';
	import { createLedger, type Ledger } from '$lib/ledger';
	import { segments } from '$lib/outcome';
	import PageHead from '$lib/PageHead.svelte';
	import Pagination from '$lib/Pagination.svelte';
	import Panel from '$lib/Panel.svelte';
	import { platformTitle } from '$lib/platforms';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import '$lib/pages/automations/automations.css';

	const jobId = $derived(page.params.id ?? '');

	// How many items one page of a run shows, and how many steps one page of
	// an item's timeline shows. A five-hundred-item bulk used to arrive as one
	// document and grow from there.
	const ITEMS_PER_PAGE = 25;
	const STEPS_PER_PAGE = 25;

	let job = $state<JobView | null>(null);
	let items = $state<ItemView[]>([]);
	let itemPage = $state(1);
	// One cursor per page reached, index `n` reaching page `n + 1`. The tokens
	// are the server's; this page mints none.
	let itemCursors = $state<(string | null)[]>([null]);
	let itemNext = $state<string | null>(null);
	let itemsBusy = $state(false);
	let itemsUnread = $state(false);

	/** A page a read asked for and did not get: what Retry asks for again,
	 *  held apart from the page on screen, which read successfully. */
	interface Attempt {
		cursor: string | null;
		page: number;
	}

	let itemAttempt = $state<Attempt | null>(null);

	/** One open item's timeline: the page in hand, the cursors that reach it,
	 *  and whatever went wrong last. Kept per item because two can be open at
	 *  once.
	 *
	 *  `detail` is null until the first page arrives, so a disclosure whose
	 *  first read failed still renders — as the failure and a Retry. It used
	 *  to write nothing at all in that case, and opening the item looked like
	 *  a control that did nothing. */
	interface Steps {
		detail: ItemDetail | null;
		page: number;
		cursors: (number | null)[];
		busy: boolean;
		failure: string | null;
		attempt: { cursor: number | null; page: number } | null;
	}

	let opened = $state<Record<string, Steps>>({});
	let live = $state(false);
	let ledger: Ledger | null = null;
	let generation = 0;
	// Which read the seller asked for, if one is in flight, and whether the
	// ledger wanted a refresh while it was. Plain variables rather than state:
	// they arbitrate reads and nothing renders them. The generation rather than
	// a flag, so a second Next pressed over the first leaves exactly one
	// navigation in charge.
	let navigatingFor = 0;
	let refreshWanted = false;

	/** One page of items, either because the seller asked for it or because
	 *  the ledger moved.
	 *
	 *  The distinction is load-bearing. A navigation must win: an Item event
	 *  arriving while page two is in flight used to call this with page one's
	 *  cursor, bump the generation, and discard the page-two answer — so
	 *  pressing Next during an active run left the seller on page one with no
	 *  sign that anything had happened. A refresh now stands aside and is
	 *  coalesced into one read after the navigation lands. */
	async function readItems(cursor: string | null, wanted: number, asked: boolean) {
		if (!jobId) {
			return;
		}
		const id = jobId;
		const current = ++generation;
		if (asked) {
			navigatingFor = current;
		}
		itemsBusy = true;
		try {
			const [next, listed] = await Promise.all([
				api.job(id),
				api.items(id, cursor, ITEMS_PER_PAGE)
			]);
			if (current !== generation || id !== jobId) return;
			job = next;
			items = listed.items;
			itemNext = listed.next_cursor;
			itemPage = wanted;
			itemCursors = [...itemCursors.slice(0, wanted), listed.next_cursor];
			itemsUnread = false;
			itemAttempt = null;
		} catch {
			if (current !== generation || id !== jobId) return;
			// The rows on screen stay, and the page that failed is remembered
			// so Retry asks for that one rather than for the page already
			// displayed. A failed background refresh is said too: a live view
			// that has quietly stopped following the run is worse than one
			// that admits it.
			itemsUnread = true;
			itemAttempt = { cursor, page: wanted };
		} finally {
			// Only the newest read owns the busy flag; a superseded one
			// clearing it would unlock the controls mid-navigation.
			if (current === generation) {
				itemsBusy = false;
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

	// The page the seller is looking at, read again.
	//
	// The ledger fires on every item the engine settles, and this is what it
	// calls: re-reading page one would walk a seller off the page they were
	// reading every time a run made progress. The cursor is read untracked
	// because this runs from an effect, and an effect that read the cursor
	// ledger its own read writes would re-enter until Svelte stopped it.
	async function refetch() {
		if (navigatingFor !== 0) {
			// One pending refresh, not one per event: the ledger can move
			// several times while a page is in flight and they all want the
			// same read.
			refreshWanted = true;
			return;
		}
		await readItems(
			untrack(() => itemCursors[itemPage - 1] ?? null),
			untrack(() => itemPage),
			false
		);
	}

	function goItems(cursor: string | null, page: number) {
		void readItems(cursor, page, true);
	}

	function retryItemPage() {
		const attempt = itemAttempt;
		if (attempt !== null) {
			void readItems(attempt.cursor, attempt.page, true);
		}
	}

	$effect(() => {
		ledger = createLedger((cursor) => new EventSource(`/v1/events/stream?cursor=${cursor}`));
		void refetch();
		let revision = 0;
		const unsubscribe = ledger.subscribe((state) => {
			live = state.connected;
			if (state.revision === revision) return;
			revision = state.revision;
			if ([...state.kinds].some((kind) => kind === 'resync' || kind.startsWith('Job') || kind.startsWith('Item'))) {
				void refetch();
			}
		});
		return () => {
			generation += 1;
			unsubscribe();
			ledger?.close();
		};
	});

	async function expand(item: ItemView) {
		if (opened[item.item]) {
			const next = { ...opened };
			delete next[item.item];
			opened = next;
			return;
		}
		// Opened before the read answers, so the disclosure is visibly busy
		// and a first read that fails has somewhere to report itself.
		opened = {
			...opened,
			[item.item]: {
				detail: null,
				page: 1,
				cursors: [null],
				busy: true,
				failure: null,
				attempt: null
			}
		};
		await readSteps(item.item, null, 1);
	}

	// One page of one item's steps, replacing whatever page was shown for it.
	// The cursor is the step's own `org_seq`, which is the order the ledger
	// allocates and therefore the order the timeline is already in.
	async function readSteps(item: string, cursor: number | null, wanted: number) {
		const held = opened[item];
		if (held === undefined) {
			return;
		}
		opened = { ...opened, [item]: { ...held, busy: true } };
		try {
			const detail = await api.item(jobId, item, cursor, STEPS_PER_PAGE);
			opened = {
				...opened,
				[item]: {
					detail,
					page: wanted,
					cursors: [...held.cursors.slice(0, wanted), detail.events_next],
					busy: false,
					failure: null,
					attempt: null
				}
			};
		} catch (caught) {
			// The steps already shown stay shown, and the failure is said
			// beside them with the page it was for: a swallowed refusal left
			// a disclosure that had simply stopped answering.
			opened = {
				...opened,
				[item]: {
					...held,
					busy: false,
					failure:
						caught instanceof ApiFailure
							? caught.message
							: 'These steps could not be read.',
					attempt: { cursor, page: wanted }
				}
			};
		}
	}

	function retrySteps(item: string) {
		const attempt = opened[item]?.attempt;
		if (attempt !== undefined && attempt !== null) {
			void readSteps(item, attempt.cursor, attempt.page);
		}
	}

	// What is on screen, which is one page of the timeline rather than the
	// whole of it. The control is named for that: a file called the item's
	// result that held twenty-five of its ninety steps would be read as the
	// whole record of a failure.
	function download(detail: ItemDetail) {
		const blob = new Blob([JSON.stringify(detail, null, 2)], { type: 'application/json' });
		const url = URL.createObjectURL(blob);
		const anchor = document.createElement('a');
		anchor.href = url;
		anchor.download = `item-${detail.item.slice(0, 8)}.json`;
		anchor.click();
		URL.revokeObjectURL(url);
		toast('info', 'These steps were downloaded.');
	}
</script>

<div class="page">
	{#if job}
		{@const run = job}
		<PageHead
			icon="refresh-cw"
			back={{ href: '/sync', label: 'Back to Marketplace Sync' }}
			title="Marketplace run"
			description={`${platformTitle(run.inventory)} · started ${agoLabel(run.created_at, Date.now())}`}
		>
			{#snippet aside()}
				<StatusPill
					tone={run.phase === 'settled' ? 'ok' : 'run'}
					label={run.phase}
				/>
				<StatusPill
					tone={live ? 'ok' : 'soon'}
					label={live ? 'Updates connected' : 'Reconnecting to updates'}
				/>
			{/snippet}
		</PageHead>

		<Panel
			title="Outcomes"
			description="Every outcome we recorded, kept apart rather than rolled into one."
		>
			<div class="run-outcome" role="img" aria-label="outcome distribution">
				{#each segments(run.counts) as segment (segment.label)}
					<div
						class={segment.tone}
						style="width: {segment.share * 100}%"
						title="{segment.label}: {segment.count}"
					></div>
				{/each}
			</div>
			<p class="foot-note">
				{#each segments(run.counts) as segment (segment.label)}
					{segment.label}
					{segment.count} ·
				{/each}
				total {run.counts.total}
			</p>
			<details class="foot-note">
				<summary>Run reference</summary>
				<code>{run.job}</code>
			</details>
		</Panel>

		<Panel title="Items" description="Open one to see its steps and its result.">
			{#if itemsUnread}
				<Banner tone="bad" action={retryItems}>
					That page of items could not be read, so the items below are the last ones that
					did. Anything still running is unaffected.
				</Banner>
			{/if}
			{#if items.length === 0}
				<p class="quiet">
					{itemPage > 1
						? 'There are no items on this page. Go back for the ones before it.'
						: 'No items recorded on this run yet.'}
				</p>
			{/if}
			{#each items as item, index (item.item)}
				<div class="run-item">
					<button
						class="run-item-head"
						aria-expanded={opened[item.item] !== undefined}
						onclick={() => expand(item)}
					>
						<span>Resource {(itemPage - 1) * ITEMS_PER_PAGE + index + 1}</span>
						<StatusPill tone="soon" label={item.state} />
						{#if item.outcome}
							<StatusPill tone="soon" label={item.outcome} />
						{/if}
						{#if item.blocked_on}
							<StatusPill tone="run" label={`blocked on ${gateLabel(item.blocked_on)}`} />
						{/if}
						{#if item.failure_code}
							<StatusPill tone="bad" label="Needs attention" />
						{/if}
						<span class="grow"></span>
					</button>
					{#if opened[item.item]}
						{@const steps = opened[item.item]}
						<div class="run-detail">
							<p class="foot-note">
								Resource reference: <code>{item.item}</code> · Attempts: {item.attempt_count}
								{#if item.failure_code}
									· Failure reference: <code>{item.failure_code}</code>
								{/if}
							</p>
							<div class="head-row">
								<h3>Step timeline</h3>
								<span class="grow"></span>
								{#if steps.detail !== null}
									{@const held = steps.detail}
									<Button tier="outline" small onclick={() => download(held)}>
										Download these steps
									</Button>
								{/if}
							</div>
							<!-- The failure is said beside whatever is already on
							     screen, with a Retry for the page it was for: a
							     disclosure that silently stopped answering reads as a
							     control that does nothing. -->
							{#if steps.failure !== null}
								<Banner tone="bad">
									{steps.failure}
									{#snippet action()}
										<Button
											tier="outline"
											small
											disabled={steps.busy}
											reason={steps.busy ? 'The steps are being read.' : undefined}
											onclick={() => retrySteps(item.item)}
										>
											Retry
										</Button>
									{/snippet}
								</Banner>
							{/if}
							<!-- The narrowing is on the positive branch, so the page in
							     hand is a value rather than a maybe-null read twice. -->
							{#if steps.detail !== null}
								{@const detail = steps.detail}
								{#if detail.events.length === 0}
									<p class="quiet">
										{steps.page > 1
											? 'No steps on this page. Go back for the earlier ones.'
											: 'No steps recorded yet.'}
									</p>
								{:else}
									<ol class="run-steps">
										{#each detail.events as event (event.org_seq)}
											<li>
												<span class="mono">#{event.org_seq}</span>
												<b>{event.kind}</b>
												<span class="s">
													{new Date(event.created_at).toLocaleTimeString()}
												</span>
											</li>
										{/each}
									</ol>
								{/if}
								{#if detail.events.length > 0 || steps.page > 1}
									<!-- A retried item records a step per attempt, so a
									     timeline is the largest thing on this page. It is
									     read in order, oldest first, and the pager keeps
									     that order while bounding what is drawn. -->
									<Pagination
										page={steps.page}
										hasNext={detail.events_next !== null}
										busy={steps.busy}
										label="Step timeline"
										summary={`${detail.events.length} steps on this page`}
										onprevious={() =>
											void readSteps(
												item.item,
												steps.cursors[steps.page - 2] ?? null,
												steps.page - 1
											)}
										onnext={() =>
											void readSteps(item.item, detail.events_next, steps.page + 1)}
									/>
								{/if}
								{#if detail.failure_detail}
									<p class="run-refusal">{detail.failure_detail}</p>
								{/if}
							{:else if steps.failure === null}
								<p class="quiet">Reading the steps…</p>
							{/if}
						</div>
					{/if}
				</div>
			{/each}
			{#if items.length > 0 || itemPage > 1}
				<Pagination
					page={itemPage}
					hasNext={itemNext !== null}
					busy={itemsBusy}
					label="Items"
					summary={`${items.length} items on this page`}
					onprevious={() => goItems(itemCursors[itemPage - 2] ?? null, itemPage - 1)}
					onnext={() => goItems(itemNext, itemPage + 1)}
				/>
			{/if}
		</Panel>
	{:else}
		<p class="quiet">Loading the run…</p>
	{/if}
</div>

<!-- Retries the page that failed, which the read remembers apart from the
     page on screen. -->
{#snippet retryItems()}
	<Button
		tier="outline"
		small
		disabled={itemsBusy}
		reason={itemsBusy ? 'A page is being read.' : undefined}
		onclick={retryItemPage}
	>
		Retry
	</Button>
{/snippet}
