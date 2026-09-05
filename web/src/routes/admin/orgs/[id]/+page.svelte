<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { api } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { present } from '$lib/connection-status';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import StatCard from '$lib/StatCard.svelte';
	import StatusPill, { type Tone } from '$lib/StatusPill.svelte';
	import { badgeTone } from '$lib/pages/account/tones';
	import { queryKeys } from '$lib/query';
	import '$lib/pages/account/account.css';
	import '$lib/pages/admin/admin.css';

	const orgId = $derived(page.params.id ?? '');

	const detail = createQuery(() => ({
		queryKey: queryKeys.adminOrg(orgId),
		queryFn: () => api.adminOrg(orgId),
		enabled: orgId.length > 0
	}));

	const view = $derived(detail.data);
	const subscription = $derived(view?.subscription);
	const now = Date.now();

	/** Paddle's vocabulary carries no tone of its own, so the states that mean
	 *  money is not arriving are the ones marked. Anything unrecognised is
	 *  rendered plainly rather than guessed at. */
	function tone(status: string): Tone {
		if (status === 'active' || status === 'trialing') {
			return 'ok';
		}
		return status === 'past_due' || status === 'canceled' || status === 'paused' ? 'bad' : 'soon';
	}

	function instant(at: number | undefined): string {
		return at === undefined ? '—' : utcInstant(at);
	}
</script>

<div class="page">
	<PageHead
		icon="building-2"
		title={view?.org.name ?? 'Organisation'}
		description={view === undefined ? 'Reading this tenant…' : `Tenant ${view.org.org}`}
	>
		{#snippet aside()}
			<Button tier="outline" icon="arrow-left" href="/admin/orgs">All organisations</Button>
		{/snippet}
	</PageHead>

	{#if detail.isPending}
		<Panel><p class="quiet">Reading this organisation…</p></Panel>
	{:else if detail.isError}
		<Panel><p class="quiet">This organisation could not be read.</p></Panel>
	{:else if view !== undefined}
		<div class="cards">
			<StatCard icon="layout-list" label="Products" sub="not deleted">{view.org.products}</StatCard>
			<StatCard icon="refresh-cw" label="Mappings" sub="listings bound to a marketplace">
				{view.org.mappings}
			</StatCard>
			<StatCard icon="store" label="Connections" sub="marketplace links held">
				{view.org.connections}
			</StatCard>
			<StatCard icon="users" label="Users" sub={`provisioned ${agoLabel(view.org.created_at, now)}`}>
				{view.org.users}
			</StatCard>
		</div>

		<div class="band">
			<Panel
				title="Connections"
				description="The same status the seller's own page renders, derived the same way."
			>
				{#if view.connections.length === 0}
					<Placeholder
						icon="store"
						headline="This tenant has linked no marketplace"
						body="Nothing can sync for them until one is linked, which only they can do."
					/>
				{:else}
					{#each view.connections as link (link.id)}
						<div class="acct-state-row">
							<span class="who">
								<span class="t">{link.marketplace}</span>
								<span class="why">{present(link.status).explanation}</span>
							</span>
							<StatusPill tone={badgeTone(present(link.status).tone)} label={link.status} />
							<StatusPill tone="flat" label={link.state} />
							<span class="why">updated {agoLabel(link.updated_at, now)}</span>
						</div>
					{/each}
				{/if}
			</Panel>

			<Panel title="Billing" description="What Paddle last told us, passed through untranslated.">
				{#if subscription === undefined}
					<Placeholder
						icon="credit-card"
						headline="This tenant has never reached checkout"
						body="That is a different fact from a cancelled subscription, which would appear here carrying Paddle's cancelled status."
					/>
				{:else}
					<dl class="acct-detail">
						<dt>Status</dt>
						<dd><StatusPill tone={tone(subscription.status)} label={subscription.status} /></dd>
						<dt>Current period ends</dt>
						<dd>
							{#if subscription.current_period_end === undefined}
								<span class="quiet">Paddle carried no billing period</span>
							{:else}
								{instant(subscription.current_period_end)}
							{/if}
						</dd>
						<dt>As of</dt>
						<dd>{instant(subscription.occurred_at)}</dd>
					</dl>
					<p class="foot-note">
						“As of” is the instant Paddle stamped on the notification that produced this state,
						not the instant we recorded it.
					</p>
				{/if}
			</Panel>
		</div>

		<Panel title="Halts" description="Work this tenant is not allowed to perform right now.">
			{#if view.halts.length === 0}
				<Placeholder
					icon="circle-check"
					headline="Nothing is halted for this tenant"
					body="Every inventory they sell on is accepting work."
				/>
			{:else}
				{#each view.halts as halt (`${halt.inventory ?? 'tenant'}-${halt.raised_at}`)}
					<Banner
						tone="warn"
						title={`${halt.inventory === undefined ? 'The whole tenant' : halt.inventory} — raised ${agoLabel(halt.raised_at, now)} by ${halt.raised_by}`}
					>
						{halt.reason}
					</Banner>
				{/each}
			{/if}
		</Panel>
	{/if}
</div>
