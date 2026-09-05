<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import { api } from '$lib/api';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import '$lib/pages/admin/admin.css';

	const orgs = createQuery(() => ({
		queryKey: queryKeys.adminOrgs,
		queryFn: () => api.adminOrgs().then((view) => view.orgs)
	}));

	const rows = $derived(orgs.data ?? []);
	const now = Date.now();
</script>

<div class="page">
	<PageHead
		icon="building-2"
		title="Organisations"
		description="Every tenant on the platform, newest first, with what each holds."
	/>

	<Panel>
		{#if orgs.isPending}
			<p class="quiet">Reading the tenant list…</p>
		{:else if orgs.isError}
			<p class="quiet">The tenant list could not be read.</p>
		{:else if rows.length === 0}
			<Placeholder
				icon="building-2"
				headline="No organisation has been provisioned yet"
				body="A tenant appears here the first time somebody signs in and the session exchange provisions them one."
			/>
		{:else}
			<div class="op-table">
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
								<td class="op-cell" data-label="Organisation">
									<span class="t" title={row.name}>
										<a class="link" href={`/admin/orgs/${row.org}`}>{row.name}</a>
									</span>
									<!-- The slug where the tenant has claimed one, because that
									     is what a seller quotes in a support email; the row id
									     otherwise, which is all there was to identify them by. -->
									<span class="s mono" title={row.slug ?? row.org}
										>{row.slug ?? row.org}</span
									>
								</td>
								<td class="num" data-label="Products">{row.products}</td>
								<td class="num" data-label="Mappings">{row.mappings}</td>
								<td class="num" data-label="Connections">{row.connections}</td>
								<td class="num" data-label="Users">{row.users}</td>
								<td class="num" data-label="Created" title={utcInstant(row.created_at)}>
									{agoLabel(row.created_at, now)}
								</td>
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
