<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api, type SyncHealthView } from '$lib/api';
	import Explain from '$lib/Explain.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { queryKeys } from '$lib/query';
	import { HEALTH_FILTERS, HEALTH_ROWS, type HealthGroup } from '$lib/pages/admin/admin-view';
	import '$lib/flow.css';
	import '$lib/pages/admin/admin.css';

	const health = createQuery(() => ({
		queryKey: queryKeys.operator,
		queryFn: () => api.adminSyncHealth()
	}));

	const ledger = $derived(health.data);
	let filter = $state<HealthGroup | 'all'>('all');
	const shown = $derived(
		HEALTH_ROWS.filter((row) => filter === 'all' || row.group === filter)
	);

	/** One figure from the ledger, or an em dash.
	 *
	 *  Guarded on the figure rather than on the ledger: a body that arrived
	 *  without a field leaves the ledger defined and the field undefined, and
	 *  guarding only the ledger printed the string "undefined". */
	function count(key: keyof SyncHealthView): string {
		const figure = ledger?.[key];
		return figure === undefined ? '—' : String(figure);
	}

	/** A group's total, for its chip. */
	function groupTotal(group: HealthGroup | 'all'): string {
		if (ledger === undefined) {
			return '—';
		}
		if (group === 'all') {
			return String(ledger.items);
		}
		if (group === 'outcome') {
			return String(ledger.settled);
		}
		return String(
			HEALTH_ROWS.filter((row) => row.group === group).reduce(
				(total, row) => total + (ledger[row.key] ?? 0),
				0
			)
		);
	}
</script>

<div class="page flow-page">
	<PageHead
		icon="heart-pulse"
		title="Sync health"
		description="Every sync item across all accounts, by state."
	/>

	<div class="flow">
		{#if health.isPending}
			<p class="quiet">Loading sync health…</p>
		{:else if health.isError}
			<p class="quiet">We could not load sync health.</p>
		{:else}
			<p class="op-facts-line">
				<span><b>{count('jobs')}</b> sync runs</span>
				<span><b>{count('items')}</b> items</span>
				<span><b>{count('settled')}</b> settled</span>
				<span><b class:op-bad={ledger?.failed !== undefined && ledger.failed > 0}>{count('failed')}</b> failed</span>
			</p>

			<section class="flow-section">
				<div class="op-filters" role="group" aria-label="Show">
					{#each HEALTH_FILTERS as chip (chip.id)}
						<button
							type="button"
							class="op-chip"
							aria-pressed={filter === chip.id}
							onclick={() => (filter = chip.id)}
						>
							{chip.label}
							<span class="c">{groupTotal(chip.id)}</span>
						</button>
					{/each}
				</div>

				<div class="flow-table-wrap op-table op-keep">
					<table class="flow-table">
						<thead>
							<tr>
								<th>
									<span class="op-th">
										State
										<Explain title="What each state means" label="">
											{#each HEALTH_ROWS as row (row.key)}
												<p><b>{row.label}</b>: {row.note}</p>
											{/each}
											<p>
												The settled outcomes add up to the Settled figure. There is no single
												health score, because an average would hide the number that matters.
											</p>
										</Explain>
									</span>
								</th>
								<th class="num">Items</th>
							</tr>
						</thead>
						<tbody>
							{#each shown as row (row.key)}
								<tr>
									<td data-label="State"><StatusPill tone={row.tone} label={row.label} /></td>
									<td
										class="num"
										class:op-flag={row.tone === 'bad' && (ledger?.[row.key] ?? 0) > 0}
										data-label="Items"
									>
										{count(row.key)}
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			</section>
		{/if}
	</div>
</div>
