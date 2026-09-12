<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import {
		ApiFailure,
		allPages,
		api,
		type ConnectionView,
		type DeviceView,
		type ProductHead,
		type ScheduleRepeat,
		type ScheduleRunView,
		type ScheduleView
	} from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { anyConnectionStands, standingMarketplaces } from '$lib/connection-standing';
	import { entitlementRead, featureOf } from '$lib/entitlement-read';
	import Field from '$lib/Field.svelte';
	import type { InventoryId, Marketplace } from '$lib/generated/vocab';
	import { MARKETPLACE_OF, normaliseQuery } from '$lib/listings-view';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { AUTHORABLE_PLATFORMS, MARK_SRC, SHORT_NAME, platformTitle } from '$lib/platforms';
	import { queryKeys } from '$lib/query';
	import Toggle from '$lib/Toggle.svelte';
	import MarketplaceList from '$lib/pages/automations/MarketplaceList.svelte';
	import { heldSelection, marketplaceRows } from '$lib/pages/automations/marketplace-list';
	import { columnCopy, readState } from '$lib/pages/automations/read-state';
	import {
		INTENTS,
		NO_DEVICE_BODY,
		NO_DEVICE_TITLE,
		NO_SCHEDULE_YET,
		REPEATS,
		RUNS_ON_YOUR_COMPUTER,
		TES_CANNOT_REVISE,
		WEEKDAYS,
		WHAT_SCHEDULING_IS,
		blankDraft,
		deletePrompt,
		draftOf,
		draftRefusal,
		lastRunLine,
		nextRunLine,
		scheduleBody,
		selectionLine,
		timezoneLine,
		timezoneName,
		timezones,
		whenSentence,
		type ScheduleDraft
	} from '$lib/pages/automations/sharing';
	import '$lib/pages/automations/automations.css';

	let schedules = $state<ScheduleView[]>([]);
	let schedulesLoaded = $state(false);
	// A schedule list that could not be read is not a seller with no schedule.
	// Told the second when the first is true, they write the same schedule
	// again and it runs twice.
	let schedulesUnread = $state(false);

	let connections = $state<ConnectionView[]>([]);
	let connectionsLoaded = $state(false);
	let connectionsUnread = $state(false);

	let devices = $state<DeviceView[]>([]);
	// Whether the machine list was read at all. A seller whose devices could
	// not be listed is not a seller with no machine, and the banner telling
	// them to install the app is the wrong thing to show in that case.
	let devicesRead = $state(false);

	let held = $state<Marketplace | null>(null);
	let box = $state('');

	// The form, which is not the wire shape: a half-filled form has a time
	// that does not parse and a selection that names neither side.
	let draft = $state<ScheduleDraft>(blankDraft(timezoneName()));
	// The schedule being edited, or null while the form is a new one. The
	// identifier rather than the row, so a refetch that replaces the list does
	// not leave the form editing a stale object.
	let editing = $state<string | null>(null);
	let saving = $state(false);
	let refusal = $state<string | null>(null);

	// Two-step rather than a browser confirm: a schedule is named, and a
	// dialog that says "are you sure" without saying which one is the dialog
	// people click through.
	let deleting = $state<string | null>(null);

	let runsOf = $state<string | null>(null);
	let runs = $state<ScheduleRunView[]>([]);
	let runsUnread = $state(false);

	const entitlement = createQuery(() => entitlementRead);
	// The whole page's gate. Every control carries it as its stated reason,
	// and the banner above says it once with the way out.
	const gate = $derived(featureOf(entitlement.data, 'scheduling'));
	// The republish rule is its own capability: a seller on a plan that
	// schedules but does not hold auto-publish rules gets every other control.
	const republishGate = $derived(featureOf(entitlement.data, 'auto_publish_rules'));

	const labels = createQuery(() => ({
		queryKey: queryKeys.labels,
		queryFn: () => api.labels().then((view) => view.labels)
	}));
	const catalogue = createQuery(() => ({
		queryKey: queryKeys.catalogue(null),
		queryFn: () =>
			allPages(
				(cursor) => api.products(cursor, null),
				(view) => view.products
			)
	}));

	const products = $derived<ProductHead[]>(catalogue.data ?? []);
	const query = $derived(normaliseQuery(box).toLocaleLowerCase());
	const shownProducts = $derived(
		query.length === 0
			? products
			: products.filter((product) => product.title.toLocaleLowerCase().includes(query))
	);

	const nothingConnected = $derived(!connectionsUnread && !anyConnectionStands(connections));
	const noDevice = $derived(devicesRead && devices.length === 0);

	/** How many schedules send to each marketplace, which is the figure the
	 *  left column carries and the filter the seller narrows by. */
	const counts = $derived.by(() => {
		const tally: Partial<Record<Marketplace, number>> = {};
		for (const schedule of schedules) {
			for (const marketplace of new Set(
				schedule.inventories.map((inventory) => MARKETPLACE_OF[inventory])
			)) {
				tally[marketplace] = (tally[marketplace] ?? 0) + 1;
			}
		}
		return tally;
	});
	const rows = $derived(marketplaceRows(connections, counts));
	const read = $derived(readState(connectionsLoaded, connectionsUnread, rows));
	const selected = $derived(heldSelection(rows, held));
	// The list narrowed to the marketplace the column has selected. A seller
	// with one marketplace sees no difference; one with three can ask what
	// goes to TPT without reading the others.
	const shownSchedules = $derived(
		selected === null
			? schedules
			: schedules.filter((schedule) =>
					schedule.inventories.some((inventory) => MARKETPLACE_OF[inventory] === selected)
				)
	);

	const zones = timezones();
	// The marketplaces a schedule may actually name. A tile for one the seller
	// has no connection to is offered and disabled with its reason, rather
	// than hidden: a seller who cannot find TPT on this form needs to be told
	// where to connect it, not shown a shorter row.
	const standing = $derived(standingMarketplaces(connections));
	const now = $derived(Date.now());
	const formRefusal = $derived(gate ?? draftRefusal(draft));

	async function load() {
		try {
			const view = await api.schedules();
			schedules = view.schedules;
			schedulesUnread = false;
		} catch {
			schedules = [];
			schedulesUnread = true;
		}
		schedulesLoaded = true;
	}

	$effect(() => {
		void load();
		void api
			.connections()
			.then((standing) => {
				connections = standing;
				connectionsUnread = false;
			})
			.catch(() => {
				connections = [];
				connectionsUnread = true;
			})
			.finally(() => (connectionsLoaded = true));
		void api
			.devices()
			.then((view) => {
				devices = view.devices;
				devicesRead = true;
			})
			.catch(() => (devicesRead = false));
	});

	function tickProduct(product: string, on: boolean) {
		const next = new Set(draft.products);
		if (on) {
			next.add(product);
		} else {
			next.delete(product);
		}
		draft = { ...draft, products: [...next] };
	}

	function tickInventory(inventory: InventoryId, on: boolean) {
		const next = new Set(draft.inventories);
		if (on) {
			next.add(inventory);
		} else {
			next.delete(inventory);
		}
		draft = { ...draft, inventories: [...next] };
	}

	function edit(schedule: ScheduleView) {
		editing = schedule.id;
		draft = draftOf(schedule);
		refusal = null;
	}

	function cancel() {
		editing = null;
		draft = blankDraft(timezoneName());
		refusal = null;
	}

	async function save() {
		saving = true;
		refusal = null;
		const body = scheduleBody(draft);
		try {
			if (editing === null) {
				await api.createSchedule(body);
			} else {
				await api.updateSchedule(editing, body);
			}
			cancel();
			await load();
		} catch (caught) {
			refusal =
				caught instanceof ApiFailure
					? caught.message
					: 'The schedule could not be saved, so nothing was changed.';
		} finally {
			saving = false;
		}
	}

	/** The enabled switch writes the whole schedule back, because the route
	 *  takes the whole schedule: a body naming only `enabled` would be a
	 *  second write shape for one field. */
	async function setEnabled(schedule: ScheduleView, enabled: boolean) {
		refusal = null;
		try {
			await api.updateSchedule(schedule.id, {
				...scheduleBody(draftOf(schedule)),
				enabled
			});
			await load();
		} catch (caught) {
			refusal =
				caught instanceof ApiFailure
					? caught.message
					: 'The schedule could not be changed, so it is as it was.';
		}
	}

	async function remove(id: string) {
		refusal = null;
		try {
			await api.deleteSchedule(id);
			deleting = null;
			if (editing === id) {
				cancel();
			}
			if (runsOf === id) {
				runsOf = null;
				runs = [];
			}
			await load();
		} catch (caught) {
			refusal =
				caught instanceof ApiFailure
					? caught.message
					: 'The schedule could not be deleted, so it is still there.';
		}
	}

	async function openRuns(id: string) {
		if (runsOf === id) {
			runsOf = null;
			runs = [];
			return;
		}
		runsOf = id;
		try {
			const view = await api.scheduleRuns(id);
			runs = view.runs;
			runsUnread = false;
		} catch {
			runs = [];
			runsUnread = true;
		}
	}
</script>

<div class="page">
	<PageHead icon="calendar-clock" title="Scheduling" description={WHAT_SCHEDULING_IS} />

	{#if gate !== null}
		<!-- Stated once, with the way out, rather than only as a tooltip on
		     every dead control. The controls below carry it too, because a
		     disabled control with no reason reads as a fault. -->
		<Banner tone="warn" title="Scheduling is not on your plan" action={toPlans}>
			{gate}
		</Banner>
	{/if}

	{#if nothingConnected}
		<Banner tone="warn" title="No marketplace is connected" action={toMarketplaces}>
			A schedule sends to a marketplace, so there is nothing to send to until one is
			connected. Connect it in the Teachouse app on your computer: the app opens the
			marketplace sign-in there and keeps your login on that machine.
		</Banner>
	{/if}

	{#if noDevice}
		<Banner tone="warn" title={NO_DEVICE_TITLE} action={toMarketplaces}>
			{NO_DEVICE_BODY}
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
			<Panel title="Your schedules" description="What goes out, where, and when.">
				{#if schedulesUnread}
					<p class="quiet">
						Your schedules could not be read, so this page cannot list them. Any schedule
						already set is unaffected and still runs.
					</p>
				{:else if !schedulesLoaded}
					<p class="quiet">Loading…</p>
				{:else if schedules.length === 0}
					<Placeholder icon="calendar-clock" headline="Nothing is scheduled" body={NO_SCHEDULE_YET} />
				{:else if shownSchedules.length === 0}
					<p class="quiet">
						No schedule sends to {selected === null ? 'this marketplace' : SHORT_NAME[selected]}.
					</p>
				{:else}
					{#each shownSchedules as schedule (schedule.id)}
						<div class="sched-row" class:off={!schedule.enabled}>
							<span class="sched-name">
								<span class="t">{schedule.name}</span>
								<span class="meta">{selectionLine(schedule.selection)}</span>
							</span>

							<span class="sched-marks">
								{#each schedule.inventories as inventory (inventory)}
									<MarketplaceMark {inventory} size={18} />
								{/each}
							</span>

							<span class="sched-when">
								<span class="t">{whenSentence(schedule)}</span>
								<span class="meta">
									Next {nextRunLine(schedule.next_run_at, now)} · last
									{lastRunLine(schedule.last_run_at, now)}
								</span>
							</span>

							<span class="sched-acts">
								<Toggle
									label="On"
									checked={schedule.enabled}
									disabled={gate !== null}
									onchange={(value) => void setEnabled(schedule, value)}
								/>
								<Button tier="quiet" small onclick={() => void openRuns(schedule.id)}>
									{runsOf === schedule.id ? 'Hide runs' : 'Runs'}
								</Button>
								<Button
									tier="outline"
									small
									disabled={gate !== null}
									reason={gate ?? undefined}
									onclick={() => edit(schedule)}
								>
									Edit
								</Button>
								<Button
									tier="outline"
									small
									danger
									disabled={gate !== null}
									reason={gate ?? undefined}
									onclick={() => (deleting = schedule.id)}
								>
									Delete
								</Button>
							</span>
						</div>

						{#if deleting === schedule.id}
							<div class="sched-confirm">
								<span>{deletePrompt(schedule.name)}</span>
								<Button tier="outline" small danger onclick={() => void remove(schedule.id)}>
									Delete it
								</Button>
								<Button tier="quiet" small onclick={() => (deleting = null)}>Keep it</Button>
							</div>
						{/if}

						{#if runsOf === schedule.id}
							<div class="sched-runs">
								{#if runsUnread}
									<p class="quiet">This schedule's runs could not be read.</p>
								{:else if runs.length === 0}
									<p class="quiet">This schedule has not run yet.</p>
								{:else}
									{#each runs as run (`${run.tick}-${run.inventory}`)}
										<div class="run-tick">
											<span class="t">
												<MarketplaceMark inventory={run.inventory} size={16} />
												{new Date(run.tick).toLocaleString()}
											</span>
											<span class="meta">
												{run.sent}
												{run.sent === 1 ? 'resource sent' : 'resources sent'}
												{#if run.job !== null}
													· <a href={`/sync/${run.job}`}>Open the run</a>
												{/if}
											</span>
											{#each run.skipped as skip (skip.product)}
												<span class="skip">
													<span class="skip-title">{skip.title}</span>
													<span class="skip-why">{skip.reason}</span>
												</span>
											{/each}
										</div>
									{/each}
								{/if}
							</div>
						{/if}
					{/each}
				{/if}
			</Panel>

			<Panel
				title={editing === null ? 'New schedule' : 'Edit schedule'}
				description={RUNS_ON_YOUR_COMPUTER}
			>
				<div class="set-grid">
					<Field label="Name" id="sched-name">
						<input
							id="sched-name"
							type="text"
							maxlength="80"
							placeholder="Friday drop"
							disabled={gate !== null}
							bind:value={draft.name}
						/>
					</Field>
				</div>

				<div class="scope-choice" role="radiogroup" aria-label="What to send">
					<button
						type="button"
						class="disp"
						class:on={draft.label !== null}
						role="radio"
						aria-checked={draft.label !== null}
						disabled={gate !== null}
						onclick={() => (draft = { ...draft, label: draft.label ?? '' })}
					>
						<span class="disp-word">Everything with a label</span>
						<span class="disp-line">The label is read again at every run.</span>
					</button>
					<button
						type="button"
						class="disp"
						class:on={draft.label === null}
						role="radio"
						aria-checked={draft.label === null}
						disabled={gate !== null}
						onclick={() => (draft = { ...draft, label: null })}
					>
						<span class="disp-word">Choose resources</span>
						<span class="disp-line">The ones you tick, and no others.</span>
					</button>
				</div>

				{#if draft.label !== null}
					<div class="set-grid">
						<Field label="Label" id="sched-label">
							<select
								id="sched-label"
								disabled={gate !== null}
								value={draft.label}
								onchange={(event) => (draft = { ...draft, label: event.currentTarget.value })}
							>
								<option value="">Choose a label</option>
								{#each labels.data ?? [] as label (label.name)}
									<option value={label.name}>{label.name}</option>
								{/each}
							</select>
						</Field>
					</div>
					{#if labels.isError}
						<p class="quiet">
							Your labels could not be read, so there is none to choose. Ticking the resources
							yourself does not need this list.
						</p>
					{/if}
				{:else if catalogue.isPending}
					<p class="quiet">Loading your resources…</p>
				{:else if catalogue.isError}
					<p class="quiet">Your resources could not be read, so there is nothing to tick.</p>
				{:else if products.length === 0}
					<p class="quiet">You have no resources yet, so there is nothing to schedule.</p>
				{:else}
					<div class="pick-head">
						<input
							type="search"
							aria-label="Search your resources"
							placeholder="Search resources"
							bind:value={box}
						/>
						<span class="pick-count">{draft.products.length} chosen</span>
					</div>
					{#if shownProducts.length === 0}
						<p class="quiet">Nothing matches that search.</p>
					{:else}
						<div class="pick-list">
							{#each shownProducts as product (product.id)}
								<label class="pick-row">
									<input
										type="checkbox"
										checked={draft.products.includes(product.id)}
										disabled={gate !== null}
										onchange={(event) => tickProduct(product.id, event.currentTarget.checked)}
									/>
									<span class="pick-title">{product.title}</span>
								</label>
							{/each}
						</div>
					{/if}
				{/if}

				<p class="pull-label">Send to</p>
				<div class="mk-tiles" role="group" aria-label="Marketplaces to send to">
					<!-- The authorable set, not every inventory: a tick mints a create
					     job at the tick, and a marketplace this console cannot author
					     to would take the schedule and write nothing. -->
					{#each AUTHORABLE_PLATFORMS as inventory (inventory)}
						{@const why =
							gate ??
							(standing.has(MARKETPLACE_OF[inventory])
								? null
								: `Connect ${SHORT_NAME[MARKETPLACE_OF[inventory]]} in the Teachouse app and a schedule can send to it.`)}
						<!-- The reason is the tooltip and the screen-reader name rather
						     than a line under the tile: the grid is evenly spaced and a
						     sentence under one tile would make that column taller than
						     the rest. -->
						<label class="mk-tile" title={why ?? platformTitle(inventory)}>
							<input
								type="checkbox"
								checked={draft.inventories.includes(inventory)}
								disabled={why !== null}
								onchange={(event) => tickInventory(inventory, event.currentTarget.checked)}
							/>
							<img class="mk-tile-mark" src={MARK_SRC[MARKETPLACE_OF[inventory]]} alt="" />
							<span class="sr-only">
								{platformTitle(inventory)}{why === null ? '' : ` — ${why}`}
							</span>
						</label>
					{/each}
				</div>

				<div class="disp-choice" role="radiogroup" aria-label="Draft or live">
					{#each INTENTS as option (option.value)}
						<button
							type="button"
							class="disp"
							class:on={draft.intent === option.value}
							role="radio"
							aria-checked={draft.intent === option.value}
							disabled={gate !== null}
							onclick={() => (draft = { ...draft, intent: option.value })}
						>
							<span class="disp-word">{option.word}</span>
							<span class="disp-line">{option.line}</span>
						</button>
					{/each}
				</div>

				<div class="set-grid">
					<Field label="Time" id="sched-time" hint={timezoneLine(draft.timezone)}>
						<input
							id="sched-time"
							type="time"
							disabled={gate !== null}
							bind:value={draft.clock}
						/>
					</Field>

					<Field label="Timezone" id="sched-zone">
						<select id="sched-zone" disabled={gate !== null} bind:value={draft.timezone}>
							{#each zones as zone (zone)}
								<option value={zone}>{zone}</option>
							{/each}
						</select>
					</Field>

					<Field label="Repeat" id="sched-repeat">
						<select
							id="sched-repeat"
							disabled={gate !== null}
							value={draft.repeat}
							onchange={(event) =>
								(draft = {
									...draft,
									repeat: event.currentTarget.value as ScheduleRepeat
								})}
						>
							{#each REPEATS as option (option.value)}
								<option value={option.value}>{option.label}</option>
							{/each}
						</select>
					</Field>

					{#if draft.repeat === 'weekly'}
						<Field label="Day" id="sched-weekday">
							<select
								id="sched-weekday"
								disabled={gate !== null}
								value={String(draft.weekday)}
								onchange={(event) =>
									(draft = { ...draft, weekday: Number(event.currentTarget.value) })}
							>
								{#each WEEKDAYS as day, index (day)}
									<option value={String(index)}>{day}</option>
								{/each}
							</select>
						</Field>
					{/if}
				</div>

				<div class="set-toggle">
					<Toggle
						label="When a resource changes, republish it"
						checked={draft.republishOnUpdate}
						disabled={gate !== null || republishGate !== null}
						onchange={(value) => (draft = { ...draft, republishOnUpdate: value })}
					/>
				</div>
				{#if republishGate !== null}
					<p class="foot-note">{republishGate}</p>
				{/if}
				<p class="foot-note">{TES_CANNOT_REVISE}</p>

				<div class="set-toggle">
					<Toggle
						label="Switched on"
						checked={draft.enabled}
						disabled={gate !== null}
						onchange={(value) => (draft = { ...draft, enabled: value })}
					/>
				</div>

				<div class="set-foot">
					<Button
						tier="primary"
						disabled={formRefusal !== null || saving}
						reason={formRefusal ?? (saving ? 'The schedule is being saved.' : undefined)}
						onclick={() => void save()}
					>
						{saving ? 'Saving…' : editing === null ? 'Save schedule' : 'Save changes'}
					</Button>
					{#if editing !== null}
						<Button tier="quiet" onclick={cancel}>Cancel</Button>
					{/if}
					{#if formRefusal !== null}
						<p class="foot-note">{formRefusal}</p>
					{/if}
				</div>

				{#if refusal !== null}
					<Banner tone="bad">{refusal}</Banner>
				{/if}
			</Panel>
		</div>
	</div>
</div>

{#snippet toMarketplaces()}
	<Button tier="outline" small href="/marketplaces">Connect on Marketplaces</Button>
{/snippet}

{#snippet toPlans()}
	<Button tier="primary" small href="/settings/subscription">See plans</Button>
{/snippet}
