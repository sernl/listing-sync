<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { identityAdminRefusal, mergeUsers, signInTrailVisible, type AdminUserRow } from '$lib/admin';
	import { api } from '$lib/api';
	import { AuthFailure, impersonatedSession, listIdentityUsers } from '$lib/auth-client';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import Explain from '$lib/Explain.svelte';
	import Icon from '$lib/Icon.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { planName, planTone } from '$lib/pages/admin/admin-view';
	import UserSheet from '$lib/pages/admin/UserSheet.svelte';
	import {
		CHIP_WORDS,
		USER_FILTERS,
		displayName,
		inFilter,
		initials,
		rowMatches,
		userChips,
		type UserFilter
	} from '$lib/pages/admin/users-view';
	import '$lib/flow.css';
	import '$lib/pages/admin/admin.css';

	/** How many accounts one listing carries. A rendering bound, not a policy
	 *  one: the search narrows the list rather than paging it. */
	const PAGE_SIZE = 50;

	const queryClient = useQueryClient();
	const now = Date.now();

	let typed = $state('');
	let applied = $state('');
	let filter = $state<UserFilter>('all');
	let openId = $state<string | null>(null);
	/** Sign-in counts, by identity account, for the accounts an operator has
	 *  opened. Not preloaded: the identity service lists sessions one account
	 *  at a time. */
	let counted = $state<Record<string, number>>({});
	/** Declared once rather than inline: the sessions dialog reports its count
	 *  from an effect, and a new function each render would loop it. */
	function noteSessionCount(userId: string, count: number) {
		counted = { ...counted, [userId]: count };
	}

	const users = createQuery(() => ({
		queryKey: queryKeys.identityUsers(applied),
		queryFn: () => listIdentityUsers(applied, PAGE_SIZE),
		retry: false
	}));

	/** The platform's half: every app user, with their organisation, its plan,
	 *  the operator marking and the last sign-in. Joined to the identity list in
	 *  the browser, because no one database role can read both planes. */
	const platform = createQuery(() => ({
		queryKey: queryKeys.adminUsers,
		queryFn: () => api.adminUsers()
	}));

	/** The operator's own identity, which the sheet will not let delete or ban
	 *  itself. The same query the impersonation banner reads. */
	const self = createQuery(() => ({
		queryKey: queryKeys.identitySession,
		queryFn: () => impersonatedSession()
	}));

	/**
	 * `rows` is read in the first arm of the markup below rather than only in
	 * the arm that draws the list, and that is load-bearing: a query result
	 * notifies only about fields its reader has touched. The platform read
	 * resolves while the identity list is still pending, so a `platform.data`
	 * first touched inside a later arm lands with nobody listening.
	 */
	const appUsers = $derived(platform.data?.users ?? []);
	const merged = $derived(mergeUsers(users.data ?? [], appUsers));
	const rows = $derived(merged.rows);
	const matching = $derived(rows.filter((row) => rowMatches(row, typed)));
	const shown = $derived(matching.filter((row) => inFilter(row, filter)));
	const trailVisible = $derived(signInTrailVisible(appUsers));
	const listRefusal = $derived(
		users.error instanceof AuthFailure ? identityAdminRefusal(users.error.status) : null
	);
	const opened = $derived<AdminUserRow | null>(
		openId === null ? null : (rows.find((row) => row.identity.id === openId) ?? null)
	);

	async function reload() {
		await Promise.all([
			queryClient.invalidateQueries({ queryKey: queryKeys.identityUsers(applied) }),
			queryClient.invalidateQueries({ queryKey: queryKeys.adminUsers })
		]);
	}

	function search(event: SubmitEvent) {
		event.preventDefault();
		applied = typed.trim();
	}

	function clearSearch() {
		typed = '';
		applied = '';
	}

	function joined(at: string | Date | null | undefined): number | null {
		if (at === null || at === undefined) return null;
		const parsed = new Date(at).getTime();
		return Number.isNaN(parsed) ? null : parsed;
	}
</script>

<div class="page flow-page">
	<PageHead icon="users" title="Users" description="Everyone with a sign-in account. Open one to act on it." />

	{#if listRefusal === 'not-identity-admin'}
		<Placeholder
			icon="users"
			headline="You are not an identity admin"
			body="Operator access and identity admin are granted separately, by hand. This list also needs
				identity admin. Ask the person who runs the identity service."
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
					<button type="button" class="ux-clear" onclick={clearSearch}>Clear</button>
				{/if}
				<Explain title="How search works" label="">
					<p>Typing narrows the accounts already on the page, by name, email or organisation.</p>
					<p>
						Press Enter to ask the identity service for addresses containing what you typed. It
						returns at most {PAGE_SIZE} accounts, newest first.
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

			{#if users.isPending && rows.length === 0}
				<p class="quiet">Loading accounts…</p>
			{:else if users.isError}
				<Placeholder
					icon="users"
					headline="We could not load the accounts"
					body="We cannot tell whether any accounts exist. Try reloading the page."
				/>
			{:else if rows.length === 0}
				<Placeholder
					icon="users"
					headline={applied.length === 0 ? 'No accounts yet' : 'No account matches that search'}
					body={applied.length === 0
						? 'The first account appears here when someone signs up.'
						: `No account's address contains “${applied}”.`}
				/>
			{:else}
				<ul class="ux-list" aria-label="Accounts">
					{#each shown as row (row.identity.id)}
						{@const user = row.identity}
						{@const chips = userChips(row)}
						{@const joinedAt = joined(user.createdAt)}
						<li>
							<button
								type="button"
								class="ux-row"
								aria-haspopup="dialog"
								onclick={() => (openId = user.id)}
							>
								<span class="ux-avatar" aria-hidden="true">{initials(user.name, user.email)}</span>
								<span class="ux-who">
									<span class="ux-name">{displayName(user)}</span>
									<span class="ux-email">{user.email}</span>
								</span>
								<span class="ux-org">
									{#if row.platform !== null}
										<span class="ux-org-name">{row.platform.organisation.name}</span>
										<StatusPill tone={planTone(row.platform.plan)} label={planName(row.platform.plan)} />
									{/if}
								</span>
								<span class="ux-chips">
									{#each chips as chip (chip)}
										<StatusPill tone={CHIP_WORDS[chip].tone} label={CHIP_WORDS[chip].label} />
									{/each}
								</span>
								<span class="ux-when" title={joinedAt === null ? undefined : utcInstant(joinedAt)}>
									{joinedAt === null ? '' : agoLabel(joinedAt, now)}
								</span>
								<span class="ux-go" aria-hidden="true"><Icon name="chevron-right" size={16} /></span>
							</button>
						</li>
					{:else}
						<li class="quiet ux-none">Nobody matches.</li>
					{/each}
				</ul>
				<p class="op-foot">
					{shown.length} of {rows.length}
					{rows.length === 1 ? 'account' : 'accounts'}, newest first, at most {PAGE_SIZE}.
					{#if merged.unlinked > 0}
						{merged.unlinked} app {merged.unlinked === 1 ? 'user is' : 'users are'} not on this
						page: the search does not match, or they were created outside sign-up.
					{/if}
				</p>
			{/if}
		</div>
	{/if}
</div>

{#if opened !== null}
	<UserSheet
		row={opened}
		{now}
		{trailVisible}
		selfId={self.data?.user.id ?? null}
		sessionCount={counted[opened.identity.id] ?? null}
		onCount={noteSessionCount}
		onChanged={reload}
		onClose={() => (openId = null)}
	/>
{/if}

<style>
	.ux {
		display: grid;
		gap: var(--s-3);
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

	.ux-clear {
		border: 0;
		background: none;
		color: var(--primary);
		font: inherit;
		font-size: 13px;
		cursor: pointer;
		padding: 0 var(--s-1);
	}

	.ux-list {
		list-style: none;
		margin: 0;
		padding: 0;
		border: 1px solid var(--line);
		border-radius: var(--r-card);
		background: var(--card);
		overflow: hidden;
	}

	.ux-list > li + li {
		border-top: 1px solid var(--line);
	}

	.ux-row {
		display: grid;
		grid-template-columns: auto minmax(0, 1.4fr) minmax(0, 1fr) auto auto auto;
		grid-template-areas: 'avatar who org chips when go';
		align-items: center;
		gap: var(--s-3);
		width: 100%;
		padding: var(--s-3) var(--s-4);
		border: 0;
		background: none;
		color: inherit;
		font: inherit;
		text-align: left;
		cursor: pointer;
	}

	.ux-row:hover {
		background: var(--hover);
	}

	.ux-row:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: -2px;
	}

	.ux-avatar {
		grid-area: avatar;
		display: inline-grid;
		place-items: center;
		width: 36px;
		height: 36px;
		border-radius: var(--r-pill);
		background: var(--accent-soft);
		color: var(--primary);
		font-weight: 600;
		font-size: 13px;
	}

	.ux-who {
		grid-area: who;
		display: grid;
		min-width: 0;
	}

	.ux-name {
		font-weight: 600;
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

	.ux-org {
		grid-area: org;
		display: flex;
		align-items: center;
		gap: var(--s-2);
		min-width: 0;
	}

	.ux-org-name {
		font-size: 13px;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.ux-chips {
		grid-area: chips;
		display: flex;
		flex-wrap: wrap;
		gap: var(--s-1);
		justify-content: flex-end;
	}

	.ux-when {
		grid-area: when;
		color: var(--muted);
		font-size: 12px;
		white-space: nowrap;
	}

	.ux-go {
		grid-area: go;
		color: var(--faint);
		display: inline-flex;
	}

	.ux-none {
		padding: var(--s-4);
	}

	/* A phone: avatar beside a stack of name, organisation and chips. */
	@media (max-width: 760px) {
		.ux-row {
			grid-template-columns: auto minmax(0, 1fr) auto;
			grid-template-areas:
				'avatar who go'
				'avatar org go'
				'avatar chips go';
			row-gap: var(--s-1);
			align-items: start;
		}

		.ux-go {
			align-self: center;
		}

		.ux-chips {
			justify-content: flex-start;
		}

		.ux-chips:empty,
		.ux-org:empty {
			display: none;
		}

		.ux-when {
			display: none;
		}
	}
</style>
