<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import { agoLabel } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';

	const trail = createQuery(() => ({
		queryKey: queryKeys.adminImpersonations,
		queryFn: () => api.adminImpersonations()
	}));

	/** Absent means the identity schema is not visible from this database, which
	 *  is a different fact from nobody having been impersonated. */
	const events = $derived(trail.data?.impersonations);
	const now = Date.now();

	function verb(event: string): { label: string; tone: string } {
		return event === 'user_impersonated'
			? { label: 'started', tone: 'bad' }
			: { label: 'stopped', tone: 'ok' };
	}

	function instant(at: number): string {
		return new Date(at).toISOString().replace('T', ' ').slice(0, 19) + 'Z';
	}
</script>

<div class="page">
	<PageHead
		icon="⧉"
		title="Impersonations"
		description="Every time an identity admin signed in as somebody else, newest first."
	/>

	{#if trail.isPending}
		<Panel><p class="quiet">Reading the audit trail…</p></Panel>
	{:else if trail.isError}
		<Panel><p class="quiet">The audit trail could not be read.</p></Panel>
	{:else if events === undefined}
		<Placeholder
			icon="⧉"
			headline="The identity audit trail is not visible from here"
			body="This database carries no identity schema, so there is no record to read. That is
				not the same as nobody having been impersonated."
		/>
	{:else}
		<Panel>
			{#if events.length === 0}
				<div class="clear">
					<span class="big" aria-hidden="true">✓</span>
					Nobody has been impersonated.
				</div>
			{:else}
				<div class="tbl-wrap scroll-tbl">
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
									<td><span class="pill {verb(row.event).tone}">{verb(row.event).label}</span></td>
									<td class="mono">{row.actor}</td>
									<td class="mono">{row.target}</td>
									<td class="title-cell">
										<div class="t">{agoLabel(row.at, now)}</div>
										<div class="s mono">{instant(row.at)}</div>
									</td>
									<td class="mono">{row.ip ?? '—'}</td>
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
