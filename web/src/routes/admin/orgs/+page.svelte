<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import { api } from '$lib/api';
	import Explain from '$lib/Explain.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { queryKeys } from '$lib/query';
	import {
		ORG_FILTERS,
		orgInFilter,
		orgMatches,
		orgRows,
		planName,
		planTone,
		type OrgFilter
	} from '$lib/pages/admin/admin-view';
	import '$lib/flow.css';
	import '$lib/pages/admin/admin.css';

	const orgs = createQuery(() => ({
		queryKey: queryKeys.adminOrgs,
		queryFn: () => api.adminOrgs().then((view) => view.orgs)
	}));
	// The plan and last sign-in live on the users answer, not the orgs one.
	const users = createQuery(() => ({
		queryKey: queryKeys.adminUsers,
		queryFn: () => api.adminUsers()
	}));

	let query = $state('');
	let filter = $state<OrgFilter>('all');

	const rows = $derived(orgRows(orgs.data ?? [], users.data?.users ?? []));
	const matched = $derived(rows.filter((row) => orgMatches(row, query)));
	const shown = $derived(matched.filter((row) => orgInFilter(row, filter)));
	const now = Date.now();
</script>

<div class="page flow-page">
	<PageHead icon="building-2" title="Organisations" description="Every account, newest first." />

	<div class="flow">
		{#if orgs.isPending}
			<p class="quiet">Loading accounts…</p>
		{:else if orgs.isError}
			<p class="quiet">We could not load accounts.</p>
		{:else if rows.length === 0}
			<Placeholder
				icon="building-2"
				headline="No organisations yet"
				body="One appears here the first time someone signs in."
			/>
		{:else}
			<section class="flow-section">
				<div class="op-filters" role="search">
					<label class="sr-only" for="org-search">Search organisations</label>
					<input
						id="org-search"
						type="search"
						placeholder="Name, slug or id"
						bind:value={query}
					/>
					{#each ORG_FILTERS as chip (chip.id)}
						<button
							type="button"
							class="op-chip"
							aria-pressed={filter === chip.id}
							onclick={() => (filter = chip.id)}
						>
							{chip.label}
							<span class="c">{matched.filter((row) => orgInFilter(row, chip.id)).length}</span>
						</button>
					{/each}
				</div>

				{#if users.isError}
					<p class="flow-warn">Plans and last sign-ins could not be loaded.</p>
				{/if}

				{#if shown.length === 0}
					<p class="quiet">No organisation matches.</p>
				{:else}
					<div class="flow-table-wrap op-table op-tall">
						<table class="flow-table">
							<thead>
								<tr>
									<th>Organisation</th>
									<th>Plan</th>
									<th class="num">Resources</th>
									<th class="num">Shops</th>
									<th class="num">Users</th>
									<th class="num">
										<span class="op-th">
											Last seen
											<Explain title="Last seen" label="">
												<p>
													The newest sign-in of anyone in the organisation, from the identity
													trail. A dash means no sign-in is recorded, or this server cannot see
													the identity schema.
												</p>
											</Explain>
										</span>
									</th>
								</tr>
							</thead>
							<tbody>
								{#each shown as row (row.org)}
									<tr>
										<td class="op-cell" data-label="Organisation">
											<a class="t op-name" href={`/admin/orgs/${row.org}`} title={row.name}>
												{row.name}
											</a>
											<!-- The slug where the tenant has claimed one, because that
											     is what a seller quotes in a support email. -->
											<span class="s" title={`${row.org} · created ${utcInstant(row.created_at)}`}>
												<span class="mono">{row.slug ?? row.org}</span> · joined
												{agoLabel(row.created_at, now)}
											</span>
										</td>
										<td data-label="Plan">
											{#if row.plan === null}
												<span class="quiet">—</span>
											{:else}
												<StatusPill tone={planTone(row.plan)} label={planName(row.plan)} />
											{/if}
										</td>
										<td class="num" data-label="Resources">{row.products}</td>
										<td class="num" class:op-flag={row.connections === 0} data-label="Shops">
											{row.connections}
										</td>
										<td class="num" data-label="Users">{row.users}</td>
										<td
											class="num"
											data-label="Last seen"
											title={row.lastSeen === null ? undefined : utcInstant(row.lastSeen)}
										>
											{row.lastSeen === null ? '—' : agoLabel(row.lastSeen, now)}
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
					<p class="op-foot">
						{shown.length} of {rows.length}
						{rows.length === 1 ? 'organisation' : 'organisations'}.
					</p>
				{/if}
			</section>
		{/if}
	</div>
</div>
