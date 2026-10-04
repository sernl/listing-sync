<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import Explain from '$lib/Explain.svelte';
	import Icon from '$lib/Icon.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatCard from '$lib/StatCard.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import AbuseSheet from '$lib/pages/admin/AbuseSheet.svelte';
	import {
		FLAG_KIND_LABEL,
		QUERY_LABEL,
		SIGNAL_KIND_LABEL,
		STANDING_LABEL,
		STANDING_TONE,
		patchFlags,
		type AbuseFlagsView,
		type AbuseOrgView,
		type AbuseState
	} from '$lib/pages/admin/abuse';
	import { nzShortDate, nzTime } from '$lib/pages/admin/payments';
	import '$lib/flow.css';
	import '$lib/pages/admin/admin.css';

	const queryClient = useQueryClient();

	let shown = $state<AbuseState>('open');
	let typed = $state('');
	let applied = $state('');

	const flags = createQuery(() => ({
		queryKey: queryKeys.adminAbuseFlags(shown, applied),
		queryFn: () => api.adminAbuseFlags(shown, applied)
	}));

	const counters = $derived(flags.data?.counters);
	const rows = $derived(flags.data?.orgs ?? []);

	let opened = $state<string | null>(null);

	const STATES: { id: AbuseState; label: string }[] = [
		{ id: 'open', label: 'Open' },
		{ id: 'all', label: 'All' }
	];

	function search(event: SubmitEvent) {
		event.preventDefault();
		applied = typed.trim();
	}

	async function acted(view: AbuseOrgView) {
		queryClient.setQueriesData<AbuseFlagsView>({ queryKey: queryKeys.adminAbuse }, (held) =>
			held === undefined ? held : patchFlags(held, view)
		);
		await queryClient.invalidateQueries({ queryKey: queryKeys.adminAbuse });
	}
</script>

<div class="page flow-page">
	<PageHead
		icon="triangle-alert"
		title="Abuse"
		description="Accounts that look like one teacher making several to keep getting free moves."
	/>

	<div class="flow">
		{#if counters}
			<div class="op-stats">
				<StatCard
					icon="building-2"
					tone={counters.open_orgs > 0 ? 'warn' : ''}
					label="Organisations to review"
				>
					{counters.open_orgs}
				</StatCard>
				<StatCard icon="triangle-alert" label="Open flags">{counters.open_flags}</StatCard>
				<StatCard icon="mail" label="Warned">{counters.warned}</StatCard>
				<StatCard icon="pause" label="Limited">{counters.limited}</StatCard>
				<StatCard icon="lock" tone={counters.banned > 0 ? 'bad' : ''} label="Banned">
					{counters.banned}
				</StatCard>
				<StatCard
					icon="circle-x"
					label="Refused identities"
					sub="Emails, shops, devices and cards"
				>
					{counters.banned_identities}
				</StatCard>
			</div>
		{/if}

		<section class="flow-section">
			<form class="op-filters" role="search" onsubmit={search}>
				<label class="sr-only" for="abuse-search">Search flagged organisations</label>
				<input
					id="abuse-search"
					name="q"
					type="search"
					placeholder="Email, IP address, tpt:store, name or slug"
					autocomplete="off"
					bind:value={typed}
				/>
				<Explain title="How search works" label="">
					<p>Press Enter to search.</p>
					<p>
						An email address finds the organisation it belongs to. An IP address, a shop written as
						<span class="mono">tpt:&lt;store id&gt;</span> or
						<span class="mono">tes:&lt;seller id&gt;</span>, or a signal value from a cluster finds
						every organisation seen with it. Anything else matches names and slugs.
					</p>
				</Explain>
				{#each STATES as chip (chip.id)}
					<button
						type="button"
						class="op-chip"
						aria-pressed={shown === chip.id}
						onclick={() => (shown = chip.id)}
					>
						{chip.label}
					</button>
				{/each}
			</form>

			{#if flags.data && flags.data.query !== 'none'}
				<p class="op-foot">{QUERY_LABEL[flags.data.query]}: “{applied}”</p>
			{/if}

			{#if flags.isPending}
				<p class="quiet">Loading flagged organisations…</p>
			{:else if flags.isError}
				<Placeholder
					icon="circle-alert"
					headline="We could not load flagged organisations"
					body="Try reloading the page."
				/>
			{:else if rows.length === 0}
				<Placeholder
					icon="shield-check"
					headline={applied ? 'Nothing matches that search' : 'Nothing to review'}
					body={applied
						? 'No flagged organisation matches what you typed.'
						: shown === 'open'
							? 'No organisation has an open flag right now.'
							: 'No organisation has ever been flagged.'}
				/>
			{:else}
				<div class="flow-table-wrap op-table op-tall">
					<table class="flow-table ab-table">
						<thead>
							<tr>
								<th>Organisation</th>
								<th class="num">
									<span class="op-th">
										Score
										<Explain title="Score" label="">
											<p>
												The open flags' scores added up. Higher means more signs of one teacher
												behind several accounts.
											</p>
										</Explain>
									</span>
								</th>
								<th>Why</th>
								<th class="num">Linked</th>
								<th>Standing</th>
								<th>Last flagged</th>
								<th><span class="sr-only">Actions</span></th>
							</tr>
						</thead>
						<tbody>
							{#each rows as row (row.org)}
								<tr>
									<td class="op-cell" data-label="Organisation">
										<span class="t op-name" title={row.name}>{row.name}</span>
										<span class="s mono" title={row.org}>{row.slug ?? row.org}</span>
									</td>
									<td class="num" data-label="Score">
										<b>{row.score}</b>
										<span class="s">
											{row.open_flags}
											{row.open_flags === 1 ? 'open flag' : 'open flags'}
										</span>
									</td>
									<td data-label="Why">
										<span class="ab-why">
											<span class="op-pills">
												{#each row.kinds as kind (kind)}
													<StatusPill tone="warn" label={FLAG_KIND_LABEL[kind]} />
												{/each}
											</span>
											{#if row.signal_kinds.length > 0}
												<span class="s">
													Shares {row.signal_kinds
														.map((kind) => SIGNAL_KIND_LABEL[kind].toLowerCase())
														.join(', ')}
												</span>
											{/if}
										</span>
									</td>
									<td class="num" data-label="Linked">{row.linked_orgs}</td>
									<td data-label="Standing">
										<StatusPill
											tone={STANDING_TONE[row.standing]}
											label={STANDING_LABEL[row.standing]}
										/>
									</td>
									<td data-label="Last flagged">
										<span>
											{nzShortDate(row.last_flagged_at)}
											<span class="s">{nzTime(row.last_flagged_at)}</span>
										</span>
									</td>
									<td data-label="">
										<span class="op-acts">
											<button type="button" class="ab-open" onclick={() => (opened = row.org)}>
												Review
												<Icon name="chevron-right" size={14} />
											</button>
										</span>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
				<p class="op-foot">
					{rows.length}
					{rows.length === 1 ? 'organisation' : 'organisations'}, highest score first. Dates are New
					Zealand time.
				</p>
			{/if}
		</section>
	</div>

	{#if opened !== null}
		<AbuseSheet org={opened} onActed={acted} onClose={() => (opened = null)} />
	{/if}
</div>

<style>
	.ab-table td {
		padding-inline: var(--s-3);
	}

	.ab-why {
		display: grid;
		gap: 4px;
		justify-items: start;
		min-width: 0;
	}

	.ab-open {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		min-height: 30px;
		padding: 0 var(--s-2);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		background: transparent;
		color: var(--primary);
		font: inherit;
		font-size: 12.5px;
		font-weight: 600;
		cursor: pointer;
	}

	.ab-open:hover {
		background: var(--hover);
	}

	.ab-open:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 1px;
	}
</style>
