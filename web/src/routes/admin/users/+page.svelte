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
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
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
			<form class="op-search" role="search" onsubmit={search}>
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

			{#if refusal}
				<Banner tone="bad" title="The impersonation was refused">{refusal}</Banner>
			{/if}

			{#if users.isPending}
				<p class="quiet">Reading the identity accounts…</p>
			{:else if users.isError}
				<!-- Before the empty arm, and not folded into it: `listRefusal`
				     is null for anything that is not an `AuthFailure`, and
				     without this the page reported an empty identity service
				     from a read that failed, with `retry: false` so it never
				     corrected itself. -->
				<Placeholder
					icon="users"
					headline="The identity accounts could not be read"
					body="The request did not come back with an answer we can act on, so this page
						cannot say whether the service holds any accounts. Reloading is the only
						thing worth trying from here."
				/>
			{:else if rows.length === 0}
				<Placeholder
					icon="users"
					headline={applied.length === 0
						? 'The identity service holds no accounts'
						: 'No account matches that search'}
					body={applied.length === 0
						? 'The first account appears here the moment somebody registers.'
						: `No account's address contains “${applied}”.`}
				/>
			{:else}
				<div class="op-table op-tall">
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
									<td class="op-cell" data-label="Account">
										<span class="t" title={user.name || user.email}>{user.name || user.email}</span>
										<span class="s" title={user.email}>{user.email}</span>
									</td>
									<td data-label="Role">
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
									<td class="op-cell" data-label="State">
										{#if user.banned}
											<StatusPill tone="bad" label="banned" />
											{#if user.banReason}<span class="s">{user.banReason}</span>{/if}
										{:else if user.emailVerified}
											<StatusPill tone="ok" label="verified" />
										{:else}
											<StatusPill tone="run" label="unverified" />
										{/if}
									</td>
									<td
										class="num"
										data-label="Joined"
										title={joinedAt === null ? undefined : utcInstant(joinedAt)}
									>
										{joinedAt === null ? '—' : agoLabel(joinedAt, now)}
									</td>
									<td data-label="Actions">
										<div class="op-acts">
											{#if user.banned}
												<Button
													tier="outline"
													small
													disabled={unbanning.isPending}
													reason={unbanning.isPending ? 'An unban is in flight.' : undefined}
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
													reason={banning.isPending ? 'A ban is in flight.' : undefined}
													onclick={() => ban(user)}
												>
													Ban
												</Button>
											{/if}
											<Button
												tier="outline"
												small
												disabled={impersonating !== null}
												reason={impersonating !== null
													? 'An impersonation is already starting.'
													: undefined}
												onclick={() => impersonate(user)}
											>
												{impersonating === user.id ? 'Starting…' : 'Impersonate'}
											</Button>
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
