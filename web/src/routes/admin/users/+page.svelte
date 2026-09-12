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
	import { PLANS } from '$lib/generated/plans';
	import type { Plan } from '$lib/generated/vocab';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import GrantPlanForm from '$lib/pages/admin/GrantPlanForm.svelte';
	import SessionsDialog from '$lib/pages/admin/SessionsDialog.svelte';
	import '$lib/pages/account/account.css';
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

	/** The plan's own name where the price table carries it, and the wire word
	 *  otherwise, exactly as the organisation page reads one. */
	function planName(plan: Plan): string {
		return PLANS.find((row) => row.id === plan)?.name ?? plan;
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
		description="Accounts in the identity service, joined to the platform users they provisioned."
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

			{#if platform.isError}
				<!-- The identity list still draws: an operator who cannot read
				     the platform's half can still ban and impersonate, and a
				     page that refused entirely would take those away too. -->
				<Banner tone="warn" title="The platform's half of this list could not be read">
					Organisation, plan and last sign-in are missing from every row below. The identity
					service's own facts are unaffected.
				</Banner>
			{/if}

			<!-- `rows.length` here, not just `users.isPending`: see the comment on
			     `appUsers`. It also keeps a table on screen through a refetch
			     rather than flashing this line over rows that are still good. -->
			{#if users.isPending && rows.length === 0}
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
				<div class="op-table op-tall op-users">
					<table>
						<thead>
							<tr>
								<th>Account</th>
								<th>Organisation and plan</th>
								<th>Identity</th>
								<th class="num">Seen</th>
								<th class="num">Actions</th>
							</tr>
						</thead>
						<tbody>
							{#each rows as row (row.identity.id)}
								{@const user = row.identity}
								{@const joinedAt = joined(user.createdAt)}
								{@const signedIn = row.platform?.last_sign_in_at ?? null}
								<tr>
									<td class="op-cell" data-label="Account">
										<span class="t" title={user.name || user.email}>{user.name || user.email}</span>
										<span class="s" title={user.email}>{user.email}</span>
									</td>
									<!-- The plan sits under the organisation rather than in a
									     column of its own, because that is whose it is: an
									     entitlement belongs to the tenant, and a tenant with two
									     members has one plan rather than two. -->
									<td class="op-cell" data-label="Organisation and plan">
										{#if row.platform === null}
											<span class="s">no platform user yet</span>
										{:else}
											{@const platformRow = row.platform}
											<a
												class="t"
												href={`/admin/orgs/${platformRow.organisation.org}`}
												title={platformRow.organisation.name}
											>
												{platformRow.organisation.name}
											</a>
											<span class="s">{platformRow.organisation.slug ?? 'no slug claimed'}</span>
											<div class="op-plan">
												<StatusPill
													tone={platformRow.plan === 'free' ? 'soon' : 'ok'}
													label={planName(platformRow.plan)}
												/>
												<Button
													tier="outline"
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
										{#if user.banned}
											<StatusPill tone="bad" label="banned" />
											{#if user.banReason}<span class="s">{user.banReason}</span>{/if}
										{:else if user.emailVerified}
											<StatusPill tone="ok" label="verified" />
										{:else}
											<StatusPill tone="run" label="unverified" />
										{/if}
										<!-- The role sits under the verification pill rather than in a
										     column of its own: both are the identity plane's word about
										     this account, and the platform's three columns to the left
										     are already as wide as the band allows. -->
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
									<td class="op-cell num" data-label="Seen">
										<span class="t" title={joinedAt === null ? undefined : utcInstant(joinedAt)}>
											{joinedAt === null ? '—' : `joined ${agoLabel(joinedAt, now)}`}
										</span>
										<span class="s" title={signedIn === null ? undefined : utcInstant(signedIn)}>
											{signedIn === null
												? 'no sign-in recorded'
												: `signed in ${agoLabel(signedIn, now)}`}
										</span>
										<span class="s">{sessionWords(counted[user.id] ?? null)}</span>
									</td>
									<td data-label="Actions">
										<div class="op-acts">
											<Button tier="outline" small onclick={() => (sessionsFor = user)}>
												Sign-ins
											</Button>
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
					{#if merged.unlinked > 0}
						{merged.unlinked} platform
						{merged.unlinked === 1 ? 'user is' : 'users are'} not on this page — either their
						search does not match, or they carry no identity subject at all, which a user
						provisioned outside the sign-up flow does.
					{/if}
					Sign-ins are read one account at a time, because the identity service lists them per
					account; the count fills in when you open a row.
					{#if !trailVisible && appUsers.length > 0}
						No row carries a last sign-in, which most likely means this deployment's API cannot
						see the identity schema rather than that nobody has ever signed in.
					{/if}
				</p>
			{/if}
		</Panel>
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
				An entitlement belongs to the organisation, not to the person: this grant applies to
				{target.name}, which is {target.email}'s tenant and anybody else's in it.
			</p>
			<GrantPlanForm org={target.org} onGranted={granted} />
			<div class="actions">
				<Button tier="outline" onclick={() => (grantingFor = null)}>Close</Button>
			</div>
		</div>
	</dialog>
{/if}
