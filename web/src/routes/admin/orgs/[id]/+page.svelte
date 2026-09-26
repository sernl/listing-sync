<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { ApiFailure, api, type GrantRowView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { present } from '$lib/connection-status';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import Explain from '$lib/Explain.svelte';
	import FlowStep from '$lib/FlowStep.svelte';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import GrantPlanForm from '$lib/pages/admin/GrantPlanForm.svelte';
	import SessionsDialog from '$lib/pages/admin/SessionsDialog.svelte';
	import { planName, planTone } from '$lib/pages/admin/admin-view';
	import PageHead from '$lib/PageHead.svelte';
	import { SHORT_NAME } from '$lib/platforms';
	import StatusPill, { type Tone } from '$lib/StatusPill.svelte';
	import Stepper, { type StepMark } from '$lib/Stepper.svelte';
	import { toast } from '$lib/toast';
	import { badgeTone } from '$lib/pages/account/tones';
	import { queryKeys } from '$lib/query';
	import '$lib/flow.css';
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

	function instant(at: number | undefined | null): string {
		return at === undefined || at === null ? '—' : utcInstant(at);
	}

	const queryClient = useQueryClient();

	/** Whether a grant is the one in force rather than expired or revoked.
	 *  Only a live grant is worth a Revoke control. */
	function live(grant: GrantRowView): boolean {
		return grant.revoked_at === null && (grant.expires_at === null || grant.expires_at > now);
	}

	function refusalOf(failure: Error, fallback: string): string {
		return failure instanceof ApiFailure ? failure.message : fallback;
	}

	async function reload() {
		await queryClient.invalidateQueries({ queryKey: queryKeys.adminOrg(orgId) });
	}

	const revoking = createMutation(() => ({
		mutationFn: (grant: string) => api.revokeGrant(orgId, grant),
		onSuccess: async () => {
			await reload();
			toast('info', 'Grant revoked. The account falls back to its next strongest grant.');
		},
		onError: (failure: Error) => toast('error', refusalOf(failure, 'The grant was not revoked.'))
	}));

	function revoke(grant: GrantRowView) {
		const asked = confirm(
			`Revoke the ${planName(grant.plan)} grant on this account?\n\n` +
				'It falls back to its next strongest unexpired grant. For most accounts that means Free.'
		);
		if (asked) {
			revoking.mutate(grant.id);
		}
	}

	// The people of this organisation, from the users answer: whose sign-ins
	// (browsers and devices) the operator can open and end.
	const users = createQuery(() => ({
		queryKey: queryKeys.adminUsers,
		queryFn: () => api.adminUsers()
	}));
	const members = $derived(
		(users.data?.users ?? []).filter((user) => user.organisation.org === orgId)
	);
	let sessionsFor = $state<{ id: string; email: string } | null>(null);
	/** Sign-in counts by identity account, filled as rows are opened. */
	let counted = $state<Record<string, number>>({});
	// Declared once, not inline: the dialog reports from an effect, and a new
	// function each render would re-trigger it without end.
	function noteSessionCount(userId: string, count: number) {
		counted = { ...counted, [userId]: count };
	}

	const steps = $derived<StepMark[]>([
		{ id: 'plan', label: 'Plan and moves' },
		{ id: 'shops', label: 'Shops and devices' },
		{ id: 'activity', label: 'Activity' }
	]);
</script>

<div class="page flow-page">
	<PageHead
		icon="building-2"
		title={view?.org.name ?? 'Organisation'}
		description={view === undefined
			? 'Loading…'
			: `${view.org.slug ?? view.org.org} · joined ${agoLabel(view.org.created_at, now)}`}
		back={{ href: '/admin/orgs', label: 'All organisations' }}
	/>

	{#if detail.isPending}
		<p class="quiet">Loading this organisation…</p>
	{:else if detail.isError}
		<p class="quiet">We could not load this organisation.</p>
	{:else if view !== undefined}
		<div class="flow">
			<p class="op-facts-line">
				<span><b>{view.org.products}</b> resources</span>
				<span><b>{view.org.mappings}</b> listings</span>
				<span><b>{view.org.connections}</b> shops</span>
				<span><b>{view.org.users}</b> {view.org.users === 1 ? 'user' : 'users'}</span>
				<span class="mono">{view.org.org}</span>
			</p>

			{#each view.halts as halt (`${halt.inventory ?? 'tenant'}-${halt.raised_at}`)}
				<Banner
					tone="warn"
					title={`${halt.inventory === undefined ? 'The whole account' : SHORT_NAME[halt.inventory]} is halted · ${agoLabel(halt.raised_at, now)} by ${halt.raised_by}`}
				>
					{halt.reason}
				</Banner>
			{/each}

			<Stepper {steps} label="Organisation" />

			<FlowStep
				n={1}
				id="plan"
				title="Plan and moves"
				hint="Pick a plan and apply it, or credit moves."
				summary={`${planName(view.plan.plan)} · ${view.moves.available} moves`}
			>
				{#snippet aside()}
					<Explain title="Billing, as the provider last said" label="Billing">
						{#if subscription === undefined}
							<p>
								This account has never reached checkout. That is not the same as cancelled: a
								cancelled subscription would show its cancelled status here.
							</p>
						{:else}
							<p>Status: {subscription.status}.</p>
							<p>
								Current period ends:
								{subscription.current_period_end === undefined
									? 'no billing period carried'
									: instant(subscription.current_period_end)}.
							</p>
							<p>
								As of {instant(subscription.occurred_at)}, the time the provider put on its
								notification, not the time we recorded it.
							</p>
						{/if}
					</Explain>
				{/snippet}

				<dl class="op-facts">
					<div>
						<dt>Plan</dt>
						<dd><StatusPill tone={planTone(view.plan.plan)} label={planName(view.plan.plan)} /></dd>
					</div>
					<div>
						<dt>Moves</dt>
						<dd>
							<b>{view.moves.available}</b>
							{#if view.moves.expiring_soonest !== undefined}
								<span class="quiet">· first expiry {instant(view.moves.expiring_soonest)}</span>
							{/if}
						</dd>
					</div>
					<div>
						<dt>Billing</dt>
						<dd>
							{#if subscription === undefined}
								<span class="quiet">never checked out</span>
							{:else}
								<StatusPill tone={tone(subscription.status)} label={subscription.status} />
							{/if}
						</dd>
					</div>
					<div>
						<dt>Set by</dt>
						<dd>
							{#if view.plan.granted_by === null}
								<span class="quiet">nobody, so Free</span>
							{:else}
								{view.plan.granted_by} · {instant(view.plan.granted_at)}
							{/if}
						</dd>
					</div>
					<div>
						<dt>Expires</dt>
						<dd>{view.plan.expires_at === null ? 'never' : instant(view.plan.expires_at)}</dd>
					</div>
					{#if view.plan.source_ref !== null}
						<div>
							<dt>Source</dt>
							<dd class="mono">{view.plan.source_ref}</dd>
						</div>
					{/if}
				</dl>

				<GrantPlanForm org={orgId} current={view.plan.plan} onGranted={reload} />
			</FlowStep>

			<FlowStep
				n={2}
				id="shops"
				title="Shops and devices"
				hint="Marketplaces connected, and who signs in from where."
				summary={`${view.connections.length} shops · ${members.length} people`}
			>
				{#if view.connections.length === 0}
					<p class="quiet">No marketplace connected. Only the seller can connect one.</p>
				{:else}
					<div class="flow-table-wrap op-table">
						<table class="flow-table">
							<thead>
								<tr>
									<th>Shop</th>
									<th>
										<span class="op-th">
											Status
											<Explain title="Shop status" label="">
												{#each view.connections as link (link.id)}
													<p><b>{link.status}</b>: {present(link.status).explanation}</p>
												{/each}
												<p>The second pill is the stored link state, shown as is.</p>
											</Explain>
										</span>
									</th>
									<th class="num">Updated</th>
								</tr>
							</thead>
							<tbody>
								{#each view.connections as link (link.id)}
									<tr>
										<td data-label="Shop">
											<span class="op-mark">
												<MarketplaceMark marketplace={link.marketplace} size={18} />
												{link.marketplace}
											</span>
										</td>
										<td data-label="Status">
											<span class="op-pills">
												<StatusPill tone={badgeTone(present(link.status).tone)} label={link.status} />
												<StatusPill tone="flat" label={link.state} />
											</span>
										</td>
										<td class="num" data-label="Updated" title={utcInstant(link.updated_at)}>
											{agoLabel(link.updated_at, now)}
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				{/if}

				<h3 class="flow-label">People and their sign-ins</h3>
				{#if users.isPending}
					<p class="quiet">Loading people…</p>
				{:else if users.isError}
					<p class="flow-warn">People could not be loaded.</p>
				{:else if members.length === 0}
					<p class="quiet">No app user belongs to this organisation.</p>
				{:else}
					<div class="flow-table-wrap op-table">
						<table class="flow-table">
							<thead>
								<tr>
									<th>Person</th>
									<th class="num">Last sign-in</th>
									<th class="num">Sign-ins</th>
								</tr>
							</thead>
							<tbody>
								{#each members as member (member.user)}
									{@const subject = member.auth_subject}
									<tr>
										<td class="op-cell" data-label="Person">
											<span class="t" title={member.email}>{member.email}</span>
											<span class="s">joined {agoLabel(member.created_at, now)}</span>
										</td>
										<td
											class="num"
											data-label="Last sign-in"
											title={member.last_sign_in_at === undefined
												? undefined
												: utcInstant(member.last_sign_in_at)}
										>
											{member.last_sign_in_at === undefined
												? '—'
												: agoLabel(member.last_sign_in_at, now)}
										</td>
										<td class="num" data-label="Sign-ins">
											<Button
												small
												icon="laptop"
												disabled={subject === undefined}
												reason={subject === undefined
													? 'This user has no identity account to read sign-ins from.'
													: undefined}
												onclick={() =>
													subject !== undefined && (sessionsFor = { id: subject, email: member.email })}
											>
												{subject !== undefined && counted[subject] !== undefined
													? `${counted[subject]} open`
													: 'Sign-ins'}
											</Button>
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				{/if}
			</FlowStep>

			<FlowStep
				n={3}
				id="activity"
				title="Activity"
				hint="Every plan grant, newest first."
				summary={`${view.grants.length} grants`}
			>
				{#if view.grants.length === 0}
					<p class="quiet">No grant has ever been set for this account.</p>
				{:else}
					<div class="flow-table-wrap op-table">
						<table class="flow-table">
							<thead>
								<tr>
									<th>Grant</th>
									<th>By</th>
									<th>Why</th>
									<th>State</th>
									<th></th>
								</tr>
							</thead>
							<tbody>
								{#each view.grants as grant (grant.id)}
									<tr>
										<td class="op-cell" data-label="Grant">
											<span class="t">
												{planName(grant.plan)}{grant.rung === null ? '' : ` · up to ${grant.rung}`}
											</span>
											<span class="s" title={instant(grant.granted_at)}>
												{agoLabel(grant.granted_at, now)}
											</span>
										</td>
										<td data-label="By">{grant.granted_by}</td>
										<td class="op-cell" data-label="Why">
											<span class="t" title={grant.reason ?? undefined}>{grant.reason ?? '—'}</span>
											{#if grant.source_ref !== null}
												<span class="s mono" title={grant.source_ref}>{grant.source_ref}</span>
											{/if}
										</td>
										<td data-label="State">
											{#if grant.revoked_at !== null}
												<StatusPill tone="bad" label="revoked" />
											{:else if grant.expires_at !== null && grant.expires_at <= now}
												<StatusPill tone="soon" label="expired" />
											{:else}
												<StatusPill tone="ok" label="in force" />
											{/if}
										</td>
										<td class="num" data-label="Action">
											{#if live(grant)}
												<Button
													small
													danger
													disabled={revoking.isPending}
													reason={revoking.isPending ? 'A revoke is in progress.' : undefined}
													onclick={() => revoke(grant)}
												>
													Revoke
												</Button>
											{/if}
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				{/if}
			</FlowStep>
		</div>
	{/if}
</div>

{#if sessionsFor !== null}
	{@const target = sessionsFor}
	<SessionsDialog
		userId={target.id}
		email={target.email}
		onClose={() => (sessionsFor = null)}
		onCount={noteSessionCount}
	/>
{/if}
