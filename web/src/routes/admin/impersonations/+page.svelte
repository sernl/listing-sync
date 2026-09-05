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
		<Panel><p class="quiet">Reading the audit trail…</p></Panel>
	{:else if trail.isError}
		<Panel><p class="quiet">The audit trail could not be read.</p></Panel>
	{:else if events === undefined}
		<Placeholder
			icon="copy"
			headline="The identity audit trail is not visible from here"
			body="This database carries no identity schema, so there is no record to read. That is not the same as nobody having been impersonated."
		/>
	{:else}
		<Panel>
			{#if events.length === 0}
				<Placeholder
					icon="circle-check"
					headline="Nobody has been impersonated"
					body="The trail is readable and empty, which is the fact this page is here to establish."
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
					Actor and target are identity-plane subject ids, not platform user ids: the two planes
					number their users separately. Reading this trail needs the operator marking, which the
					identity admin role does not confer — so the party who can impersonate and the party who
					can read this page are not the same party by construction.
				</p>
			{/if}
		</Panel>
	{/if}
</div>
