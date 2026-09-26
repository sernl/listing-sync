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
		description="All accounts at a glance. This page only reads and changes nothing."
	/>

	<div class="cards">
		<StatCard icon="refresh-cw" label="Sync runs" sub="across all accounts">
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
				? 'not loaded yet'
				: `${ledger.parked_live} live, ${ledger.parked_cold} cold`}
		>
			{parked ?? '—'}
		</StatCard>
		<StatCard
			icon={failedTile.icon}
			tone={failedTile.tone}
			label="Failed items"
			sub={ledger === undefined
				? 'not loaded yet'
				: `${ledger.settled} settled in all`}
		>
			{failed ?? '—'}
		</StatCard>
	</div>

	<Panel
		title="Signups over time"
		description="The newest {SIGNUP_DAYS} days each system recorded, by UTC day."
	>
		{#snippet more()}
			<Button tier="quiet" icon="heart-pulse" href="/admin/health">Sync health</Button>
		{/snippet}

		{#if signups.isPending}
			<p class="quiet">Loading signups…</p>
		{:else if signups.isError}
			<p class="quiet">We could not load signups.</p>
		{:else if rows.length === 0}
			<Placeholder
				icon="users"
				headline="Nobody has signed up yet"
				body="Neither the identity service nor the app's user table has any rows."
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
				<span class="mono">app_user</span>, written on first sign-in. Someone who registered but
				never signed in shows in the first series only.
				{#if !identityVisible}
					This database has no identity schema, so the first series is missing, not zero.
				{/if}
			</p>
		{/if}
	</Panel>
</div>
