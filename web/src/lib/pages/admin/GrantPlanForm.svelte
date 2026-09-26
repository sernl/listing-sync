<script lang="ts">
	import { createMutation } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import { PLANS } from '$lib/generated/plans';
	import type { Plan } from '$lib/generated/vocab';
	import { toast } from '$lib/toast';

	// The two controls on the operator surface that write an entitlement: the
	// plan a tenant holds, and the moves it can spend. A component rather than
	// markup inside the organisation page because the users page sets a plan
	// on the same organisation from a dialog, and two copies of this form are
	// two forms to keep in step with what the server requires.
	let {
		org,
		current,
		onGranted
	}: {
		org: string;
		/** The plan in force, marked on its tile. */
		current?: Plan;
		/** Re-read whatever the host draws from. The server answers the whole
		 *  org detail back, but each host renders a different part of it, so
		 *  the host invalidates rather than this taking the answer apart. */
		onGranted: () => Promise<void> | void;
	} = $props();

	let chosen = $state<Plan>('subscriber');
	let expiry = $state('');
	let why = $state('');

	const trimmedWhy = $derived(why.trim());

	/** Why the grant form cannot be submitted, or null where it can. */
	const formRefusal = $derived.by(() => {
		if (trimmedWhy.length === 0) {
			return 'Enter a reason. It goes in the audit log.';
		}
		if (expiry !== '' && Number.isNaN(Date.parse(`${expiry}T00:00:00Z`))) {
			return 'Enter a valid expiry date.';
		}
		return null;
	});

	const granting = createMutation(() => ({
		mutationFn: () =>
			api.grantPlan(org, {
				plan: chosen,
				// Midnight UTC on the named day, because the server counts a
				// month in UTC and a local midnight would expire a grant on the
				// wrong day for half the world.
				...(expiry === '' ? {} : { expires_at: Date.parse(`${expiry}T00:00:00Z`) }),
				reason: trimmedWhy
			}),
		onSuccess: async () => {
			await onGranted();
			why = '';
			expiry = '';
			toast('info', 'Plan set. The account sees it on its next request.');
		},
		onError: (failure: Error) =>
			toast('error', failure instanceof ApiFailure ? failure.message : 'The plan was not set.')
	}));

	// The moves half. Signed, because an operator reaches for this to correct
	// a balance as often as to give moves away, and a correction that could
	// only add would leave a double credit standing.
	let delta = $state(0);
	let creditWhy = $state('');

	const trimmedCreditWhy = $derived(creditWhy.trim());

	/** Why the credit cannot be submitted, or null where it can. */
	const creditRefusal = $derived.by(() => {
		if (!Number.isInteger(delta) || delta === 0) {
			return 'Enter a number of moves other than zero.';
		}
		if (trimmedCreditWhy.length === 0) {
			return 'Enter a reason. The same reason twice counts as one credit.';
		}
		return null;
	});

	const crediting = createMutation(() => ({
		mutationFn: () => api.creditMoves(org, { moves: delta, reason: trimmedCreditWhy }),
		onSuccess: async () => {
			await onGranted();
			delta = 0;
			creditWhy = '';
			toast('info', 'Balance set. The account sees it on its next request.');
		},
		onError: (failure: Error) =>
			toast(
				'error',
				failure instanceof ApiFailure ? failure.message : 'The balance was not changed.'
			)
	}));

	/** What a plan gives in moves, the figure an operator is choosing on. */
	function allowance(plan: (typeof PLANS)[number]): string {
		const { moves_per_month: monthly, free_moves_lifetime: once } = plan.capabilities;
		return monthly > 0 ? `${monthly} moves a month` : `${once} moves once`;
	}
</script>

<div class="op-grant">
	<div class="flow-choice" role="radiogroup" aria-label="Plan">
		{#each PLANS as row (row.id)}
			<button
				type="button"
				role="radio"
				aria-checked={chosen === row.id}
				onclick={() => (chosen = row.id)}
			>
				{row.name}
				<span class="sub">
					{allowance(row)}{row.sold ? '' : ' · not sold'}{row.id === current ? ' · current' : ''}
				</span>
			</button>
		{/each}
	</div>
	<div class="op-grant-fields">
		<Field label="Expires" id={`grant-expiry-${org}`} hint="Empty never lapses.">
			<input id={`grant-expiry-${org}`} type="date" bind:value={expiry} />
		</Field>
		<Field label="Reason" id={`grant-reason-${org}`} required>
			<input
				id={`grant-reason-${org}`}
				type="text"
				bind:value={why}
				placeholder="Why this account gets this plan"
			/>
		</Field>
	</div>
	<div class="op-grant-foot">
		<Button
			tier="primary"
			icon="check"
			disabled={formRefusal !== null || granting.isPending}
			reason={formRefusal ?? (granting.isPending ? 'Setting the plan.' : undefined)}
			onclick={() => granting.mutate()}
		>
			{granting.isPending ? 'Applying…' : 'Apply plan'}
		</Button>
		<Explain title="What applying a plan does" label="">
			<p>
				This writes an operator grant under your operator account. It does not touch the billing
				provider, so an account that also pays keeps whichever grant is stronger.
			</p>
			<p>The expiry is midnight UTC on the day you pick.</p>
		</Explain>
	</div>

	<hr />

	<div class="op-grant-fields">
		<Field label="Moves" id={`credit-moves-${org}`} hint="Negative takes moves back.">
			<input id={`credit-moves-${org}`} type="number" step="1" bind:value={delta} />
		</Field>
		<Field label="Reason" id={`credit-reason-${org}`} required>
			<input
				id={`credit-reason-${org}`}
				type="text"
				bind:value={creditWhy}
				placeholder="Why this balance is being changed"
			/>
		</Field>
	</div>
	<div class="flow-actions">
		<Button
			icon="gift"
			disabled={creditRefusal !== null || crediting.isPending}
			reason={creditRefusal ?? (crediting.isPending ? 'Crediting moves.' : undefined)}
			onclick={() => crediting.mutate()}
		>
			{crediting.isPending ? 'Crediting…' : 'Credit moves'}
		</Button>
	</div>
</div>
