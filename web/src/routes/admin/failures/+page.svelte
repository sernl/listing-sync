<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import { agoLabel } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { queryKeys } from '$lib/query';

	const failures = createQuery(() => ({
		queryKey: queryKeys.adminFailures,
		queryFn: () => api.adminFailedWrites().then((view) => view.writes)
	}));

	const rows = $derived(failures.data ?? []);
	const now = Date.now();
</script>

<div class="page">
	<PageHead
		icon="✕"
		title="Failed writes"
		description="Write attempts that recorded a failure, newest first, across every tenant."
	>
		{#snippet aside()}
			<span class="tag-note">newest 100</span>
		{/snippet}
	</PageHead>

	<Panel>
		{#if failures.isPending}
			<p class="quiet">Reading the failed attempts…</p>
		{:else if failures.isError}
			<p class="quiet">The failed attempts could not be read.</p>
		{:else if rows.length === 0}
			<div class="clear">
				<span class="big" aria-hidden="true">✓</span>
				No write attempt on the platform has recorded a failure.
			</div>
		{:else}
			<div class="tbl-wrap scroll-tbl">
				<table>
					<thead>
						<tr>
							<th>Opened</th>
							<th>Attempt</th>
							<th>Failure</th>
							<th>Item</th>
							<th>Tenant</th>
						</tr>
					</thead>
					<tbody>
						{#each rows as write (write.attempt)}
							<tr>
								<td class="title-cell">
									<div class="t">{agoLabel(write.opened_at, now)}</div>
									<div class="s">
										{write.settled_at === undefined
											? 'not settled'
											: `settled ${agoLabel(write.settled_at, now)}`}
									</div>
								</td>
								<td class="title-cell">
									<div class="t mono">{write.attempt.slice(0, 8)}…</div>
									<div class="s">{write.state}</div>
								</td>
								<td>
									<span class="pill bad">{write.failure_code}</span>
									{#if write.ambiguity_cause !== undefined}
										<div class="s">{write.ambiguity_cause}</div>
									{/if}
								</td>
								<td class="title-cell">
									<div class="t mono">{write.item.slice(0, 8)}…</div>
									{#if write.item_failure_code !== undefined}
										<div class="s">
											<span class="badge">{write.item_failure_code}</span>
											{write.item_failure_detail ?? ''}
										</div>
									{/if}
								</td>
								<td class="title-cell">
									<div class="t">
										<a class="link" href={`/admin/orgs/${write.org}`}>{write.org.slice(0, 8)}…</a>
									</div>
									<div class="s mono">mapping {write.mapping.slice(0, 8)}…</div>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
			<p class="foot-note">
				{rows.length}
				{rows.length === 1 ? 'attempt' : 'attempts'}. The attempt's own failure code sits beside the
				owning item's, which can differ: an attempt can fail without the item settling failed.
			</p>
		{/if}
	</Panel>
</div>
