<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill, { type Tone } from '$lib/StatusPill.svelte';
	import '$lib/pages/admin/admin.css';

	const trail = createQuery(() => ({
		queryKey: queryKeys.adminImpersonations,
		queryFn: () => api.adminImpersonations()
	}));

	/** Absent means the identity schema is not visible from this database, which
	 *  is a different fact from nobody having been impersonated. */
	const events = $derived(trail.data?.impersonations);
	const now = Date.now();

	function verb(event: string): { label: string; tone: Tone } {
		return event === 'user_impersonated'
			? { label: 'started', tone: 'bad' }
			: { label: 'stopped', tone: 'ok' };
	}
</script>

<div class="page">
	<PageHead
		icon="copy"
		title="Impersonations"
		description="When an identity admin signed in as somebody else, newest first."
	>
		{#snippet aside()}
			<StatusPill tone="soon" label="newest 100" />
		{/snippet}
	</PageHead>

	{#if trail.isPending}
		<Panel><p class="quiet">Loading the audit trail…</p></Panel>
	{:else if trail.isError}
		<Panel><p class="quiet">We could not load the audit trail.</p></Panel>
	{:else if events === undefined}
		<Placeholder
			icon="copy"
			headline="The identity audit trail is not available here"
			body="This database has no identity schema, so there is nothing to read. That does not mean nobody was impersonated."
		/>
	{:else}
		<Panel>
			{#if events.length === 0}
				<Placeholder
					icon="circle-check"
					headline="Nobody has been impersonated"
					body="The audit trail loaded and shows no impersonations."
				/>
			{:else}
				<div class="op-table op-tall">
					<table>
						<thead>
							<tr>
								<th>Event</th>
								<th>Actor</th>
								<th>Target</th>
								<th>When</th>
								<th>From</th>
							</tr>
						</thead>
						<tbody>
							{#each events as row (`${row.event}-${row.at}-${row.actor}-${row.target}`)}
								<tr>
									<td data-label="Event">
										<StatusPill tone={verb(row.event).tone} label={verb(row.event).label} />
									</td>
									<td class="op-cell" data-label="Actor">
										<span class="t mono" title={row.actor}>{row.actor}</span>
									</td>
									<td class="op-cell" data-label="Target">
										<span class="t mono" title={row.target}>{row.target}</span>
									</td>
									<td class="op-cell" data-label="When">
										<span class="t">{agoLabel(row.at, now)}</span>
										<span class="s mono">{utcInstant(row.at)}</span>
									</td>
									<td class="mono" data-label="From">{row.ip ?? '—'}</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
				<p class="foot-note">
					Actor and target are identity-service subject ids, not app user ids; the two number
					users separately. Reading this page needs operator access, which the identity admin
					role does not give, so the people who can impersonate and the people who can read this
					page are kept apart.
				</p>
			{/if}
		</Panel>
	{/if}
</div>
