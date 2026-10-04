<script lang="ts">
	// One flagged organisation, opened from the Abuse list: why it was flagged,
	// what it shares, the organisations sharing it, and what an operator does
	// about it. A linked organisation opens in the same sheet, with Back to
	// return. A ban asks for a reason and one more press.

	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import Icon from '$lib/Icon.svelte';
	import { queryKeys } from '$lib/query';
	import Sheet, { type SheetHandle } from '$lib/Sheet.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import {
		ACTIONS,
		ACTION_DONE,
		ACTION_LABEL,
		FLAG_KIND_LABEL,
		REASON_MAX,
		RESOLUTION_LABEL,
		SIGNAL_KIND_LABEL,
		STANDING_LABEL,
		STANDING_TONE,
		actionBody,
		actionFlag,
		actionIsCurrent,
		banSentence,
		reasonProblem,
		type AbuseOrgView,
		type OperatorAction
	} from './abuse';
	import { nzShortDate, nzTime } from './payments';

	let {
		org,
		onActed,
		onClose
	}: {
		/** The organisation the sheet opens on. */
		org: string;
		/** Redraw the list from an action's answer. */
		onActed: (view: AbuseOrgView) => Promise<void>;
		onClose: () => void;
	} = $props();

	const queryClient = useQueryClient();
	let sheet = $state<SheetHandle>();

	// svelte-ignore state_referenced_locally
	let trail = $state<string[]>([org]);
	const current = $derived(trail[trail.length - 1]);

	let reason = $state('');
	let confirming = $state(false);
	let tried = $state(false);
	let refusal = $state<string | null>(null);

	const cluster = createQuery(() => ({
		queryKey: queryKeys.adminAbuseOrg(current),
		queryFn: () => api.adminAbuseOrg(current)
	}));

	const flag = $derived(cluster.data ? actionFlag(cluster.data) : null);
	const banProblem = $derived(tried ? reasonProblem('ban', reason) : null);

	const acting = createMutation(() => ({
		mutationFn: (input: { flag: string; action: OperatorAction }) =>
			api.adminAbuseAct(input.flag, input.action, actionBody(reason)),
		onSuccess: async (view: AbuseOrgView, input) => {
			queryClient.setQueryData(queryKeys.adminAbuseOrg(view.org), view);
			await onActed(view);
			reason = '';
			tried = false;
			confirming = false;
			refusal = null;
			toast('success', ACTION_DONE[input.action]);
		},
		onError: (failure: Error) => {
			confirming = false;
			refusal = failure instanceof ApiFailure ? failure.message : 'Nothing was changed. Try again.';
		}
	}));

	function act(action: OperatorAction) {
		if (flag === null) return;
		const problem = reasonProblem(action, reason);
		if (action === 'ban' && !confirming) {
			tried = true;
			if (problem === null) confirming = true;
			return;
		}
		if (problem !== null) {
			refusal = problem;
			return;
		}
		acting.mutate({ flag, action });
	}

	function open(next: string) {
		trail = [...trail, next];
		reason = '';
		tried = false;
		confirming = false;
		refusal = null;
	}

	function back() {
		trail = trail.slice(0, -1);
		reason = '';
		tried = false;
		confirming = false;
		refusal = null;
	}

	function keepOpenWhileSending(event: Event) {
		if (acting.isPending) event.preventDefault();
	}
</script>

<Sheet labelledby="abuse-sheet-title" {onClose} oncancel={keepOpenWhileSending} bind:handle={sheet}>
	<header class="ab-head">
		{#if trail.length > 1}
			<button type="button" class="ab-icon" aria-label="Back" onclick={back}>
				<Icon name="arrow-left" />
			</button>
		{/if}
		<div class="ab-who">
			<h2 id="abuse-sheet-title">{cluster.data?.name ?? 'Organisation'}</h2>
			{#if cluster.data}
				<span class="ab-sub">
					<span class="mono">{cluster.data.slug ?? cluster.data.org}</span> · joined
					{nzShortDate(cluster.data.created_at)}
				</span>
				<span>
					<StatusPill
						tone={STANDING_TONE[cluster.data.standing]}
						label={STANDING_LABEL[cluster.data.standing]}
					/>
				</span>
			{/if}
		</div>
		<button
			type="button"
			class="ab-icon"
			aria-label="Close"
			disabled={acting.isPending}
			onclick={() => sheet?.close()}
		>
			<Icon name="x" />
		</button>
	</header>

	<div class="ab-body">
		{#if cluster.isPending}
			<p class="quiet">Reading this organisation…</p>
		{:else if cluster.isError}
			<p class="quiet">This organisation could not be read. Close this and open it again.</p>
		{:else}
			{@const view = cluster.data}
			<section class="ab-block" aria-labelledby="ab-act-title">
				<h3 id="ab-act-title">
					What to do
					<Explain title="What each action does" label="">
						<p>Every action closes all of this organisation's open flags.</p>
						<p>
							Dismiss clears them with no change. Warn emails everyone in the organisation. Limit
							marks the account limited without emailing anyone.
						</p>
						<p>
							Ban signs everyone out, suspends the account, emails them, and refuses its email
							address, shops, devices and cards on any new account for 24 months.
						</p>
						<p>Dismiss, Warn or Limit on a banned organisation lifts the ban.</p>
					</Explain>
				</h3>

				{#if refusal}
					<Banner tone="bad" title="Nothing was changed">{refusal}</Banner>
				{/if}

				{#if flag === null}
					<p class="quiet">This organisation has never been flagged, so there is nothing to act on.</p>
				{:else if confirming}
					<p class="ab-confirm">{banSentence(view.name)}</p>
					<Explain title="What a ban does" label="Explain">
						<p>
							Everyone in {view.name} is signed out at once, and signing in again shows them that the
							account is suspended with our email address.
						</p>
						<p>
							For 24 months, a new sign-up with the same email address, shop, device or card is
							refused. Dismiss, Warn or Limit from here lifts all of it.
						</p>
						<p>They get an email saying so.</p>
					</Explain>
					<p class="quiet">Reason: “{reason.trim()}”</p>
					<div class="ab-acts">
						<Button
							disabled={acting.isPending}
							reason={acting.isPending ? 'Banning.' : undefined}
							onclick={() => (confirming = false)}
						>
							Back
						</Button>
						<Button
							tier="primary"
							danger
							icon="lock"
							disabled={acting.isPending}
							reason={acting.isPending ? 'Banning.' : undefined}
							onclick={() => act('ban')}
						>
							{acting.isPending ? 'Banning…' : `Ban ${view.name}`}
						</Button>
					</div>
				{:else}
					<Field
						label="Reason"
						id="abuse-reason"
						hint="Needed for a ban. Kept with the flags; they never see it."
					>
						<textarea
							id="abuse-reason"
							rows="3"
							maxlength={REASON_MAX}
							bind:value={reason}
							aria-invalid={banProblem !== null}
						></textarea>
						{#if banProblem !== null}
							<span class="ab-error" role="alert">
								<Icon name="circle-alert" size={13} />
								{banProblem}
							</span>
						{/if}
					</Field>
					<div class="ab-acts">
						{#each ACTIONS as action (action)}
							{@const same = actionIsCurrent(action, view)}
							{@const banning = action === 'ban'}
							<Button
								small
								tier={banning ? 'outline' : undefined}
								danger={banning}
								icon={banning ? 'lock' : undefined}
								disabled={acting.isPending || same}
								reason={same
									? 'It already stands there.'
									: acting.isPending
										? 'Saving.'
										: undefined}
								onclick={() => act(action)}
							>
								{acting.isPending && acting.variables?.action === action
									? 'Saving…'
									: ACTION_LABEL[action]}
							</Button>
						{/each}
					</div>
				{/if}
			</section>

			<section class="ab-block" aria-labelledby="ab-flags-title">
				<h3 id="ab-flags-title">Flags</h3>
				{#if view.flags.length === 0}
					<p class="quiet">No flags.</p>
				{:else}
					<ul class="ab-list">
						{#each view.flags as item (item.id)}
							<li class="ab-item">
								<span class="ab-line">
									<StatusPill
										tone={item.resolved_at === null ? 'warn' : 'soon'}
										label={FLAG_KIND_LABEL[item.kind]}
									/>
									<b>Score {item.score}</b>
									<span class="quiet">
										{nzShortDate(item.created_at)}, {nzTime(item.created_at)}
									</span>
								</span>
								<span>{item.reason}</span>
								{#if item.resolved_at !== null}
									<span class="quiet">
										{RESOLUTION_LABEL[item.action]}
										{nzShortDate(item.resolved_at)}{item.action_reason
											? `: “${item.action_reason}”`
											: ''}
									</span>
								{:else}
									<span class="quiet">Open</span>
								{/if}
								{#if item.mail_error !== null}
									<span class="ab-unsent">The email did not go: {item.mail_error}</span>
								{:else if item.mail_sent_at !== null}
									<span class="quiet">Emailed {nzShortDate(item.mail_sent_at)}</span>
								{/if}
							</li>
						{/each}
					</ul>
				{/if}
			</section>

			<section class="ab-block" aria-labelledby="ab-signals-title">
				<h3 id="ab-signals-title">
					Signals
					<Explain title="What a signal is" label="">
						<p>
							A shop, device, card, network or browser this organisation was seen with. We keep a
							scrambled fingerprint, never the value itself, so only the first few characters show.
						</p>
					</Explain>
				</h3>
				{#if view.signals.length === 0}
					<p class="quiet">No signals recorded.</p>
				{:else}
					<ul class="ab-list">
						{#each view.signals as signal (`${signal.kind}-${signal.value}`)}
							<li class="ab-item">
								<span class="ab-line">
									<b>{SIGNAL_KIND_LABEL[signal.kind]}</b>
									<span class="mono ab-hash">{signal.value}</span>
									{#if signal.shared_with > 0}
										<StatusPill
											tone="warn"
											label={`Shared with ${signal.shared_with} ${signal.shared_with === 1 ? 'other' : 'others'}`}
										/>
									{/if}
								</span>
								<span class="quiet">
									First seen {nzShortDate(signal.first_seen)} · last {nzShortDate(signal.last_seen)}
								</span>
							</li>
						{/each}
					</ul>
				{/if}
			</section>

			<section class="ab-block" aria-labelledby="ab-linked-title">
				<h3 id="ab-linked-title">Linked organisations</h3>
				{#if view.linked.length === 0}
					<p class="quiet">No other organisation shares a signal with this one.</p>
				{:else}
					<ul class="ab-list">
						{#each view.linked as linked (linked.org)}
							<li class="ab-item">
								<span class="ab-line">
									<button type="button" class="ab-link" onclick={() => open(linked.org)}>
										{linked.name}
									</button>
									<StatusPill
										tone={STANDING_TONE[linked.standing]}
										label={STANDING_LABEL[linked.standing]}
									/>
								</span>
								<span class="quiet">
									Same {linked.shared.map((kind) => SIGNAL_KIND_LABEL[kind].toLowerCase()).join(', ')}
								</span>
							</li>
						{/each}
					</ul>
				{/if}
			</section>
		{/if}
	</div>
</Sheet>

<style>
	.ab-head {
		display: flex;
		gap: var(--s-3);
		align-items: flex-start;
		padding: var(--s-4) var(--s-4) var(--s-3);
		border-bottom: 1px solid var(--line);
	}

	.ab-who {
		flex: 1;
		display: grid;
		gap: 4px;
		min-width: 0;
	}

	.ab-who h2 {
		margin: 0;
		font-size: 16px;
		font-weight: 600;
		overflow-wrap: anywhere;
	}

	.ab-sub {
		color: var(--muted);
		font-size: 13px;
		overflow-wrap: anywhere;
	}

	.ab-icon {
		flex: none;
		display: inline-grid;
		place-items: center;
		width: 34px;
		height: 34px;
		border: 0;
		border-radius: var(--r-pill);
		background: transparent;
		color: var(--muted);
		cursor: pointer;
	}

	.ab-icon:hover {
		background: var(--hover);
		color: var(--ink);
	}

	.ab-body {
		flex: 1;
		overflow-y: auto;
		padding: var(--s-2) var(--s-4) calc(var(--s-4) + env(safe-area-inset-bottom));
		display: grid;
		align-content: start;
		gap: var(--s-2);
	}

	.ab-block {
		padding: var(--s-3) 0;
		display: grid;
		gap: var(--s-2);
		min-width: 0;
	}

	.ab-block + .ab-block {
		border-top: 1px solid var(--line);
	}

	.ab-block h3 {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		margin: 0;
		font-size: 12px;
		font-weight: 600;
		letter-spacing: 0.04em;
		text-transform: uppercase;
		color: var(--muted);
	}

	.ab-block textarea {
		width: 100%;
		box-sizing: border-box;
		min-height: var(--control-h);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		background: var(--card);
		padding: 8px 11px;
		font: inherit;
		font-size: 13px;
		color: var(--ink);
	}

	.ab-block textarea[aria-invalid='true'] {
		border-color: var(--bad);
	}

	.ab-error {
		display: inline-flex;
		align-items: center;
		gap: var(--s-1);
		font-size: 12.5px;
		font-weight: 500;
		color: var(--bad-ink);
	}

	.ab-acts {
		display: flex;
		flex-wrap: wrap;
		gap: var(--s-2);
	}

	.ab-confirm {
		margin: 0;
		font-size: 16px;
		font-weight: 600;
		line-height: 1.4;
	}

	.ab-list {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: var(--s-2);
	}

	.ab-item {
		display: grid;
		gap: 2px;
		min-width: 0;
		padding: var(--s-3);
		background: var(--card);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		font-size: 13.5px;
		overflow-wrap: anywhere;
	}

	.ab-line {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--s-1) var(--s-2);
	}

	.ab-hash {
		font-size: 12px;
		color: var(--muted);
	}

	.ab-unsent {
		font-size: 12.5px;
		font-weight: 600;
		color: var(--warn-ink);
	}

	.ab-link {
		border: 0;
		padding: 0;
		background: none;
		color: var(--additive);
		font: inherit;
		font-weight: 600;
		text-decoration: underline;
		text-underline-offset: 2px;
		cursor: pointer;
	}

	.quiet {
		margin: 0;
		color: var(--muted);
		font-size: 13px;
	}
</style>
