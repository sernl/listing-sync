<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { mergeUsers } from '$lib/admin';
	import { api } from '$lib/api';
	import { listIdentityUsers, type IdentityUser } from '$lib/auth-client';
	import Button from '$lib/Button.svelte';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import Explain from '$lib/Explain.svelte';
	import Icon from '$lib/Icon.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import SessionsDialog from '$lib/pages/admin/SessionsDialog.svelte';
	import { impersonationKey, openImpersonations } from '$lib/pages/admin/admin-view';
	import { partyOf } from '$lib/pages/admin/users-view';
	import '$lib/flow.css';
	import '$lib/pages/admin/admin.css';

	/** How many identity accounts are read to put names to the trail. Wider
	 *  than the users page's listing: the trail's parties are often older
	 *  accounts than the newest fifty. */
	const DIRECTORY_SIZE = 500;

	const trail = createQuery(() => ({
		queryKey: queryKeys.adminImpersonations,
		queryFn: () => api.adminImpersonations()
	}));

	/** Names for the subject ids. Optional: an operator who is not an identity
	 *  admin still reads the trail, by id. */
	const directory = createQuery(() => ({
		queryKey: queryKeys.identityDirectory,
		queryFn: () =>
			listIdentityUsers({ search: '', limit: DIRECTORY_SIZE }).then((page) => page.users),
		retry: false
	}));

	/** The platform half, for the organisation under a target's name. */
	const platform = createQuery(() => ({
		queryKey: queryKeys.adminUsers,
		queryFn: () => api.adminUsers()
	}));

	/** Absent means the identity schema is not visible from this database, which
	 *  is a different fact from nobody having been impersonated. */
	const events = $derived(trail.data?.impersonations);
	const open = $derived(openImpersonations(events ?? []));
	const rows = $derived(mergeUsers(directory.data ?? [], platform.data?.users ?? []));
	const accounts = $derived(
		new Map<string, IdentityUser>(rows.map((row) => [row.identity.id, row.identity]))
	);
	const orgOf = $derived(
		new Map<string, string>(
			rows.flatMap((row) =>
				row.platform === null ? [] : [[row.identity.id, row.platform.organisation.name] as const]
			)
		)
	);
	let filter = $state<'all' | 'open'>('all');
	const shown = $derived(
		(events ?? []).filter((row) => filter === 'all' || open.has(impersonationKey(row)))
	);
	/** The target whose sign-ins are open: ending an impersonation is ending
	 *  the target's session, which the sign-ins dialog does with its own
	 *  confirmation. */
	let ending = $state<{ id: string; label: string } | null>(null);
	function noCount() {}
	const now = Date.now();
</script>

<div class="page flow-page">
	<PageHead
		icon="copy"
		title="Impersonations"
		description="When an admin signed in as somebody else. Newest 100."
	/>

	<div class="flow">
		{#if trail.isPending}
			<p class="quiet">Loading the audit trail…</p>
		{:else if trail.isError}
			<p class="quiet">We could not load the audit trail.</p>
		{:else if events === undefined}
			<Placeholder
				icon="copy"
				headline="The identity audit trail is not available here"
				body="This database has no identity schema. That does not mean nobody was impersonated."
			/>
		{:else if events.length === 0}
			<Placeholder
				icon="circle-check"
				headline="Nobody has been impersonated"
				body="The trail is empty."
			/>
		{:else}
			<section class="imp">
				<div class="imp-bar">
					<div class="op-filters" role="group" aria-label="Show">
						<button
							type="button"
							class="op-chip"
							aria-pressed={filter === 'all'}
							onclick={() => (filter = 'all')}
						>
							All <span class="c">{events.length}</span>
						</button>
						<button
							type="button"
							class="op-chip"
							aria-pressed={filter === 'open'}
							onclick={() => (filter = 'open')}
						>
							Not stopped <span class="c">{open.size}</span>
						</button>
					</div>
					<Explain title="Reading the trail" label="How to read this">
						<p>
							Each row is an admin (left) signing in as someone (right). Hover a name to see its
							identity id.
						</p>
						<p>
							A name shows as a short id when the account is not in the identity list: it was
							deleted, or you are not an identity admin.
						</p>
						<p>
							Reading this page needs operator access, which the identity admin role does not give.
							So who can impersonate and who can read this are kept apart.
						</p>
						<p>
							“Not stopped” is a start with no later stop for the same pair. End opens the person's
							sign-ins, where signing them out ends it.
						</p>
					</Explain>
				</div>

				{#if directory.isError}
					<p class="flow-warn">Names could not be loaded, so people show by id.</p>
				{/if}

				<ul class="imp-list" aria-label="Impersonations">
					{#each shown as row (impersonationKey(row))}
						{@const live = open.has(impersonationKey(row))}
						{@const actor = partyOf(row.actor, accounts)}
						{@const target = partyOf(row.target, accounts)}
						{@const targetOrg = orgOf.get(row.target)}
						<li class:live>
							<span class="imp-state">
								{#if row.event !== 'user_impersonated'}
									<StatusPill tone="ok" label="stopped" />
								{:else if live}
									<StatusPill tone="bad" label="not stopped" />
								{:else}
									<StatusPill tone="soon" label="started" />
								{/if}
							</span>
							<span class="imp-parties">
								<span class="imp-party" class:mono={!actor.known} title={row.actor}>
									{actor.label}
								</span>
								<span class="imp-arrow" aria-label="signed in as">
									<Icon name="arrow-right-left" size={14} />
								</span>
								<span class="imp-party" class:mono={!target.known} title={row.target}>
									<span class="t">{target.label}</span>
									{#if targetOrg}<span class="s">{targetOrg}</span>{/if}
								</span>
							</span>
							<span class="imp-when">
								<span title={utcInstant(row.at)}>{agoLabel(row.at, now)}</span>
								{#if row.ip}<span class="s mono">from {row.ip}</span>{/if}
							</span>
							<span class="imp-act">
								{#if live}
									<Button
										small
										danger
										icon="log-out"
										onclick={() => (ending = { id: row.target, label: target.label })}
									>
										End
									</Button>
								{/if}
							</span>
						</li>
					{:else}
						<li class="quiet imp-none">None under this filter.</li>
					{/each}
				</ul>
			</section>
		{/if}
	</div>
</div>

{#if ending !== null}
	<SessionsDialog
		userId={ending.id}
		email={ending.label}
		onClose={() => (ending = null)}
		onCount={noCount}
	/>
{/if}

<style>
	.imp {
		display: grid;
		gap: var(--s-3);
	}

	.imp-bar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--s-2);
		flex-wrap: wrap;
	}

	.imp-list {
		list-style: none;
		margin: 0;
		padding: 0;
		border: 1px solid var(--line);
		border-radius: var(--r-card);
		background: var(--card);
		overflow: hidden;
	}

	.imp-list > li {
		display: grid;
		grid-template-columns: 110px minmax(0, 1fr) auto 72px;
		align-items: center;
		gap: var(--s-3);
		padding: var(--s-3) var(--s-4);
	}

	.imp-list > li + li {
		border-top: 1px solid var(--line);
	}

	.imp-list > li.live {
		background: var(--bad-soft);
	}

	.imp-parties {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		min-width: 0;
		flex-wrap: wrap;
	}

	.imp-party {
		display: grid;
		min-width: 0;
		font-weight: 600;
		overflow-wrap: anywhere;
	}

	.imp-party .s {
		font-weight: 400;
		color: var(--muted);
		font-size: 12.5px;
	}

	.imp-arrow {
		display: inline-flex;
		color: var(--muted);
	}

	.imp-when {
		display: grid;
		justify-items: end;
		font-size: 13px;
		white-space: nowrap;
	}

	.imp-when .s {
		color: var(--muted);
		font-size: 12px;
	}

	.imp-act {
		display: flex;
		justify-content: flex-end;
	}

	.imp-party.mono {
		font-weight: 400;
	}

	.imp-none {
		padding: var(--s-4);
	}

	@media (max-width: 760px) {
		.imp-list > li {
			grid-template-columns: minmax(0, 1fr) auto;
			grid-template-areas:
				'state act'
				'parties parties'
				'when when';
			row-gap: var(--s-2);
		}

		.imp-state {
			grid-area: state;
		}

		.imp-parties {
			grid-area: parties;
		}

		.imp-when {
			grid-area: when;
			justify-items: start;
		}

		.imp-act {
			grid-area: act;
		}

		.imp-list > li.imp-none {
			display: block;
		}
	}
</style>
