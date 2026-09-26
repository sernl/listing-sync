<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import Explain from '$lib/Explain.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { WRITE_FILTERS, writeKind, type WriteKind } from '$lib/pages/admin/admin-view';
	import '$lib/flow.css';
	import '$lib/pages/admin/admin.css';

	const failures = createQuery(() => ({
		queryKey: queryKeys.adminFailures,
		queryFn: () => api.adminFailedWrites().then((view) => view.writes)
	}));

	const rows = $derived(failures.data ?? []);
	let filter = $state<WriteKind | 'all'>('all');
	const shown = $derived(
		rows.filter((write) => filter === 'all' || writeKind(write) === filter)
	);
	const now = Date.now();
</script>

<div class="page flow-page">
	<PageHead
		icon="circle-x"
		title="Failed writes"
		description="Stranded first, then newest. The newest 100, all accounts."
	/>

	<div class="flow">
		{#if failures.isPending}
			<p class="quiet">Loading failed writes…</p>
		{:else if failures.isError}
			<p class="quiet">We could not load failed writes.</p>
		{:else if rows.length === 0}
			<Placeholder
				icon="circle-check"
				headline="No failed writes"
				body="Nothing is stranded in flight either."
			/>
		{:else}
			<section class="flow-section">
				<div class="op-filters" role="group" aria-label="Show">
					{#each WRITE_FILTERS as chip (chip.id)}
						<button
							type="button"
							class="op-chip"
							aria-pressed={filter === chip.id}
							onclick={() => (filter = chip.id)}
						>
							{chip.label}
							<span class="c">
								{rows.filter((write) => chip.id === 'all' || writeKind(write) === chip.id).length}
							</span>
						</button>
					{/each}
				</div>

				<div class="flow-table-wrap op-table op-tall">
					<table class="flow-table">
						<thead>
							<tr>
								<th>
									<span class="op-th">
										Failure
										<Explain title="Failure and item codes" label="">
											<p>
												A red pill is the attempt's own failure code. An amber “stranded” pill is an
												attempt still in flight past its lease after its run ended.
											</p>
											<p>
												The item column carries the item's failure code. The two can differ: an
												attempt can fail while its item does not.
											</p>
										</Explain>
									</span>
								</th>
								<th>Opened</th>
								<th>Attempt</th>
								<th>Item</th>
								<th>Account</th>
							</tr>
						</thead>
						<tbody>
							{#each shown as write (write.attempt)}
								<tr>
									<td class="op-cell" data-label="Failure">
										{#if write.failure_code !== undefined}
											<StatusPill tone="bad" label={write.failure_code} />
										{:else}
											<StatusPill tone="warn" label="stranded" />
										{/if}
										{#if write.ambiguity_cause !== undefined}
											<span class="s" title={write.ambiguity_cause}>{write.ambiguity_cause}</span>
										{/if}
									</td>
									<td class="op-cell" data-label="Opened">
										<span class="t" title={utcInstant(write.opened_at)}>
											{agoLabel(write.opened_at, now)}
										</span>
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
									<td class="op-cell" data-label="Item">
										<span class="t mono" title={write.item}>{write.item.slice(0, 8)}…</span>
										{#if write.item_failure_code !== undefined}
											<span class="s" title={write.item_failure_detail}>
												{write.item_failure_code}{write.item_failure_detail === undefined
													? ''
													: ` · ${write.item_failure_detail}`}
											</span>
										{/if}
									</td>
									<td class="op-cell" data-label="Account">
										<a class="t op-name mono" title={write.org} href={`/admin/orgs/${write.org}`}>
											{write.org.slice(0, 8)}…
										</a>
										<span class="s mono" title={write.mapping}>mapping {write.mapping.slice(0, 8)}…</span>
									</td>
								</tr>
							{:else}
								<tr><td colspan="5" class="quiet">None under this filter.</td></tr>
							{/each}
						</tbody>
					</table>
				</div>
				<p class="op-foot">{shown.length} of {rows.length} attempts.</p>
			</section>
		{/if}
	</div>
</div>
