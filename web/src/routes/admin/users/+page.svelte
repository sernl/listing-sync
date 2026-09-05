<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto, invalidateAll } from '$app/navigation';
	import { identityAdminRefusal } from '$lib/admin';
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
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';

	/** How many accounts one listing carries. A rendering bound, not a policy
	 *  one: the search narrows the list rather than paging it. */
	const PAGE_SIZE = 50;

	const queryClient = useQueryClient();
	const now = Date.now();

	let typed = $state('');
	let applied = $state('');
	let impersonating = $state<string | null>(null);
	let refusal = $state<string | null>(null);

	const users = createQuery(() => ({
		queryKey: queryKeys.identityUsers(applied),
		queryFn: () => listIdentityUsers(applied, PAGE_SIZE),
		retry: false
	}));

	const rows = $derived(users.data ?? []);
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
			toast('info', 'Account banned. Their sessions no longer sign in.');
		},
		onError: (failure: Error) => toast('error', refusalOf(failure, 'The account was not banned.'))
	}));

	const unbanning = createMutation(() => ({
		mutationFn: (id: string) => unbanIdentityUser(id),
		onSuccess: async () => {
			await reload();
			toast('info', 'Account unbanned.');
		},
		onError: (failure: Error) => toast('error', refusalOf(failure, 'The account was not unbanned.'))
	}));

	const settingRole = createMutation(() => ({
		mutationFn: (input: { id: string; role: IdentityRole }) =>
			setIdentityRole(input.id, input.role),
		onSuccess: async () => {
			await reload();
			toast('info', 'Role changed.');
		},
		onError: (failure: Error) => toast('error', refusalOf(failure, 'The role was not changed.'))
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
			`Ban ${user.email}? They stop being able to sign in immediately.\n\n` +
				'Why (stored on the account and shown here afterwards):'
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
				'Every page you then open is their workspace, and anything you do is done as ' +
				'them. It is recorded in the impersonation trail either way.'
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

<div class="page">
	<PageHead
		icon="users"
		title="Identity users"
		description="Accounts in the identity service. These are not platform users: the two planes number their people separately."
	/>

	{#if listRefusal === 'not-identity-admin'}
		<Placeholder
			icon="users"
			headline="Your identity account is not an identity admin"
			body="Operating the platform and administering the identity service are two markings,
				granted separately and by hand. You hold the first, which is what let you reach
				this page; this list needs the second. Nothing here can grant it to you — ask the
				person who runs the identity service."
		/>
	{:else if listRefusal === 'unreadable'}
		<Placeholder
			icon="users"
			headline="The identity accounts could not be listed"
			body="The identity service did not answer with something we can act on. Reloading is
				the only thing worth trying from here."
		/>
	{:else}
		<Panel>
			<form class="filter-bar" role="search" onsubmit={search}>
				<label class="sr-only" for="identity-search">Search accounts by email address</label>
				<input
					id="identity-search"
					name="q"
					type="search"
					placeholder="Search by email address…"
					bind:value={typed}
				/>
				<button class="cta small" type="submit">Search</button>
				{#if applied.length > 0}
					<button class="btn small" type="button" onclick={clearSearch}>Clear</button>
				{/if}
			</form>

			{#if refusal}
				<div class="notice">{refusal}</div>
			{/if}

			{#if users.isPending}
				<p class="quiet">Reading the identity accounts…</p>
			{:else if rows.length === 0}
				<div class="clear">
					<span class="big" aria-hidden="true">☺</span>
					{applied.length === 0
						? 'The identity service holds no accounts.'
						: `No account's address contains “${applied}”.`}
				</div>
			{:else}
				<div class="tbl-wrap scroll-tbl">
					<table>
						<thead>
							<tr>
								<th>Account</th>
								<th>Role</th>
								<th>State</th>
								<th class="num">Joined</th>
								<th class="num">Actions</th>
							</tr>
						</thead>
						<tbody>
							{#each rows as user (user.id)}
								{@const joinedAt = joined(user.createdAt)}
								<tr>
									<td class="title-cell">
										<div class="t" title={user.name || user.email}>{user.name || user.email}</div>
										<div class="s" title={user.email}>{user.email}</div>
									</td>
									<td>
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
									</td>
									<td>
										{#if user.banned}
											<span class="pill bad">banned</span>
											{#if user.banReason}<div class="s">{user.banReason}</div>{/if}
										{:else if user.emailVerified}
											<span class="pill ok">verified</span>
										{:else}
											<span class="pill run">unverified</span>
										{/if}
									</td>
									<td class="num" title={joinedAt === null ? undefined : utcInstant(joinedAt)}>
										{joinedAt === null ? '—' : agoLabel(joinedAt, now)}
									</td>
									<td>
										<div class="row-actions">
											{#if user.banned}
												<button
													class="btn small"
													type="button"
													disabled={unbanning.isPending}
													onclick={() => unbanning.mutate(user.id)}>Unban</button
												>
											{:else}
												<button
													class="btn small danger"
													type="button"
													disabled={banning.isPending}
													onclick={() => ban(user)}>Ban</button
												>
											{/if}
											<button
												class="btn small"
												type="button"
												disabled={impersonating !== null}
												onclick={() => impersonate(user)}
											>
												{impersonating === user.id ? 'Starting…' : 'Impersonate'}
											</button>
										</div>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
				<p class="foot-note">
					{rows.length}
					{rows.length === 1 ? 'account' : 'accounts'}, newest first, at most {PAGE_SIZE}. An
					unverified account cannot be impersonated: the session exchange refuses one, and the
					impersonation is undone rather than left half-applied.
				</p>
			{/if}
		</Panel>
	{/if}
</div>
