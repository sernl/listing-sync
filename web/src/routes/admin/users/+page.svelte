<script lang="ts">
	import { createQuery, keepPreviousData, useQueryClient } from '@tanstack/svelte-query';
	import {
		identityAdminRefusal,
		mergeUsers,
		signInTrailVisible,
		type AdminUserRow
	} from '$lib/admin';
	import { walkAdminUsers } from '$lib/api';
	import {
		AuthFailure,
		impersonatedSession,
		listAllIdentityUsers,
		listIdentityUsers,
		type IdentityUser
	} from '$lib/auth-client';
	import Button from '$lib/Button.svelte';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import Explain from '$lib/Explain.svelte';
	import Icon from '$lib/Icon.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Pagination from '$lib/Pagination.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { planName, planTone } from '$lib/pages/admin/admin-view';
	import BulkUsersDialog from '$lib/pages/admin/BulkUsersDialog.svelte';
	import UserSheet from '$lib/pages/admin/UserSheet.svelte';
	import { BULK_ORDER, BULK_WORDS, type BulkAction } from '$lib/pages/admin/users-bulk';
	import {
		NOTHING,
		afterSearch,
		isSelected,
		pageTick,
		selectMatching,
		selectedCount,
		targets,
		toggle,
		togglePage,
		type Selection
	} from '$lib/pages/admin/user-selection';
	import {
		CHIP_WORDS,
		USER_FILTERS,
		bySignIn,
		displayName,
		inFilter,
		initials,
		joinedAt,
		rowMatches,
		userChips,
		type SortKey,
		type UserFilter,
		type UserSort
	} from '$lib/pages/admin/users-view';
	import '$lib/flow.css';
	import '$lib/styles/data.css';
	import '$lib/pages/admin/admin.css';

	/** Accounts per page. */
	const PAGE_SIZE = 25;
	/** How far an app user's row may predate its identity account and still
	 *  be found: the clocks of the two services are not the same clock. */
	const CLOCK_SLACK_MS = 60_000;

	const queryClient = useQueryClient();
	const now = Date.now();

	let typed = $state('');
	let applied = $state('');
	let filter = $state<UserFilter>('all');
	let page = $state(0);
	let sort = $state<UserSort>({ key: 'joined', direction: 'desc' });
	let selection = $state<Selection>(NOTHING);
	let bulk = $state<BulkAction | null>(null);
	let openId = $state<string | null>(null);
	/** Every identity account a loaded page has shown, by id: what a ticked
	 *  row on another page resolves to when the bulk action runs. Not state:
	 *  nothing draws it, it is only read when an action starts. */
	const known = new Map<string, IdentityUser>();
	/** Sign-in counts, by identity account, for the accounts an operator has
	 *  opened. Not preloaded: the identity service lists sessions one account
	 *  at a time. */
	let counted = $state<Record<string, number>>({});
	/** Declared once rather than inline: the sessions dialog reports its count
	 *  from an effect, and a new function each render would loop it. */
	function noteSessionCount(userId: string, count: number) {
		counted = { ...counted, [userId]: count };
	}

	/** The identity service pages by when accounts were made, so that is the
	 *  order it is asked for; a last-sign-in sort reorders the page it gives. */
	const direction = $derived(sort.key === 'joined' ? sort.direction : 'desc');

	const users = createQuery(() => ({
		queryKey: queryKeys.identityUsers(applied, page, direction),
		queryFn: () =>
			listIdentityUsers({
				search: applied,
				limit: PAGE_SIZE,
				offset: page * PAGE_SIZE,
				direction
			}),
		placeholderData: keepPreviousData,
		retry: false
	}));

	/** The oldest account on this page. The platform's half is read newest
	 *  first until it reaches back past this, which is every app user this
	 *  page can join to: an app user is made after its identity account. */
	const oldest = $derived.by(() => {
		const times = (users.data?.users ?? []).map((user) => joinedAt(user) ?? 0);
		return times.length === 0 ? null : Math.min(...times) - CLOCK_SLACK_MS;
	});

	/** The platform's half: app users with their organisation, its plan, the
	 *  operator marking and the last sign-in. Joined to the identity page in
	 *  the browser, because no one database role can read both planes. */
	const platform = createQuery(() => ({
		queryKey: queryKeys.adminUsersCovering(oldest ?? 0),
		queryFn: () =>
			walkAdminUsers((rows) => {
				const last = rows.at(-1);
				return last !== undefined && last.created_at < (oldest ?? 0);
			}),
		enabled: oldest !== null,
		placeholderData: keepPreviousData
	}));

	/** The operator's own identity, which cannot delete, ban or demote
	 *  itself. The same query the impersonation banner reads. */
	const self = createQuery(() => ({
		queryKey: queryKeys.identitySession,
		queryFn: () => impersonatedSession()
	}));
	const selfId = $derived(self.data?.user.id ?? null);

	/**
	 * `rows` is read in the first arm of the markup below rather than only in
	 * the arm that draws the list, and that is load-bearing: a query result
	 * notifies only about fields its reader has touched. The platform read
	 * resolves while the identity list is still pending, so a `platform.data`
	 * first touched inside a later arm lands with nobody listening.
	 */
	const appUsers = $derived(platform.data?.users ?? []);
	const rows = $derived(mergeUsers(users.data?.users ?? [], appUsers));
	const total = $derived(users.data?.total ?? 0);
	const matching = $derived(rows.filter((row) => rowMatches(row, typed)));
	const filtered = $derived(matching.filter((row) => inFilter(row, filter)));
	const shown = $derived(sort.key === 'signin' ? bySignIn(filtered, sort.direction) : filtered);
	const shownIds = $derived(shown.map((row) => row.identity.id));
	const tick = $derived(pageTick(selection, shownIds));
	const chosen = $derived(selectedCount(selection));
	const trailVisible = $derived(signInTrailVisible(appUsers));
	const listRefusal = $derived(
		users.error instanceof AuthFailure ? identityAdminRefusal(users.error.status) : null
	);
	const opened = $derived<AdminUserRow | null>(
		openId === null ? null : (rows.find((row) => row.identity.id === openId) ?? null)
	);
	const first = $derived(total === 0 ? 0 : page * PAGE_SIZE + 1);
	const last = $derived(Math.min(total, page * PAGE_SIZE + (users.data?.users.length ?? 0)));

	$effect(() => {
		const fresh = users.data?.users;
		if (fresh === undefined) return;
		for (const user of fresh) known.set(user.id, user);
	});

	async function reload() {
		await Promise.all([
			queryClient.invalidateQueries({ queryKey: ['identity-users'] }),
			queryClient.invalidateQueries({ queryKey: queryKeys.adminUsers })
		]);
	}

	function search(event: SubmitEvent) {
		event.preventDefault();
		applyingSearch(typed.trim());
	}

	function applyingSearch(next: string) {
		applied = next;
		page = 0;
		selection = afterSearch(selection, next);
	}

	function sortBy(key: SortKey) {
		const direction =
			sort.key === key ? (sort.direction === 'desc' ? 'asc' : 'desc') : ('desc' as const);
		sort = { key, direction };
		// Joined reorders the whole listing, so the operator starts at its top.
		if (key === 'joined') page = 0;
	}

	function ariaSort(key: SortKey): 'ascending' | 'descending' | 'none' {
		if (sort.key !== key) return 'none';
		return sort.direction === 'asc' ? 'ascending' : 'descending';
	}

	/** The accounts the bulk action runs on, read when the operator confirms. */
	async function resolveTargets(): Promise<IdentityUser[]> {
		if (selection.kind === 'ids') return targets(selection, [...known.values()]);
		return targets(selection, await listAllIdentityUsers(selection.search));
	}

	async function bulkFinished() {
		selection = NOTHING;
		await reload();
	}

	function openRow(event: MouseEvent, id: string) {
		// A press on the tick box or a link inside the row is its own action.
		if ((event.target as HTMLElement).closest('input, a, button, label') !== null) return;
		openId = id;
	}
</script>

<div class="page flow-page">
	<PageHead
		icon="users"
		title="Users"
		description="Everyone with a sign-in account. Open one to act on it."
	/>

	{#if listRefusal === 'not-identity-admin'}
		<Placeholder
			icon="users"
			headline="You are not an identity admin"
			body="Operator access and identity admin are granted separately. This list also needs identity
				admin. Ask another identity admin to turn it on for you."
		/>
	{:else if listRefusal === 'unreadable'}
		<Placeholder
			icon="users"
			headline="We could not load the accounts"
			body="The identity service did not answer. Try reloading the page."
		/>
	{:else}
		<div class="ux">
			<form class="ux-search" role="search" onsubmit={search}>
				<span class="ux-search-icon" aria-hidden="true"><Icon name="search" size={16} /></span>
				<label class="sr-only" for="user-search">Search by name, email or organisation</label>
				<input
					id="user-search"
					name="q"
					type="search"
					placeholder="Search by name, email or organisation"
					autocomplete="off"
					bind:value={typed}
				/>
				{#if applied.length > 0 || typed.length > 0}
					<button
						type="button"
						class="ux-clear"
						onclick={() => {
							typed = '';
							applyingSearch('');
						}}>Clear</button
					>
				{/if}
				<Explain title="How search works" label="">
					<p>Typing narrows the accounts on this page, by name, email or organisation.</p>
					<p>
						Press Enter to search every account whose address contains what you typed. The chips
						below count this page only.
					</p>
				</Explain>
			</form>

			<div class="op-filters" role="group" aria-label="Show">
				{#each USER_FILTERS as chip (chip.id)}
					<button
						type="button"
						class="op-chip"
						aria-pressed={filter === chip.id}
						onclick={() => (filter = chip.id)}
					>
						{chip.label}
						<span class="c">{matching.filter((row) => inFilter(row, chip.id)).length}</span>
					</button>
				{/each}
			</div>

			{#if platform.isError}
				<p class="flow-warn">Organisations, plans and sign-ins could not be loaded.</p>
			{/if}

			{#if chosen > 0}
				<div class="ux-bulk" role="region" aria-label="Selected accounts">
					<div class="ux-bulk-count">
						<strong>{chosen} selected</strong>
						{#if selection.kind === 'matching'}
							<span class="quiet">on every page{applied ? `, matching “${applied}”` : ''}</span>
						{:else if tick === 'all' && total > shownIds.length}
							<button
								type="button"
								class="ux-link"
								onclick={() => (selection = selectMatching(applied, total, Date.now()))}
							>
								Select all {total}{applied ? ' matching' : ''}
							</button>
						{/if}
						<button type="button" class="ux-link" onclick={() => (selection = NOTHING)}>
							Clear
						</button>
					</div>
					<div class="ux-bulk-acts">
						{#each BULK_ORDER as action (action)}
							<Button
								small
								tier="outline"
								danger={BULK_WORDS[action].danger}
								icon={BULK_WORDS[action].icon}
								onclick={() => (bulk = action)}
							>
								{BULK_WORDS[action].label}
							</Button>
						{/each}
					</div>
				</div>
			{/if}

			{#if users.isPending && rows.length === 0}
				<p class="quiet">Loading accounts…</p>
			{:else if users.isError}
				<Placeholder
					icon="users"
					headline="We could not load the accounts"
					body="We cannot tell whether any accounts exist. Try reloading the page."
				/>
			{:else if total === 0}
				<Placeholder
					icon="users"
					headline={applied.length === 0 ? 'No accounts yet' : 'No account matches that search'}
					body={applied.length === 0
						? 'The first account appears here when someone signs up.'
						: `No account's address contains “${applied}”.`}
				/>
			{:else}
				<div class="ux-tools">
					<label class="ux-check">
						<input
							type="checkbox"
							checked={tick === 'all'}
							indeterminate={tick === 'some'}
							disabled={shownIds.length === 0}
							onchange={() => (selection = togglePage(selection, shownIds))}
						/>
						Select page
					</label>
					<label class="ux-sort">
						<span>Sort</span>
						<select
							value={`${sort.key}-${sort.direction}`}
							onchange={(event) => {
								const [key, dir] = event.currentTarget.value.split('-') as [
									SortKey,
									'asc' | 'desc'
								];
								sort = { key, direction: dir };
								if (key === 'joined') page = 0;
							}}
						>
							<option value="joined-desc">Newest first</option>
							<option value="joined-asc">Oldest first</option>
							<option value="signin-desc">Signed in recently</option>
							<option value="signin-asc">Signed in longest ago</option>
						</select>
					</label>
				</div>

				<div class="data-table-wrap">
					<table class="data-table ux-table" aria-busy={users.isFetching}>
						<thead>
							<tr>
								<th class="c-check" scope="col">
									<input
										type="checkbox"
										aria-label="Select every account on this page"
										checked={tick === 'all'}
										indeterminate={tick === 'some'}
										disabled={shownIds.length === 0}
										onchange={() => (selection = togglePage(selection, shownIds))}
									/>
								</th>
								<th scope="col">Name and email</th>
								<th scope="col">Organisation</th>
								<th scope="col">Plan</th>
								<th scope="col">Status</th>
								<th scope="col" aria-sort={ariaSort('joined')}>
									<button type="button" class="ux-sorter" onclick={() => sortBy('joined')}>
										Joined
										<span
											class="ux-arrow"
											class:up={sort.key === 'joined' && sort.direction === 'asc'}
										>
											<Icon name="chevron-down" size={14} />
										</span>
									</button>
								</th>
								<th scope="col" aria-sort={ariaSort('signin')}>
									<button
										type="button"
										class="ux-sorter"
										title="Sorts the accounts on this page"
										onclick={() => sortBy('signin')}
									>
										Last sign-in
										<span
											class="ux-arrow"
											class:up={sort.key === 'signin' && sort.direction === 'asc'}
										>
											<Icon name="chevron-down" size={14} />
										</span>
									</button>
								</th>
								<th class="c-go" scope="col"><span class="sr-only">Open</span></th>
							</tr>
						</thead>
						<tbody>
							{#each shown as row (row.identity.id)}
								{@const user = row.identity}
								{@const chips = userChips(row)}
								{@const joined = joinedAt(user)}
								{@const signedIn = row.platform?.last_sign_in_at ?? null}
								{@const ticked = isSelected(selection, user.id)}
								<!-- The name is the row's keyboard-reachable control; a press anywhere
								     else on the row is a larger target for the same thing. -->
								<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
								<tr class:ticked onclick={(event) => openRow(event, user.id)}>
									<td class="c-check">
										<input
											type="checkbox"
											aria-label={`Select ${user.email}`}
											checked={ticked}
											onchange={() => (selection = toggle(selection, user.id))}
										/>
									</td>
									<td class="c-who">
										<div class="ux-who">
											<span class="ux-avatar" aria-hidden="true"
												>{initials(user.name, user.email)}</span
											>
											<button
												type="button"
												class="ux-open"
												aria-haspopup="dialog"
												onclick={() => (openId = user.id)}
											>
												<span class="ux-name">{displayName(user)}</span>
												<span class="ux-email">{user.email}</span>
											</button>
										</div>
									</td>
									<td class="c-org">
										{#if row.platform !== null}
											<span class="ux-org-name">{row.platform.organisation.name}</span>
										{:else}
											<span class="quiet">—</span>
										{/if}
									</td>
									<td class="c-plan">
										{#if row.platform !== null}
											<StatusPill
												tone={planTone(row.platform.plan)}
												label={planName(row.platform.plan)}
											/>
										{/if}
									</td>
									<td class="c-status">
										<span class="ux-chips">
											{#each chips as chip (chip)}
												<StatusPill tone={CHIP_WORDS[chip].tone} label={CHIP_WORDS[chip].label} />
											{/each}
										</span>
									</td>
									<td class="c-when" title={joined === null ? undefined : utcInstant(joined)}>
										<span class="ux-when-label">Joined</span>
										{joined === null ? '—' : agoLabel(joined, now)}
									</td>
									<td class="c-when" title={signedIn === null ? undefined : utcInstant(signedIn)}>
										<span class="ux-when-label">Signed in</span>
										{signedIn === null ? '—' : agoLabel(signedIn, now)}
									</td>
									<td class="c-go" aria-hidden="true"><Icon name="chevron-right" size={16} /></td>
								</tr>
							{:else}
								<tr><td colspan="8" class="quiet">Nobody on this page matches.</td></tr>
							{/each}
						</tbody>
					</table>
				</div>

				<Pagination
					label="Account pages"
					page={page + 1}
					hasNext={last < total}
					busy={users.isFetching}
					summary={`${first}–${last} of ${total} ${total === 1 ? 'account' : 'accounts'}${
						sort.key === 'signin' ? ', this page sorted by last sign-in' : ''
					}`}
					onprevious={() => (page = Math.max(0, page - 1))}
					onnext={() => (page += 1)}
				/>
			{/if}
		</div>
	{/if}
</div>

{#if opened !== null}
	<UserSheet
		row={opened}
		{now}
		{trailVisible}
		{selfId}
		sessionCount={counted[opened.identity.id] ?? null}
		onCount={noteSessionCount}
		onChanged={reload}
		onClose={() => (openId = null)}
	/>
{/if}

{#if bulk !== null}
	<BulkUsersDialog
		action={bulk}
		count={chosen}
		{selfId}
		resolve={resolveTargets}
		onClose={() => (bulk = null)}
		onFinished={bulkFinished}
	/>
{/if}

<style>
	.ux {
		display: grid;
		gap: var(--s-3);
		min-width: 0;
	}

	.ux-search {
		position: relative;
		display: flex;
		align-items: center;
		gap: var(--s-2);
	}

	.ux-search-icon {
		position: absolute;
		left: 14px;
		display: inline-flex;
		color: var(--muted);
		pointer-events: none;
	}

	.ux-search input {
		flex: 1;
		min-width: 0;
		min-height: var(--control-h);
		border: 1px solid var(--line);
		background: var(--card);
		border-radius: var(--r-pill);
		padding: 0 14px 0 38px;
		font: inherit;
		color: var(--text);
	}

	.ux-search input:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 1px;
	}

	.ux-clear,
	.ux-link {
		border: 0;
		background: none;
		color: var(--primary);
		font: inherit;
		font-size: 13px;
		cursor: pointer;
		padding: 0 var(--s-1);
	}

	.ux-link {
		text-decoration: underline;
		text-underline-offset: 2px;
	}

	/* The bulk bar: what is selected, and what can be done to it. Sticks to
	   the bottom of the screen while the operator scrolls the table. */
	.ux-bulk {
		position: sticky;
		bottom: var(--s-3);
		z-index: 2;
		display: grid;
		gap: var(--s-2);
		padding: var(--s-3) var(--s-4);
		border: 1px solid var(--line);
		border-radius: var(--r-panel);
		background: var(--card);
		box-shadow: var(--sh-2);
	}

	.ux-bulk-count {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: var(--s-1) var(--s-3);
		font-size: 13.5px;
	}

	.ux-bulk-acts {
		display: flex;
		flex-wrap: wrap;
		gap: var(--s-2);
	}

	.quiet {
		color: var(--muted);
		font-size: 13px;
	}

	.ux-tools {
		display: none;
	}

	.ux-table th,
	.ux-table td {
		padding-inline: var(--s-3);
	}

	.ux-table th.c-check,
	.ux-table td.c-check {
		width: 1%;
		padding-right: 0;
	}

	.ux-table th.c-go,
	.ux-table td.c-go {
		width: 1%;
		padding-left: 0;
		color: var(--faint);
	}

	.ux-table input[type='checkbox'] {
		width: 16px;
		height: 16px;
		accent-color: var(--primary);
		cursor: pointer;
	}

	.ux-sorter {
		display: inline-flex;
		align-items: center;
		gap: 2px;
		border: 0;
		padding: 0;
		background: none;
		color: inherit;
		font: inherit;
		text-transform: inherit;
		letter-spacing: inherit;
		cursor: pointer;
	}

	.ux-arrow {
		display: inline-flex;
		opacity: 0.4;
	}

	th[aria-sort='ascending'] .ux-arrow,
	th[aria-sort='descending'] .ux-arrow {
		opacity: 1;
	}

	.ux-arrow.up {
		transform: rotate(180deg);
	}

	.ux-sorter:hover,
	th[aria-sort='ascending'] .ux-sorter,
	th[aria-sort='descending'] .ux-sorter {
		color: var(--ink);
	}

	/* Zebra rows, and a ticked row that reads as ticked. */
	.ux-table tbody tr:nth-child(even) td {
		background: color-mix(in srgb, var(--hover) 45%, transparent);
	}

	.ux-table tbody tr {
		cursor: pointer;
	}

	.ux-table tbody tr.ticked td {
		background: var(--accent-soft);
	}

	.ux-who {
		display: flex;
		align-items: center;
		gap: var(--s-3);
		min-width: 11rem;
		max-width: 20rem;
	}

	.ux-avatar {
		flex: none;
		display: inline-grid;
		place-items: center;
		width: 32px;
		height: 32px;
		border-radius: var(--r-pill);
		background: var(--accent-soft);
		color: var(--primary);
		font-weight: 600;
		font-size: 12px;
	}

	.ux-open {
		display: grid;
		min-width: 0;
		border: 0;
		padding: 0;
		background: none;
		color: inherit;
		font: inherit;
		text-align: left;
		cursor: pointer;
	}

	.ux-open:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 2px;
		border-radius: var(--r-field);
	}

	.ux-name {
		font-weight: 600;
		color: var(--ink);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.ux-email {
		color: var(--muted);
		font-size: 12.5px;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.ux-org-name {
		display: block;
		max-width: 16ch;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.ux-chips {
		display: flex;
		flex-wrap: wrap;
		gap: var(--s-1);
	}

	.c-when {
		color: var(--muted);
		white-space: nowrap;
	}

	.ux-when-label {
		display: none;
	}

	/* A phone: the table becomes a list of cards, each a tick box beside the
	   name, the organisation and plan, the chips and the two dates. */
	@media (max-width: 760px) {
		.ux-tools {
			display: flex;
			align-items: center;
			justify-content: space-between;
			gap: var(--s-3);
			font-size: 13px;
		}

		.ux-check,
		.ux-sort {
			display: inline-flex;
			align-items: center;
			gap: var(--s-2);
		}

		.ux-check input {
			width: 16px;
			height: 16px;
			accent-color: var(--primary);
		}

		.ux-sort select {
			min-height: var(--control-h-sm);
			border: 1px solid var(--line);
			border-radius: var(--r-field);
			background: var(--card);
			color: var(--text);
			padding-inline: 8px;
			font: inherit;
		}

		.ux-table thead {
			position: absolute;
			width: 1px;
			height: 1px;
			overflow: hidden;
			clip-path: inset(50%);
		}

		.ux-table,
		.ux-table tbody {
			display: block;
		}

		.ux-table tbody tr {
			display: grid;
			grid-template-columns: auto minmax(0, max-content) minmax(0, 1fr) auto;
			grid-template-areas:
				'check who who go'
				'. org plan go'
				'. status status go'
				'. joined signin go';
			gap: var(--s-1) var(--s-2);
			padding: var(--s-3) var(--s-4);
			border-top: 1px solid var(--line);
		}

		.ux-table tbody tr:first-child {
			border-top: 0;
		}

		.ux-table tbody tr:nth-child(even) {
			background: color-mix(in srgb, var(--hover) 45%, transparent);
		}

		.ux-table tbody tr.ticked {
			background: var(--accent-soft);
		}

		.ux-table tbody td,
		.ux-table tbody tr:nth-child(even) td,
		.ux-table tbody tr.ticked td {
			padding: 0;
			border: 0;
			background: none;
		}

		.ux-table td.c-check {
			grid-area: check;
			align-self: center;
		}

		.ux-table td.c-who {
			grid-area: who;
		}

		.ux-table td.c-org {
			grid-area: org;
		}

		.ux-table td.c-plan {
			grid-area: plan;
		}

		.ux-table td.c-status {
			grid-area: status;
		}

		.ux-table td.c-status:has(.ux-chips:empty) {
			display: none;
		}

		.ux-table td.c-when {
			font-size: 12px;
		}

		.ux-table td.c-when:nth-of-type(6) {
			grid-area: joined;
		}

		.ux-table td.c-when:nth-of-type(7) {
			grid-area: signin;
		}

		.ux-when-label {
			display: inline;
		}

		.ux-table td.c-go {
			grid-area: go;
			align-self: center;
		}

		.ux-avatar {
			display: none;
		}

		.ux-who {
			min-width: 0;
		}
	}
</style>
