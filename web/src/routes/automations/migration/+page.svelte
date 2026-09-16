<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import ActivityLog from '$lib/ActivityLog.svelte';
	import {
		ApiFailure,
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
	import Pagination from '$lib/Pagination.svelte';
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
	import {
		countWord,
		deleteRefusal,
		keptSelection,
		retainedBadge,
		type WorkDeleteOutcome,
		type WorkItem
	} from '$lib/work-delete';
	import WorkDeleteDialog from '$lib/WorkDeleteDialog.svelte';
	import MarketplaceList from '$lib/pages/automations/MarketplaceList.svelte';
	import { heldSelection, marketplaceRows } from '$lib/pages/automations/marketplace-list';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import {
		NO_MIGRATION_YET,
		WHAT_A_MIGRATION_IS,
		migrationLog,
		migrationRows,
		pageSelection,
		type MigrationRow
	} from '$lib/pages/automations/migration';
	import '$lib/pages/automations/automations.css';

	// How many migrations one page shows. Ten, because a past migration is a
	// heading a seller scans for the one they are looking for.
	const PER_PAGE = 10;
	// How many resources the tick list shows at once.
	const PICK_PER_PAGE = 25;

	/** A page a read asked for and did not get: what Retry asks for again.
	 *  Held apart from the displayed page, which stays on the last page that
	 *  actually read. */
	interface Attempt {
		cursor: string | null;
		page: number;
	}

	let requests = $state<SyncRequestHead[]>([]);
	let requestPage = $state(1);
	// One cursor per page reached, index `n` reaching page `n + 1`. The server
	// mints them; Previous steps back through the ones it issued rather than
	// re-walking from the newest migration.
	let requestCursors = $state<(string | null)[]>([null]);
	let requestNext = $state<string | null>(null);
	let requestsBusy = $state(false);
	let requestsLoaded = $state(false);
	// A migration list that could not be read is not a seller who has brought
	// no shop across. Told the second when the first is true, they start the
	// migration again. A page that fails therefore leaves the last page that
	// read on screen rather than emptying the list.
	let requestsUnread = $state(false);
	let requestAttempt = $state<Attempt | null>(null);
	/** Which dispositions the history shows: both, or one of them. The
	 *  seller's own choice, empty for both, and applied by the server. Held
	 *  apart from `disposition` above, which is what a *new* transfer would
	 *  be. */
	let historyKind = $state<Disposition | ''>('');

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

	// The catalogue a page at a time, by the cursor the products endpoint
	// already mints. It used to be `allPages`, which walked every page of the
	// shop before the tick list drew anything: a five-hundred-resource seller
	// waited for five hundred rows to arrive and then scrolled five hundred
	// checkboxes. Selection is held by resource id and the whole-shop choice
	// is its own explicit option, so neither depends on which page is loaded.
	let products = $state<ProductHead[]>([]);
	let pickPage = $state(1);
	let pickCursors = $state<(string | null)[]>([null]);
	let pickNext = $state<string | null>(null);
	let pickBusy = $state(false);
	let pickLoaded = $state(false);
	let pickUnread = $state(false);
	let pickAttempt = $state<Attempt | null>(null);

	const mappings = createQuery(() => ({
		queryKey: queryKeys.mappings,
		queryFn: () => api.mappings().then((view) => view.mappings)
	}));

	const marksOf = $derived.by(() => {
		const index = new Map<string, InventoryId[]>();
		for (const mapping of mappings.data ?? []) {
			index.set(mapping.product, [...(index.get(mapping.product) ?? []), mapping.inventory]);
		}
		return index;
	});
	const query = $derived(normaliseQuery(box).toLocaleLowerCase());
	// This narrows the page in hand and says so where the seller reads it.
	// The products endpoint takes a cursor and a label and no text, so a box
	// that claimed to search the shop would be searching twenty-five rows of
	// it — and a seller who found nothing would conclude the resource is not
	// there. The whole-shop option above is what covers the shop.
	const shown = $derived(
		query.length === 0
			? products
			: products.filter((product) => product.title.toLocaleLowerCase().includes(query))
	);
	const allShownTicked = $derived(
		shown.length > 0 && shown.every((product) => ticked.has(product.id))
	);
	// How many of the seller's ticks are not on the page they are looking at.
	// Said plainly, because a count of ticks that only matched the visible
	// rows would make a selection look lost the moment the page turned.
	const tickedOffPage = $derived(
		[...ticked].filter((id) => !products.some((product) => product.id === id)).length
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

	// No count badge on this column. The figure used to be drawn from the
	// whole request list, which was one bounded read; it is a page now, so a
	// count taken from it would mean "migrations on the page you are looking
	// at" and would read as a per-marketplace total.
	const rows = $derived(marketplaceRows(connections));
	// The resolved selection rather than the raw click, so the highlighted row
	// and the card beside it cannot name different marketplaces.
	const selected = $derived(heldSelection(rows, held));
	// `Date.now()` inside the derivation rather than captured at init, so a
	// label recomputes with its list instead of freezing at mount.
	const past = $derived(migrationRows(requests, Date.now()));
	const log = $derived(migrationLog(requests, Date.now()));

	/** What the seller has ticked in the migration history: the request's id
	 *  against the words a confirmation names it by. A map rather than a set
	 *  of ids, because the selection survives turning the page and a row that
	 *  is no longer on screen still has to be nameable by the dialog that is
	 *  about to delete it.
	 *
	 *  Kept well apart from `ticked` above, which is the resources a *new*
	 *  migration would carry. */
	let pickedPast = $state<Map<string, string>>(new Map());
	/** The migrations a Delete is being confirmed for, or null while none is. */
	let deleting = $state<WorkItem[] | null>(null);

	// "Transfer" rather than "migration": this history holds Copies and Moves,
	// and both are a transfer between two shops.
	const TRANSFERS = { one: 'transfer', many: 'transfers' };
	// One already stopping or kept for review is not something a second
	// Delete could move, so it is not offered as one.
	const pickable = $derived(past.filter((row) => row.deletion === null));
	const allPickedHere = $derived(
		pickable.length > 0 && pickable.every((row) => pickedPast.has(row.request))
	);
	const pickedItems = $derived([...pickedPast].map(([id, label]) => ({ id, label })));
	const pickedElsewhere = $derived(
		[...pickedPast.keys()].filter((id) => !past.some((row) => row.request === id)).length
	);

	/** How one migration is named where the row's two marks cannot be: in a
	 *  confirmation, which is text. */
	function pastLabel(row: MigrationRow): string {
		return `${SHORT_NAME[row.source]} → ${SHORT_NAME[row.target]} · ${row.meta}`;
	}

	function pickPast(row: MigrationRow, on: boolean) {
		const next = new Map(pickedPast);
		if (on) {
			next.set(row.request, pastLabel(row));
		} else {
			next.delete(row.request);
		}
		pickedPast = next;
	}

	/** Tick or untick the page in hand, leaving other pages' ticks alone. */
	function pickPastPage() {
		const next = new Map(pickedPast);
		for (const row of pickable) {
			if (allPickedHere) {
				next.delete(row.request);
			} else {
				next.set(row.request, pastLabel(row));
			}
		}
		pickedPast = next;
	}

	/** What one Delete settled: the rows the server answered about are
	 *  unticked, the refusals stay ticked, and the page in hand is read again
	 *  by the cursor it came from rather than reset to the newest
	 *  migration. */
	function settled(outcome: WorkDeleteOutcome) {
		const kept = keptSelection(new Set(pickedPast.keys()), outcome);
		pickedPast = new Map([...pickedPast].filter(([id]) => kept.has(id)));
		void readRequests(requestCursors[requestPage - 1] ?? null, requestPage);
	}

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

	let requestsRead = 0;

	/** One page of the transfer history, narrowed by the server and never by
	 *  this page.
	 *
	 *  Both dispositions by default. This used to ask for `migrate` only,
	 *  which is Move: a seller whose transfers were all Copies — the default
	 *  this screen offers — found their own history empty and had no way to
	 *  delete anything from it. `historyKind` narrows it through the
	 *  endpoint's own query, so a page of ten is ten of what was asked for
	 *  rather than ten mixed rows filtered down to whatever survived. */
	async function readRequests(cursor: string | null, page: number) {
		const generation = ++requestsRead;
		requestsBusy = true;
		const asked = historyKind;
		try {
			const view = await api.syncRequests({
				cursor,
				limit: PER_PAGE,
				disposition: asked === '' ? undefined : asked
			});
			if (generation !== requestsRead) return;
			requests = view.requests;
			requestNext = view.next_cursor;
			requestPage = page;
			requestCursors = [...requestCursors.slice(0, page), view.next_cursor];
			requestsUnread = false;
			requestAttempt = null;
		} catch {
			if (generation !== requestsRead) return;
			// The page on screen and the page number both stay, and the page
			// that failed is remembered apart from them: an emptied list would
			// read as "you have never moved a shop", and a Retry that asked
			// for the page still on screen would clear the failure without
			// ever fetching the page the seller pressed Next for.
			requestsUnread = true;
			requestAttempt = { cursor, page };
		} finally {
			if (generation === requestsRead) {
				requestsBusy = false;
				requestsLoaded = true;
			}
		}
	}

	/** Narrow the history to Copies, to Moves, or to neither.
	 *
	 *  Back to the first page and a fresh cursor ledger: the server's tokens
	 *  are cut against the filter they were issued under, so page two of
	 *  "everything" is not page two of "Moves". */
	function narrowHistory() {
		requestCursors = [null];
		requestNext = null;
		requestAttempt = null;
		void readRequests(null, 1);
	}

	function retryRequestPage() {
		const attempt = requestAttempt;
		if (attempt !== null) {
			void readRequests(attempt.cursor, attempt.page);
		}
	}

	async function readProducts(cursor: string | null, page: number) {
		pickBusy = true;
		try {
			const view = await api.products(cursor, null, PICK_PER_PAGE);
			products = view.products;
			pickNext = view.next_cursor;
			pickPage = page;
			pickCursors = [...pickCursors.slice(0, page), view.next_cursor];
			pickUnread = false;
			pickAttempt = null;
		} catch {
			pickUnread = true;
			pickAttempt = { cursor, page };
		} finally {
			pickBusy = false;
			pickLoaded = true;
		}
	}

	function retryPickPage() {
		const attempt = pickAttempt;
		if (attempt !== null) {
			void readProducts(attempt.cursor, attempt.page);
		}
	}

	onMount(() => {
		void readRequests(null, 1);
		void readProducts(null, 1);
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

	// Only the rows on screen move; the rule is `pageSelection`, which is
	// tested apart from this component because a migration's contents depend
	// on it. It used to replace the selection with the visible ids, which was
	// the same thing while the tick list was the whole catalogue and is silent
	// data loss now that it is a page.
	function toggleAllShown() {
		ticked = pageSelection(
			ticked,
			shown.map((product) => product.id),
			allShownTicked
		);
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
						{#if !pickLoaded}
							<p class="quiet">Loading your resources…</p>
						{:else if pickUnread && products.length === 0}
							<Banner tone="bad" action={retryPicks}>
								Your resources could not be read, so there is nothing to tick. Moving all
								of {SHORT_NAME[source]} does not need this list and still works.
							</Banner>
						{:else if products.length === 0 && pickPage === 1}
							<p class="quiet">
								You have no resources yet, so there is nothing to tick. Import brings your
								existing shop across first.
							</p>
						{:else}
							{#if pickUnread}
								<Banner tone="bad" action={retryPicks}>
									That page of your resources could not be read, so the rows below are
									the last ones that did. Nothing you have ticked has been lost.
								</Banner>
							{/if}
							<div class="pick-head">
								<!-- "on this page", because that is what it does. The
								     resources endpoint takes a cursor and a label and no
								     text, so a box labelled "Search resources" would
								     search twenty-five rows and answer "nothing" about a
								     resource sitting on page four. -->
								<input
									type="search"
									aria-label="Filter the resources on this page"
									placeholder="Filter this page"
									bind:value={box}
								/>
								<label class="pick-all">
									<input
										type="checkbox"
										checked={allShownTicked}
										onchange={toggleAllShown}
									/>
									Select these {shown.length}
								</label>
								<!-- The ticks that are not on this page are counted and
								     said, because a selection that looks smaller after a
								     page turn reads as a selection that was dropped. -->
								<span class="pick-count" role="status" aria-live="polite">
									{ticked.size} chosen{tickedOffPage > 0
										? `, including ${tickedOffPage} not on this page`
										: ''}
								</span>
							</div>

							{#if shown.length === 0}
								<p class="quiet">
									{products.length === 0
										? 'There are no resources on this page. Go back for the ones before it.'
										: 'Nothing on this page matches that. Clear the filter, or try Next for more resources.'}
								</p>
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
							<!-- Turning the page changes nothing about the selection:
							     `ticked` is keyed by resource id and the whole-shop
							     choice is the separate option above, so neither is
							     derived from the rows in hand. -->
							<Pagination
								page={pickPage}
								hasNext={pickNext !== null}
								busy={pickBusy}
								label="Your resources"
								summary={`${products.length} resources on this page`}
								onprevious={() =>
									void readProducts(pickCursors[pickPage - 2] ?? null, pickPage - 1)}
								onnext={() => void readProducts(pickNext, pickPage + 1)}
							/>
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

			<!-- The list and the log are one page of the same read: the log is
			     this page's migrations said as lines, so the two cannot
			     disagree about what the seller is looking at, and turning the
			     page turns both. -->
			<Panel
				title="Your transfers"
				description="Every transfer you have run, Copy and Move together, newest first."
			>
				<!-- The filter is the endpoint's own `disposition` query, not a
				     sieve over the page: a page of ten filtered in the browser
				     would answer "the Copies among the newest ten". -->
				{#snippet more()}
					<Field label="Show" id="history-kind">
						<select id="history-kind" bind:value={historyKind} onchange={narrowHistory}>
							<option value="">Copies and Moves</option>
							<option value="sync">{DISPOSITION_WORD.sync} only</option>
							<option value="migrate">{DISPOSITION_WORD.migrate} only</option>
						</select>
					</Field>
				{/snippet}
				{#if !requestsLoaded}
					<p class="quiet">Loading…</p>
				{:else if requestsUnread && requests.length === 0}
					<Banner tone="bad" action={retryRequests}>
						Your transfers could not be read, so this page cannot list them. Any transfer
						already running is unaffected.
					</Banner>
				{:else}
					{#if requestsUnread}
						<Banner tone="bad" action={retryRequests}>
							That page could not be read, so the transfers below are the last ones that
							did.
						</Banner>
					{/if}
					{#if past.length === 0}
						<!-- "Nothing on this page", "nothing of this kind" and "you
						     have never transferred a shop" are three different facts. -->
						<p class="quiet">
							{requestPage > 1
								? 'There is nothing left on this page. Go back for the transfers before it.'
								: historyKind !== ''
									? `No ${DISPOSITION_WORD[historyKind].toLocaleLowerCase()} has run yet. Show both to see the rest.`
									: NO_MIGRATION_YET}
						</p>
					{:else}
						<!-- The tick for the page in hand and whatever the seller has
						     ticked elsewhere. In the flow above the rows, because a bar
						     pinned to a phone's viewport covers the row it acts on. -->
						<div class="work-bar">
							<label class="work-pick-all">
								<input
									type="checkbox"
									checked={allPickedHere}
									disabled={pickable.length === 0}
									onchange={pickPastPage}
								/>
								Select the {countWord(pickable.length, TRANSFERS)} on this page
							</label>
							{#if pickedPast.size > 0}
								<span class="work-picked">
									{countWord(pickedPast.size, TRANSFERS)} selected{pickedElsewhere > 0
										? `, ${pickedElsewhere} of them on another page`
										: ''}
								</span>
								<div class="work-bar-acts">
									<Button small tier="quiet" onclick={() => (pickedPast = new Map())}>
										Clear selection
									</Button>
									<Button small danger onclick={() => (deleting = pickedItems)}>
										Delete {countWord(pickedPast.size, TRANSFERS)}
									</Button>
								</div>
							{/if}
						</div>

						{#each past as row (row.request)}
							{@const going = retainedBadge(row.deletion)}
							{@const refusal = deleteRefusal(row.deletion)}
							<!-- A row rather than one whole-row anchor: it carries a tick
							     and a Delete, and a control nested in a link is reached by
							     the keyboard as part of the link and a press activates
							     both. -->
							<div class="auto-row">
								<span class="pick">
									<input
										type="checkbox"
										checked={pickedPast.has(row.request)}
										disabled={refusal !== null}
										title={refusal ?? undefined}
										aria-label={`Select the transfer ${pastLabel(row)}`}
										onchange={(event) => pickPast(row, event.currentTarget.checked)}
									/>
								</span>
								<span class="who">
									<a class="t" href={row.href}>
										<MarketplaceMark inventory={row.source} /> →
										<MarketplaceMark inventory={row.target} />
									</a>
									<span class="meta">{row.meta}</span>
								</span>
								<span class="mark">
									<StatusPill tone={going?.tone ?? row.tone} label={going?.label ?? row.label} />
								</span>
								<span class="act">
									<Button
										small
										danger
										disabled={refusal !== null}
										reason={refusal ?? undefined}
										onclick={() => (deleting = [{ id: row.request, label: pastLabel(row) }])}
									>
										Delete
									</Button>
								</span>
							</div>
						{/each}
					{/if}
					{#if past.length > 0 || requestPage > 1}
						<Pagination
							page={requestPage}
							hasNext={requestNext !== null}
							busy={requestsBusy}
							label="Your transfers"
							summary={`${past.length} transfers on this page`}
							onprevious={() =>
								void readRequests(requestCursors[requestPage - 2] ?? null, requestPage - 1)}
							onnext={() => void readRequests(requestNext, requestPage + 1)}
						/>
					{/if}
				{/if}
			</Panel>

			<Panel
				title="Activity log"
				description="What each transfer on this page did, newest first."
			>
				<ActivityLog
					entries={log}
					bind:query={logQuery}
					empty={requestPage > 1
						? 'There is nothing to log on this page. Go back for the transfers before it.'
						: 'No transfer has run yet, so there is nothing to log.'}
				/>
			</Panel>
		</div>
	</div>
</div>

{#snippet toDownloads()}
	<Button tier="outline" small href="/marketplaces">Connect on Marketplaces</Button>
{/snippet}

<!-- Each retries the page that failed, which the read remembers apart from
     the page on screen. Retrying the displayed page instead would clear the
     failure without ever fetching what the seller pressed Next for. -->
{#snippet retryRequests()}
	<Button
		tier="outline"
		small
		disabled={requestsBusy}
		reason={requestsBusy ? 'A page is being read.' : undefined}
		onclick={retryRequestPage}
	>
		Retry
	</Button>
{/snippet}

{#snippet retryPicks()}
	<Button
		tier="outline"
		small
		disabled={pickBusy}
		reason={pickBusy ? 'A page is being read.' : undefined}
		onclick={retryPickPage}
	>
		Retry
	</Button>
{/snippet}

<!-- One dialog for a row's own Delete and for the selection's: what a seller
     has to read before deleting a transfer is the same either way. -->
<WorkDeleteDialog
	open={deleting !== null}
	items={deleting ?? []}
	noun={TRANSFERS}
	remove={api.deleteSyncRequest}
	onClose={() => (deleting = null)}
	onsettled={settled}
/>
