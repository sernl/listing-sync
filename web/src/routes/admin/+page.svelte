<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { SIGNUP_DAYS, barWidth, dayLabel, signupPeak, signupSeries } from '$lib/admin';
	import { api } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import StatCard from '$lib/StatCard.svelte';
	import { queryKeys } from '$lib/query';
	import { sumOf, tileFor } from '$lib/pages/admin/ledger-tile';
	import '$lib/pages/admin/admin.css';

	const signups = createQuery(() => ({
		queryKey: queryKeys.adminSignups,
		queryFn: () => api.adminSignups()
	}));
	const health = createQuery(() => ({
		queryKey: queryKeys.operator,
		queryFn: () => api.adminSyncHealth()
	}));

	const rows = $derived(signups.data === undefined ? [] : signupSeries(signups.data, SIGNUP_DAYS));
	const peak = $derived(signupPeak(rows));
	const identityVisible = $derived(signups.data?.identity !== undefined);
	const ledger = $derived(health.data);

	// Undefined rather than zero where the ledger has not been read, so the
	// glyph, the tint and the figure on each card all read one source and
	// cannot come to disagree about whether there is a figure at all.
	const inFlight = $derived(
		sumOf(ledger?.queued, ledger?.leased, ledger?.running, ledger?.verifying)
	);
	const parked = $derived(sumOf(ledger?.parked_live, ledger?.parked_cold));
	const failed = $derived(ledger?.failed);

	const parkedTile = $derived(
		tileFor(parked, { icon: 'pause', tone: 'warn' }, { icon: 'circle-check', tone: 'ok' })
	);
	const failedTile = $derived(
		tileFor(failed, { icon: 'circle-x', tone: 'bad' }, { icon: 'circle-check', tone: 'ok' })
	);
</script>

<div class="page">
	<PageHead
		icon="layout-dashboard"
		title="Platform overview"
		description="Every tenant at once. Read-only: nothing on this page writes anything."
	/>

	<div class="cards">
		<StatCard icon="refresh-cw" label="Sync runs" sub="across every tenant">
			{ledger?.jobs ?? '—'}
		</StatCard>
		<StatCard
			icon="layout-list"
			tag="in flight"
			label="Items moving"
			sub="queued, leased, running or verifying"
		>
			{inFlight ?? '—'}
		</StatCard>
		<StatCard
			icon={parkedTile.icon}
			tone={parkedTile.tone}
			label="Parked items"
			sub={ledger === undefined
				? 'the ledger has not been read'
				: `${ledger.parked_live} live, ${ledger.parked_cold} cold`}
		>
			{parked ?? '—'}
		</StatCard>
		<StatCard
			icon={failedTile.icon}
			tone={failedTile.tone}
			label="Failed items"
			sub={ledger === undefined
				? 'the ledger has not been read'
				: `${ledger.settled} settled in all`}
		>
			{failed ?? '—'}
		</StatCard>
	</div>

	<Panel
		title="Signups over time"
		description="The newest {SIGNUP_DAYS} days each plane recorded, by UTC day."
	>
		{#snippet more()}
			<Button tier="quiet" icon="heart-pulse" href="/admin/health">Sync health</Button>
		{/snippet}

		{#if signups.isPending}
			<p class="quiet">Reading the signup series…</p>
		{:else if signups.isError}
			<p class="quiet">The signup series could not be read.</p>
		{:else if rows.length === 0}
			<Placeholder
				icon="users"
				headline="Nobody has signed up yet"
				body="Neither the identity plane nor the platform's own user table holds a row."
			/>
		{:else}
			<div class="op-plane-key">
				<span><i class="second" aria-hidden="true"></i> Identity signups</span>
				<span><i aria-hidden="true"></i> Provisioned platform users</span>
			</div>

			{#each rows as row (row.day)}
				<div class="op-day-row">
					<div class="day">{dayLabel(row.day)}</div>
					<div class="pair">
						<div class="op-plane-bar second">
							{#if row.identity === null}
								<span class="absent">identity trail not visible from this database</span>
							{:else}
								<span class="fill" style={`width:${barWidth(row.identity, peak)}%`}></span>
								<span class="n">{row.identity}</span>
							{/if}
						</div>
						<div class="op-plane-bar">
							<span class="fill" style={`width:${barWidth(row.provisioned, peak)}%`}></span>
							<span class="n">{row.provisioned}</span>
						</div>
					</div>
				</div>
			{/each}

			<p class="foot-note">
				Identity signups are <span class="mono">user_signed_up</span> events in the identity
				service's audit trail. Provisioned platform users are rows in
				<span class="mono">app_user</span>, which the session exchange writes the first time a
				subject signs in — so a human who registered and never came back appears in the first
				series and not the second.
				{#if !identityVisible}
					This database carries no identity schema, so the first series is absent rather than
					zero.
				{/if}
			</p>
		{/if}
	</Panel>
</div>
