<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto, invalidateAll } from '$app/navigation';
	import { identityAdminRefusal, mergeUsers, sessionWords, signInTrailVisible } from '$lib/admin';
	import { api } from '$lib/api';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import {
		AuthFailure,
		IDENTITY_ROLES,
		banIdentityUser,
		impersonateAndCarry,
		listIdentityUsers,
		setIdentityRole,
		unbanIdentityUser,
		type IdentityRole,
		type IdentityUser
	} from '$lib/auth-client';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import GrantPlanForm from '$lib/pages/admin/GrantPlanForm.svelte';
	import { planName } from '$lib/pages/admin/admin-view';
	import SessionsDialog from '$lib/pages/admin/SessionsDialog.svelte';
	import '$lib/flow.css';
	import '$lib/pages/admin/admin.css';

	/** How many accounts one listing carries. A rendering bound, not a policy
	 *  one: the search narrows the list rather than paging it. */
	const PAGE_SIZE = 50;

	const queryClient = useQueryClient();
	const now = Date.now();

	let typed = $state('');
	let applied = $state('');
	let impersonating = $state<string | null>(null);
	let refusal = $state<string | null>(null);
	let sessionsFor = $state<IdentityUser | null>(null);
	let grantingFor = $state<{ org: string; name: string; email: string } | null>(null);
	/** Sign-in counts, by identity account, for the rows an operator has
	 *  opened the drawer on. Not preloaded: the identity service lists
	 *  sessions one account at a time, so filling this column for a page of
	 *  fifty would be fifty requests to answer a question about one. */
	let counted = $state<Record<string, number>>({});
	/** Declared once rather than written inline at the drawer, and that
	 *  matters: the drawer reports its count from an effect, and an inline
	 *  arrow is a new function on every render — the effect would see a changed
	 *  dependency, report again, re-render the page, and Svelte would stop the
	 *  loop with `effect_update_depth_exceeded`. */
	function noteSessionCount(userId: string, count: number) {
		counted = { ...counted, [userId]: count };
	}

	/** The grant dialog's element, so it opens modal rather than inline: an
	 *  `open` attribute renders the dialog in the flow with no backdrop and no
	 *  escape key. Guarded on the element's own state, because `showModal` on a
	 *  dialog that is already modal throws. */
	let grantDialog = $state<HTMLDialogElement | null>(null);
	$effect(() => {
		if (grantDialog !== null && !grantDialog.open) {
			grantDialog.showModal();
		}
	});

	const users = createQuery(() => ({
		queryKey: queryKeys.identityUsers(applied),
		queryFn: () => listIdentityUsers(applied, PAGE_SIZE),
		retry: false
	}));

	/** The platform's half: every app user, with their organisation, its plan
	 *  and the identity trail's last sign-in. Unfiltered — the identity list
	 *  is the one that searches — and joined to it in the browser, because no
	 *  one database role can read both planes. */
	const platform = createQuery(() => ({
		queryKey: queryKeys.adminUsers,
		queryFn: () => api.adminUsers()
	}));

	/**
	 * The two halves, joined.
	 *
	 * `rows` is read in the first arm of the markup below rather than only
	 * inside the arm that draws the table, and that is load-bearing: a query
	 * result notifies its reader only about the fields that reader has actually
	 * touched. The platform read resolves while the identity list is still
	 * pending, so a `platform.data` first touched inside the `{:else}` arm lands
	 * with nobody listening — and the organisation, plan and sign-in columns
	 * stay empty for the life of the page, with no error to explain it.
	 */
	const appUsers = $derived(platform.data?.users ?? []);
	const merged = $derived(mergeUsers(users.data ?? [], appUsers));
	const rows = $derived(merged.rows);

	type UserFilter = 'all' | 'unverified' | 'banned' | 'no-platform';
	const USER_FILTERS: readonly { id: UserFilter; label: string }[] = [
		{ id: 'all', label: 'All' },
		{ id: 'unverified', label: 'Unverified' },
		{ id: 'banned', label: 'Banned' },
		{ id: 'no-platform', label: 'No app user' }
	];
	let filter = $state<UserFilter>('all');
	function inFilter(row: (typeof rows)[number], chip: UserFilter): boolean {
		switch (chip) {
			case 'all':
				return true;
			case 'unverified':
				return !row.identity.emailVerified && !row.identity.banned;
			case 'banned':
				return row.identity.banned === true;
			case 'no-platform':
				return row.platform === null;
		}
	}
	const shown = $derived(rows.filter((row) => inFilter(row, filter)));
	const trailVisible = $derived(signInTrailVisible(appUsers));
	const listRefusal = $derived(
		users.error instanceof AuthFailure ? identityAdminRefusal(users.error.status) : null
	);

	async function reload() {
		await queryClient.invalidateQueries({ queryKey: queryKeys.identityUsers(applied) });
	}

	function refusalOf(failure: Error, fallback: string): string {
		return failure instanceof AuthFailure ? failure.message : fallback;
	}

	const banning = createMutation(() => ({
		mutationFn: (input: { id: string; reason: string }) => banIdentityUser(input.id, input.reason),
		onSuccess: async () => {
			await reload();
			toast('info', 'Account banned. They can no longer sign in.');
		},
		onError: (failure: Error) => toast('error', refusalOf(failure, 'The account was not banned. Try again.'))
	}));

	const unbanning = createMutation(() => ({
		mutationFn: (id: string) => unbanIdentityUser(id),
		onSuccess: async () => {
			await reload();
			toast('info', 'Account unbanned.');
		},
		onError: (failure: Error) => toast('error', refusalOf(failure, 'The account was not unbanned. Try again.'))
	}));

	const settingRole = createMutation(() => ({
		mutationFn: (input: { id: string; role: IdentityRole }) =>
			setIdentityRole(input.id, input.role),
		onSuccess: async () => {
			await reload();
			toast('info', 'Role changed.');
		},
		onError: (failure: Error) => toast('error', refusalOf(failure, 'The role was not changed. Try again.'))
	}));

	function search(event: SubmitEvent) {
		event.preventDefault();
		applied = typed.trim();
	}

	function clearSearch() {
		typed = '';
		applied = '';
	}

	function ban(user: IdentityUser) {
		const reason = prompt(
			`Ban ${user.email}? They can no longer sign in, starting now.\n\n` +
				'Reason (saved on the account and shown here):'
		);
		if (reason === null) {
			return;
		}
		banning.mutate({ id: user.id, reason });
	}

	function changeRole(user: IdentityUser, event: Event) {
		const chosen = (event.currentTarget as HTMLSelectElement).value as IdentityRole;
		if (chosen === (user.role ?? 'user')) {
			return;
		}
		settingRole.mutate({ id: user.id, role: chosen });
	}

	/** A plan set from here is written against the user's organisation, not
	 *  against the user: entitlements are a tenant's, and a tenant with two
	 *  members has one plan. The dialog says so. */
	async function granted() {
		await queryClient.invalidateQueries({ queryKey: queryKeys.adminUsers });
		grantingFor = null;
	}

	/**
	 * Sign in as this account, and carry the console with it.
	 *
	 * The identity call alone would leave the two planes disagreeing: the
	 * identity session would be theirs while every page still read the
	 * operator's own organisation. `impersonateAndCarry` re-establishes the app
	 * session from the impersonated identity, and undoes the impersonation if
	 * that exchange refuses — an unverified target is refused by design, and a
	 * half-impersonated console is the one state that must not persist.
	 */
	async function impersonate(user: IdentityUser) {
		const sure = confirm(
			`Sign in as ${user.email}?\n\n` +
				'Every page you open is their account, and anything you do is done as them. ' +
				'It is logged in the impersonation trail.'
		);
		if (!sure) {
			return;
		}
		impersonating = user.id;
		refusal = null;
		try {
			await impersonateAndCarry(user.id);
			// Everything cached was read as the operator's own tenant.
			queryClient.clear();
			await invalidateAll();
			await goto('/');
		} catch (failure) {
			refusal =
				failure instanceof Error
					? `${user.email}: ${failure.message}`
					: `${user.email}: the impersonation was refused.`;
			// The rollback inside impersonateAndCarry can itself fail, leaving the
			// identity session impersonating with no matching app session. Re-read
			// it rather than trusting the cache, so the banner raises itself over
			// that state instead of it passing unseen.
			await queryClient.invalidateQueries({ queryKey: queryKeys.identitySession });
		} finally {
			impersonating = null;
		}
	}

	function joined(at: string | Date | null | undefined): number | null {
		if (at === null || at === undefined) {
			return null;
		}
		const parsed = new Date(at);
		return Number.isNaN(parsed.getTime()) ? null : parsed.getTime();
	}
</script>

<div class="page flow-page">
	<PageHead
		icon="users"
		title="Identity users"
		description="Sign-in accounts, matched to the app users they created."
	/>

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
		<div class="flow">
			<section class="flow-section">
				<form class="op-filters" role="search" onsubmit={search}>
					<label class="sr-only" for="identity-search">Search accounts by email address</label>
					<input
						id="identity-search"
						name="q"
						type="search"
						placeholder="Search by email address…"
						bind:value={typed}
					/>
					<Button tier="additive" type="submit" icon="search">Search</Button>
					{#if applied.length > 0}
						<Button tier="outline" onclick={clearSearch}>Clear</Button>
					{/if}
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
							<span class="c">{rows.filter((row) => inFilter(row, chip.id)).length}</span>
						</button>
					{/each}
				</div>

				{#if refusal}
					<Banner tone="bad" title="The impersonation was refused">{refusal}</Banner>
				{/if}

				{#if platform.isError}
					<!-- The identity list still draws: an operator who cannot read
					     the platform's half can still ban and impersonate. -->
					<p class="flow-warn">Organisation, plan and last sign-in could not be loaded.</p>
				{/if}

				<!-- `rows.length` here, not just `users.isPending`: see the comment on
				     `appUsers`. It also keeps a table on screen through a refetch. -->
				{#if users.isPending && rows.length === 0}
					<p class="quiet">Loading accounts…</p>
				{:else if users.isError}
					<!-- Before the empty arm: `listRefusal` is null for anything that
					     is not an `AuthFailure`, and without this a failed read
					     reported an empty identity service. -->
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
					<div class="flow-table-wrap op-table op-tall op-users">
						<table class="flow-table">
							<thead>
								<tr>
									<th>Account</th>
									<th>Organisation and plan</th>
									<th>
										<span class="op-th">
											Identity
											<Explain title="Identity and sign-ins" label="">
												<p>
													Verification, ban and role are the identity service's. You cannot
													impersonate an unverified account; the attempt is undone, not left
													half-done.
												</p>
												<p>
													Sign-ins load one account at a time. The count fills in when you open a
													row.
												</p>
												{#if !trailVisible && appUsers.length > 0}
													<p>
														No row shows a last sign-in. Most likely this server cannot see the
														identity schema, not that nobody has signed in.
													</p>
												{/if}
											</Explain>
										</span>
									</th>
									<th class="num">Seen</th>
									<th class="num">Actions</th>
								</tr>
							</thead>
							<tbody>
								{#each shown as row (row.identity.id)}
									{@const user = row.identity}
									{@const joinedAt = joined(user.createdAt)}
									{@const signedIn = row.platform?.last_sign_in_at ?? null}
									<tr>
										<td class="op-cell" data-label="Account">
											<span class="t" title={user.name || user.email}>{user.name || user.email}</span>
											<span class="s" title={user.email}>{user.email}</span>
										</td>
										<!-- The plan sits under the organisation because that is whose
										     it is: a tenant with two members has one plan. -->
										<td class="op-cell" data-label="Organisation and plan">
											{#if row.platform === null}
												<span class="s">no platform user yet</span>
											{:else}
												{@const platformRow = row.platform}
												<a
													class="t op-name"
													href={`/admin/orgs/${platformRow.organisation.org}`}
													title={platformRow.organisation.name}
												>
													{platformRow.organisation.name}
												</a>
												<div class="op-plan">
													<StatusPill
														tone={platformRow.plan === 'free' ? 'soon' : 'ok'}
														label={planName(platformRow.plan)}
													/>
													<Button
														tier="quiet"
														small
														onclick={() =>
															(grantingFor = {
																org: platformRow.organisation.org,
																name: platformRow.organisation.name,
																email: user.email
															})}
													>
														Set plan
													</Button>
												</div>
											{/if}
										</td>
										<td class="op-cell" data-label="Identity">
											<div class="op-plan">
												{#if user.banned}
													<StatusPill tone="bad" label="banned" />
												{:else if user.emailVerified}
													<StatusPill tone="ok" label="verified" />
												{:else}
													<StatusPill tone="warn" label="unverified" />
												{/if}
												{#if user.banned && user.banReason}<span class="s">{user.banReason}</span>{/if}
												<label class="sr-only" for={`role-${user.id}`}>Role for {user.email}</label>
												<select
													id={`role-${user.id}`}
													value={user.role ?? 'user'}
													disabled={settingRole.isPending}
													onchange={(event) => changeRole(user, event)}
												>
													{#each IDENTITY_ROLES as role (role)}
														<option value={role}>{role}</option>
													{/each}
												</select>
											</div>
										</td>
										<td class="op-cell num" data-label="Seen">
											<span class="t" title={joinedAt === null ? undefined : utcInstant(joinedAt)}>
												{joinedAt === null ? '—' : `joined ${agoLabel(joinedAt, now)}`}
											</span>
											<span class="s" title={signedIn === null ? undefined : utcInstant(signedIn)}>
												{signedIn === null ? 'no sign-in recorded' : `signed in ${agoLabel(signedIn, now)}`}
											</span>
											<span class="s">{sessionWords(counted[user.id] ?? null)}</span>
										</td>
										<td data-label="Actions">
											<div class="op-acts">
												<Button
													tier="primary"
													small
													icon="log-out"
													disabled={impersonating !== null}
													reason={impersonating !== null
														? 'An impersonation is already starting.'
														: undefined}
													onclick={() => impersonate(user)}
												>
													{impersonating === user.id ? 'Starting…' : 'Impersonate'}
												</Button>
												<Button tier="outline" small icon="laptop" onclick={() => (sessionsFor = user)}>
													Sign-ins
												</Button>
												{#if user.banned}
													<Button
														tier="outline"
														small
														disabled={unbanning.isPending}
														reason={unbanning.isPending ? 'Unbanning.' : undefined}
														onclick={() => unbanning.mutate(user.id)}
													>
														Unban
													</Button>
												{:else}
													<Button
														tier="outline"
														small
														danger
														disabled={banning.isPending}
														reason={banning.isPending ? 'Banning.' : undefined}
														onclick={() => ban(user)}
													>
														Ban
													</Button>
												{/if}
											</div>
										</td>
									</tr>
								{:else}
									<tr><td colspan="5" class="quiet">None under this filter.</td></tr>
								{/each}
							</tbody>
						</table>
					</div>
					<p class="op-foot">
						{shown.length} of {rows.length}
						{rows.length === 1 ? 'account' : 'accounts'}, newest first, at most {PAGE_SIZE}.
						{#if merged.unlinked > 0}
							{merged.unlinked} platform {merged.unlinked === 1 ? 'user is' : 'users are'} not on
							this page: the search does not match, or they were created outside sign-up.
						{/if}
					</p>
				{/if}
			</section>
		</div>
	{/if}
</div>

{#if sessionsFor !== null}
	{@const target = sessionsFor}
	<SessionsDialog
		userId={target.id}
		email={target.email}
		onClose={() => (sessionsFor = null)}
		onCount={noteSessionCount}
	/>
{/if}

{#if grantingFor !== null}
	{@const target = grantingFor}
	<dialog
		bind:this={grantDialog}
		aria-labelledby="grant-title"
		onclose={() => (grantingFor = null)}
	>
		<div class="dialog-body">
			<h2 id="grant-title">Set the plan for {target.name}</h2>
			<p>
				A plan belongs to the organisation, not the person. This applies to {target.name},
				the account {target.email} and everyone else in it share.
			</p>
			<GrantPlanForm org={target.org} onGranted={granted} />
			<div class="actions">
				<Button tier="outline" onclick={() => (grantingFor = null)}>Close</Button>
			</div>
		</div>
	</dialog>
{/if}
