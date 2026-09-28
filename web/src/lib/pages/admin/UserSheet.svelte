<script lang="ts">
	// One account, opened from the user list: who they are, their organisation
	// and plan, and every action an operator takes on them. A side sheet on a
	// desktop, a bottom sheet on a phone; Escape or a press outside closes it.

	import { createMutation, useQueryClient } from '@tanstack/svelte-query';
	import { goto, invalidateAll } from '$app/navigation';
	import { sessionWords, type AdminUserRow } from '$lib/admin';
	import { ApiFailure, api } from '$lib/api';
	import {
		AuthFailure,
		banIdentityUser,
		impersonateAndCarry,
		setIdentityRole,
		unbanIdentityUser,
		type IdentityRole
	} from '$lib/auth-client';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import Explain from '$lib/Explain.svelte';
	import { queryKeys } from '$lib/query';
	import Icon from '$lib/Icon.svelte';
	import Sheet, { type SheetHandle } from '$lib/Sheet.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import Toggle from '$lib/Toggle.svelte';
	import GrantPlanForm from './GrantPlanForm.svelte';
	import { planName, planTone } from './admin-view';
	import DeleteUserDialog from './DeleteUserDialog.svelte';
	import SessionsDialog from './SessionsDialog.svelte';
	import { CHIP_WORDS, displayName, initials, joinedAt, userChips } from './users-view';

	let {
		row,
		now,
		trailVisible,
		selfId,
		sessionCount,
		onCount,
		onChanged,
		onClose
	}: {
		row: AdminUserRow;
		now: number;
		/** Whether any row carries a sign-in; see `signInTrailVisible`. */
		trailVisible: boolean;
		/** The signed-in operator's own identity id, which cannot delete or ban
		 *  itself. */
		selfId: string | null;
		sessionCount: number | null;
		onCount: (userId: string, count: number) => void;
		/** Re-read both halves of the list after a change. */
		onChanged: () => Promise<void>;
		onClose: () => void;
	} = $props();

	const queryClient = useQueryClient();

	let sheet = $state<SheetHandle>();
	let showSessions = $state(false);
	let showDelete = $state(false);
	let settingPlan = $state(false);
	let banning = $state(false);
	let banReason = $state('');
	let impersonating = $state(false);
	let refusal = $state<string | null>(null);

	const user = $derived(row.identity);
	const platform = $derived(row.platform);
	const chips = $derived(userChips(row));
	const isSelf = $derived(selfId !== null && selfId === user.id);
	const joined = $derived(joinedAt(user));
	const signedIn = $derived(platform?.last_sign_in_at ?? null);
	const isAdmin = $derived(user.role === 'admin');

	function refusalOf(failure: Error, fallback: string): string {
		return failure instanceof AuthFailure || failure instanceof ApiFailure
			? failure.message
			: fallback;
	}

	const ban = createMutation(() => ({
		mutationFn: (input: { id: string; reason: string }) => banIdentityUser(input.id, input.reason),
		onSuccess: async () => {
			banning = false;
			banReason = '';
			await onChanged();
			toast('info', 'Account banned. They can no longer sign in.');
		},
		onError: (failure: Error) =>
			toast('error', refusalOf(failure, 'The account was not banned. Try again.'))
	}));

	const unban = createMutation(() => ({
		mutationFn: (id: string) => unbanIdentityUser(id),
		onSuccess: async () => {
			await onChanged();
			toast('info', 'Account unbanned.');
		},
		onError: (failure: Error) =>
			toast('error', refusalOf(failure, 'The account was not unbanned. Try again.'))
	}));

	const role = createMutation(() => ({
		mutationFn: (input: { id: string; role: IdentityRole }) =>
			setIdentityRole(input.id, input.role),
		onSuccess: async (_, input) => {
			await onChanged();
			toast(
				'info',
				input.role === 'admin'
					? 'They are now identity admin.'
					: 'They are no longer identity admin.'
			);
		},
		onError: async (failure: Error) => {
			await onChanged();
			toast('error', refusalOf(failure, 'The role was not changed. Try again.'));
		}
	}));

	const marking = createMutation(() => ({
		mutationFn: (input: { id: string; operator: boolean }) =>
			input.operator ? api.adminGrantOperator(input.id) : api.adminRevokeOperator(input.id),
		onSuccess: async (_, input) => {
			await onChanged();
			toast(
				'info',
				input.operator ? 'They are now an operator.' : 'They are no longer an operator.'
			);
		},
		onError: async (failure: Error) => {
			await onChanged();
			toast('error', refusalOf(failure, 'The operator marking was not changed. Try again.'));
		}
	}));

	/**
	 * Sign in as this account, and carry the console with it.
	 *
	 * `impersonateAndCarry` re-establishes the app session from the
	 * impersonated identity and undoes the impersonation if that exchange
	 * refuses — an unverified target is refused by design, and a
	 * half-impersonated console is the one state that must not persist.
	 */
	async function impersonate() {
		const sure = confirm(
			`Sign in as ${user.email}?\n\n` +
				'Every page you open is their account, and anything you do is done as them. ' +
				'It is logged in the impersonation trail.'
		);
		if (!sure) return;
		impersonating = true;
		refusal = null;
		try {
			await impersonateAndCarry(user.id);
			// Everything cached was read as the operator's own tenant.
			queryClient.clear();
			await invalidateAll();
			await goto('/');
		} catch (failure) {
			refusal = failure instanceof Error ? failure.message : 'The impersonation was refused.';
			// The rollback inside impersonateAndCarry can itself fail, leaving
			// the identity session impersonating with no matching app session.
			// Re-read it so the banner raises itself over that state.
			await queryClient.invalidateQueries({ queryKey: queryKeys.identitySession });
		} finally {
			impersonating = false;
		}
	}

	async function granted() {
		settingPlan = false;
		await onChanged();
	}

	async function deleted(email: string) {
		showDelete = false;
		await onChanged();
		toast('info', `${email} is deleted.`);
		sheet?.close();
	}
</script>

<Sheet labelledby="user-sheet-title" {onClose} bind:handle={sheet}>
	<header class="sheet-head">
		<span class="avatar" aria-hidden="true">{initials(user.name, user.email)}</span>
		<div class="who">
			<h2 id="user-sheet-title">{displayName(user)}</h2>
			<span class="email">{user.email}</span>
			{#if chips.length > 0}
				<span class="chips">
					{#each chips as chip (chip)}
						<StatusPill tone={CHIP_WORDS[chip].tone} label={CHIP_WORDS[chip].label} />
					{/each}
				</span>
			{/if}
		</div>
		<button type="button" class="close" aria-label="Close" onclick={() => sheet?.close()}>
			<Icon name="x" />
		</button>
	</header>

	<div class="sheet-body">
		{#if refusal}
			<Banner tone="bad" title="The impersonation was refused">{refusal}</Banner>
		{/if}

		<section class="block">
			<h3>Account</h3>
			<dl>
				<dt>Email</dt>
				<dd>
					{#if user.emailVerified}
						<StatusPill tone="ok" label="verified" />
					{:else}
						<StatusPill tone="warn" label="not verified" />
					{/if}
				</dd>
				<dt>Joined</dt>
				<dd title={joined === null ? undefined : utcInstant(joined)}>
					{joined === null ? '—' : agoLabel(joined, now)}
				</dd>
				<dt>Last sign-in</dt>
				<dd title={signedIn === null ? undefined : utcInstant(signedIn)}>
					{#if signedIn !== null}
						{agoLabel(signedIn, now)}
					{:else}
						none recorded
						{#if !trailVisible}
							<Explain title="Why no sign-in shows" label="">
								<p>
									No account on this page shows a last sign-in. Most likely this server cannot see
									the identity schema, not that nobody has signed in.
								</p>
							</Explain>
						{/if}
					{/if}
				</dd>
				<dt>Signed in on</dt>
				<dd>
					<button type="button" class="link" onclick={() => (showSessions = true)}>
						{sessionCount === null ? 'See devices' : sessionWords(sessionCount)}
					</button>
				</dd>
			</dl>
		</section>

		<section class="block">
			<h3>Organisation and plan</h3>
			{#if platform === null}
				<p class="quiet">
					No app user yet. They made a sign-in account but have not opened the app.
				</p>
			{:else}
				<div class="org">
					<a class="org-name" href={`/admin/orgs/${platform.organisation.org}`}>
						{platform.organisation.name}
					</a>
					<StatusPill tone={planTone(platform.plan)} label={planName(platform.plan)} />
				</div>
				{#if settingPlan}
					<p class="quiet">
						A plan belongs to the organisation, so everyone in {platform.organisation.name} gets it.
					</p>
					<GrantPlanForm
						org={platform.organisation.org}
						current={platform.plan}
						onGranted={granted}
					/>
					<Button tier="quiet" small onclick={() => (settingPlan = false)}>Cancel</Button>
				{:else}
					<Button tier="outline" small icon="gift" onclick={() => (settingPlan = true)}>
						Set plan
					</Button>
				{/if}
			{/if}
		</section>

		<section class="block">
			<h3>Access</h3>
			<div class="access">
				{#key `${platform?.operator}-${marking.isPending}`}
					<Toggle
						label="Operator"
						checked={platform?.operator === true}
						disabled={platform === null || isSelf || marking.isPending}
						onchange={(on) => marking.mutate({ id: user.id, operator: on })}
					/>
				{/key}
				<span class="access-why">
					{#if platform === null}
						They must open the app once first.
					{:else if isSelf}
						This is you. Another operator can change it.
					{:else}
						Opens this Admin area and every organisation.
					{/if}
				</span>
				<Explain title="What an operator is" label="">
					<p>
						An operator can open this Admin area and read every organisation on the platform: their
						resources, listings and sync history.
					</p>
					<p>
						Only an operator can make someone else one, and nobody can remove their own, so there is
						always someone left who can. Every change is recorded with who made it.
					</p>
				</Explain>

				{#key `${user.role}-${role.isPending}`}
					<Toggle
						label="Identity admin"
						checked={isAdmin}
						disabled={isSelf || role.isPending}
						onchange={(on) => role.mutate({ id: user.id, role: on ? 'admin' : 'user' })}
					/>
				{/key}
				<span class="access-why">
					{isSelf
						? 'This is you. Another admin can change it.'
						: 'Can ban, sign in as, and delete accounts.'}
				</span>
				<Explain title="What identity admin is" label="">
					<p>
						Identity admin is a role on the sign-in service. It lets someone ban people, sign in as
						them, and delete their sign-in accounts. This users list needs it too.
					</p>
					<p>
						It is separate from operator on purpose. Most people who run the platform need both; a
						helper who only reads organisations needs operator alone.
					</p>
				</Explain>
			</div>
		</section>

		<section class="block">
			<h3>Actions</h3>
			<div class="acts">
				<Button
					tier="primary"
					icon="log-out"
					disabled={impersonating || isSelf || user.banned === true}
					reason={isSelf
						? 'This is you.'
						: user.banned
							? 'Unban them first.'
							: impersonating
								? 'Starting.'
								: undefined}
					onclick={impersonate}
				>
					{impersonating ? 'Starting…' : 'Sign in as them'}
				</Button>

				{#if user.banned}
					<Button
						tier="outline"
						disabled={unban.isPending}
						reason={unban.isPending ? 'Unbanning.' : undefined}
						onclick={() => unban.mutate(user.id)}
					>
						Unban
					</Button>
				{:else if !banning}
					<Button
						tier="outline"
						icon="lock"
						disabled={isSelf}
						reason={isSelf ? 'This is you.' : undefined}
						onclick={() => (banning = true)}
					>
						Ban
					</Button>
				{/if}
			</div>
			{#if user.banned && user.banReason}
				<p class="quiet">Banned because: {user.banReason}</p>
			{/if}
			{#if banning && !user.banned}
				<form
					class="ban"
					onsubmit={(event) => {
						event.preventDefault();
						ban.mutate({ id: user.id, reason: banReason });
					}}
				>
					<label for="ban-reason">Why? They can no longer sign in, starting now.</label>
					<input id="ban-reason" bind:value={banReason} placeholder="Reason, shown here later" />
					<div class="acts">
						<Button tier="quiet" small onclick={() => (banning = false)}>Cancel</Button>
						<Button
							tier="primary"
							small
							danger
							type="submit"
							disabled={ban.isPending}
							reason={ban.isPending ? 'Banning.' : undefined}
						>
							Ban them
						</Button>
					</div>
				</form>
			{/if}
		</section>

		<section class="block danger-zone">
			<h3>Delete</h3>
			<p class="quiet">
				Deletes their sign-in account{platform === null
					? ''
					: ', their organisation and everything in it'}.
			</p>
			<Button
				tier="outline"
				danger
				icon="x"
				disabled={isSelf || platform?.operator === true}
				reason={isSelf
					? 'You cannot delete yourself.'
					: platform?.operator
						? 'Operators cannot be deleted. Turn Operator off first.'
						: undefined}
				onclick={() => (showDelete = true)}
			>
				Delete account
			</Button>
		</section>
	</div>
</Sheet>

{#if showSessions}
	<SessionsDialog
		userId={user.id}
		email={user.email}
		onClose={() => (showSessions = false)}
		{onCount}
	/>
{/if}

{#if showDelete}
	<DeleteUserDialog {row} onClose={() => (showDelete = false)} onDeleted={deleted} />
{/if}

<style>
	.sheet-head {
		display: grid;
		grid-template-columns: auto 1fr auto;
		gap: var(--s-3);
		align-items: start;
		padding: var(--s-4) var(--s-4) var(--s-3);
		border-bottom: 1px solid var(--line);
	}

	.avatar {
		display: inline-grid;
		place-items: center;
		width: 44px;
		height: 44px;
		border-radius: var(--r-pill);
		background: var(--accent-soft);
		color: var(--primary);
		font-weight: 600;
		font-size: 15px;
	}

	.who {
		display: grid;
		gap: 2px;
		min-width: 0;
	}

	.who h2 {
		margin: 0;
		font-size: 16px;
		font-weight: 600;
		overflow-wrap: anywhere;
	}

	.email {
		color: var(--muted);
		font-size: 13px;
		overflow-wrap: anywhere;
	}

	.chips {
		display: flex;
		flex-wrap: wrap;
		gap: var(--s-1);
		margin-top: var(--s-1);
	}

	.close {
		display: inline-grid;
		place-items: center;
		width: 34px;
		height: 34px;
		border: 0;
		border-radius: var(--r-pill);
		background: transparent;
		color: var(--muted);
		cursor: pointer;
	}

	.close:hover {
		background: var(--hover);
		color: var(--ink);
	}

	.sheet-body {
		flex: 1;
		overflow-y: auto;
		padding: var(--s-2) var(--s-4) calc(var(--s-4) + env(safe-area-inset-bottom));
		display: grid;
		align-content: start;
		gap: var(--s-2);
	}

	.block {
		padding: var(--s-3) 0;
		display: grid;
		gap: var(--s-2);
	}

	.block + .block {
		border-top: 1px solid var(--line);
	}

	.block h3 {
		margin: 0;
		font-size: 12px;
		font-weight: 600;
		letter-spacing: 0.04em;
		text-transform: uppercase;
		color: var(--muted);
	}

	dl {
		display: grid;
		grid-template-columns: max-content 1fr;
		gap: var(--s-2) var(--s-4);
		margin: 0;
		font-size: 13.5px;
	}

	dt {
		color: var(--muted);
	}

	dd {
		margin: 0;
		display: flex;
		align-items: center;
		gap: var(--s-2);
		min-width: 0;
	}

	.access {
		display: grid;
		grid-template-columns: max-content 1fr auto;
		align-items: center;
		gap: var(--s-2) var(--s-3);
		font-size: 13.5px;
	}

	.access-why {
		color: var(--muted);
		font-size: 13px;
	}

	.link {
		border: 0;
		padding: 0;
		background: none;
		color: var(--primary);
		font: inherit;
		text-decoration: underline;
		text-underline-offset: 2px;
		cursor: pointer;
	}

	.org {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		flex-wrap: wrap;
	}

	.org-name {
		font-weight: 600;
		color: var(--ink);
		overflow-wrap: anywhere;
	}

	.acts {
		display: flex;
		flex-wrap: wrap;
		gap: var(--s-2);
	}

	.ban {
		display: grid;
		gap: var(--s-2);
		font-size: 13px;
	}

	.ban input {
		min-height: var(--control-h);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		background: var(--card);
		color: var(--text);
		padding-inline: 12px;
		font: inherit;
	}

	.quiet {
		margin: 0;
		color: var(--muted);
		font-size: 13px;
	}

	.danger-zone h3 {
		color: var(--bad-ink);
	}
</style>
