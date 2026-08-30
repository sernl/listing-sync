<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { agoLabel } from '$lib/elapsed';
	import { api } from '$lib/api';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { queryKeys } from '$lib/query';

	const orgs = createQuery(() => ({
		queryKey: queryKeys.adminOrgs,
		queryFn: () => api.adminOrgs().then((view) => view.orgs)
	}));

	const rows = $derived(orgs.data ?? []);
	const now = Date.now();
</script>

<div class="page">
	<PageHead
		icon="⌂"
		title="Organisations"
		description="Every tenant on the platform, newest first, with what each holds."
	/>

	<Panel>
		{#if orgs.isPending}
			<p class="quiet">Reading the tenant list…</p>
		{:else if orgs.isError}
			<p class="quiet">The tenant list could not be read.</p>
		{:else if rows.length === 0}
			<div class="clear">
				<span class="big" aria-hidden="true">⌂</span>
				No organisation has been provisioned yet.
			</div>
		{:else}
			<div class="tbl-wrap">
				<table>
					<thead>
						<tr>
							<th>Organisation</th>
							<th class="num">Products</th>
							<th class="num">Mappings</th>
							<th class="num">Connections</th>
							<th class="num">Users</th>
							<th class="num">Created</th>
						</tr>
					</thead>
					<tbody>
						{#each rows as row (row.org)}
							<tr>
								<td class="title-cell">
									<div class="t"><a class="link" href={`/admin/orgs/${row.org}`}>{row.name}</a></div>
									<div class="s mono">{row.org}</div>
								</td>
								<td class="num">{row.products}</td>
								<td class="num">{row.mappings}</td>
								<td class="num">{row.connections}</td>
								<td class="num">{row.users}</td>
								<td class="num">{agoLabel(row.created_at, now)}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
			<p class="foot-note">
				{rows.length}
				{rows.length === 1 ? 'organisation' : 'organisations'}. Counts are read across the tenant
				fence on the backoffice connection, so they include rows no single tenant can see.
			</p>
		{/if}
	</Panel>
</div>
