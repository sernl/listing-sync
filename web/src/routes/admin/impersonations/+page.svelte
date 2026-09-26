<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import Explain from '$lib/Explain.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import SessionsDialog from '$lib/pages/admin/SessionsDialog.svelte';
	import { impersonationKey, openImpersonations } from '$lib/pages/admin/admin-view';
	import '$lib/flow.css';
	import '$lib/pages/admin/admin.css';

	const trail = createQuery(() => ({
		queryKey: queryKeys.adminImpersonations,
		queryFn: () => api.adminImpersonations()
	}));

	/** Absent means the identity schema is not visible from this database, which
	 *  is a different fact from nobody having been impersonated. */
	const events = $derived(trail.data?.impersonations);
	const open = $derived(openImpersonations(events ?? []));
	let filter = $state<'all' | 'open'>('all');
	const shown = $derived(
		(events ?? []).filter((row) => filter === 'all' || open.has(impersonationKey(row)))
	);
	/** The target whose sign-ins are open: ending an impersonation is ending
	 *  the target's session, which the sign-ins dialog does with its own
	 *  confirmation. */
	let ending = $state<string | null>(null);
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
			<Placeholder icon="circle-check" headline="Nobody has been impersonated" body="The trail is empty." />
		{:else}
			<section class="flow-section">
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

				<div class="flow-table-wrap op-table op-tall">
					<table class="flow-table">
						<thead>
							<tr>
								<th>Event</th>
								<th>
									<span class="op-th">
										Actor → target
										<Explain title="Actor and target" label="">
											<p>
												Identity-service subject ids, not app user ids: the two number users
												separately.
											</p>
											<p>
												Reading this page needs operator access, which the identity admin role does
												not give, so who can impersonate and who can read this are kept apart.
											</p>
											<p>
												“Not stopped” is a start with no later stop for the same pair. End opens the
												target's sign-ins, where signing them out ends it.
											</p>
										</Explain>
									</span>
								</th>
								<th>When</th>
								<th>From</th>
								<th></th>
							</tr>
						</thead>
						<tbody>
							{#each shown as row (impersonationKey(row))}
								{@const live = open.has(impersonationKey(row))}
								<tr class:op-strong={live}>
									<td data-label="Event">
										{#if row.event !== 'user_impersonated'}
											<StatusPill tone="ok" label="stopped" />
										{:else if live}
											<StatusPill tone="bad" label="not stopped" />
										{:else}
											<StatusPill tone="soon" label="started" />
										{/if}
									</td>
									<td class="op-cell" data-label="Actor → target">
										<span class="t mono" title={row.actor}>{row.actor}</span>
										<span class="s mono" title={row.target}>→ {row.target}</span>
									</td>
									<td class="op-cell" data-label="When">
										<span class="t" title={utcInstant(row.at)}>{agoLabel(row.at, now)}</span>
									</td>
									<td class="mono" data-label="From">{row.ip ?? '—'}</td>
									<td class="num" data-label="Action">
										{#if live}
											<Button small danger icon="log-out" onclick={() => (ending = row.target)}>
												End
											</Button>
										{/if}
									</td>
								</tr>
							{:else}
								<tr><td colspan="5" class="quiet">None under this filter.</td></tr>
							{/each}
						</tbody>
					</table>
				</div>
			</section>
		{/if}
	</div>
</div>

{#if ending !== null}
	<SessionsDialog
		userId={ending}
		email={ending}
		onClose={() => (ending = null)}
		onCount={noCount}
	/>
{/if}
