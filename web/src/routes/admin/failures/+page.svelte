<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import { agoLabel } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import '$lib/pages/admin/admin.css';

	const failures = createQuery(() => ({
		queryKey: queryKeys.adminFailures,
		queryFn: () => api.adminFailedWrites().then((view) => view.writes)
	}));

	const rows = $derived(failures.data ?? []);
	const now = Date.now();
</script>

<div class="page">
	<PageHead
		icon="circle-x"
		title="Failed and stranded writes"
		description="Write attempts that failed, and attempts stranded after their run ended. Stranded first, then newest, across all accounts."
	>
		{#snippet aside()}
			<StatusPill tone="soon" label="newest 100" />
		{/snippet}
	</PageHead>

	<Panel>
		{#if failures.isPending}
			<p class="quiet">Loading failed writes…</p>
		{:else if failures.isError}
			<p class="quiet">We could not load failed writes.</p>
		{:else if rows.length === 0}
			<Placeholder
				icon="circle-check"
				headline="No failed writes"
				body="Nothing is stranded in flight either. New ones appear here as they happen."
			/>
		{:else}
			<div class="op-table op-tall">
				<table>
					<thead>
						<tr>
							<th>Opened</th>
							<th>Attempt</th>
							<th>Failure</th>
							<th>Item</th>
							<th>Account</th>
						</tr>
					</thead>
					<tbody>
						{#each rows as write (write.attempt)}
							<tr>
								<td class="op-cell" data-label="Opened">
									<span class="t">{agoLabel(write.opened_at, now)}</span>
									<span class="s">
										{write.settled_at === undefined
											? 'not settled'
											: `settled ${agoLabel(write.settled_at, now)}`}
									</span>
								</td>
								<td class="op-cell" data-label="Attempt">
									<span class="t mono" title={write.attempt}>{write.attempt.slice(0, 8)}…</span>
									<span class="s">{write.state}</span>
								</td>
								<td data-label="Failure">
									<span>
										{#if write.failure_code !== undefined}
											<StatusPill tone="bad" label={write.failure_code} />
										{:else}
											<StatusPill tone="run" label="in flight past its lease" />
										{/if}
										{#if write.ambiguity_cause !== undefined}
											<span class="s">{write.ambiguity_cause}</span>
										{/if}
									</span>
								</td>
								<td class="op-cell" data-label="Item">
									<span class="t mono" title={write.item}>{write.item.slice(0, 8)}…</span>
									{#if write.item_failure_code !== undefined}
										<span class="s" title={write.item_failure_detail}>
											{write.item_failure_code} · {write.item_failure_detail ?? ''}
										</span>
									{/if}
								</td>
								<td class="op-cell" data-label="Account">
									<span class="t">
										<a class="link" title={write.org} href={`/admin/orgs/${write.org}`}
											>{write.org.slice(0, 8)}…</a
										>
									</span>
									<span class="s mono" title={write.mapping}>mapping {write.mapping.slice(0, 8)}…</span>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
			<p class="foot-note">
				{rows.length}
				{rows.length === 1 ? 'attempt' : 'attempts'}. Each attempt's failure code is shown beside its
				item's. They can differ: an attempt can fail while its item does not.
			</p>
		{/if}
	</Panel>
</div>
