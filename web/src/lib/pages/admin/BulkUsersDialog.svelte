<script lang="ts">
	// The bulk bar's confirm: names the action and the count, runs it one
	// account at a time with a running tally, then lists what happened to
	// each, failures first and in the server's own words.

	import type { IdentityUser } from '$lib/auth-client';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { lightDismiss } from '$lib/dismiss';
	import Icon from '$lib/Icon.svelte';
	import {
		BULK_WORDS,
		countWords,
		perform,
		refusedBeforehand,
		runBulk,
		type BulkAction,
		type BulkResult
	} from './users-bulk';

	let {
		action,
		count,
		selfId,
		resolve,
		onClose,
		onFinished
	}: {
		action: BulkAction;
		/** How many accounts are selected, as the bulk bar said. */
		count: number;
		selfId: string | null;
		/** The selected accounts, read when the operator confirms. */
		resolve: () => Promise<IdentityUser[]>;
		onClose: () => void;
		/** The run ended; the host re-reads its lists. */
		onFinished: () => Promise<void>;
	} = $props();

	type Stage = 'confirm' | 'running' | 'done';

	let element = $state<HTMLDialogElement | null>(null);
	let stage = $state<Stage>('confirm');
	let typed = $state('');
	let reason = $state('');
	let total = $state(0);
	let results = $state<BulkResult[]>([]);
	let refusal = $state<string | null>(null);

	const words = $derived(BULK_WORDS[action]);
	const ready = $derived(action !== 'delete' || typed.trim() === String(count));
	const failures = $derived(results.filter((result) => result.failure !== null));
	const worked = $derived(results.filter((result) => result.failure === null));

	$effect(() => {
		if (element !== null && !element.open) {
			element.showModal();
		}
	});

	async function run(event: SubmitEvent) {
		event.preventDefault();
		if (!ready || stage !== 'confirm') return;
		stage = 'running';
		refusal = null;
		let chosen: IdentityUser[];
		try {
			chosen = await resolve();
		} catch (failure) {
			stage = 'confirm';
			refusal = failure instanceof Error ? failure.message : 'The accounts could not be read.';
			return;
		}
		total = chosen.length;
		results = [];
		await runBulk(
			chosen,
			(target) => perform(action, target.id, reason),
			(target) => refusedBeforehand(action, target, selfId),
			(result) => (results = [...results, result])
		);
		stage = 'done';
		await onFinished();
	}
</script>

<dialog
	bind:this={element}
	class="ux-bulk-dialog"
	aria-labelledby="bulk-users-title"
	onclose={onClose}
	oncancel={(event) => {
		if (stage === 'running') event.preventDefault();
	}}
	use:lightDismiss
>
	<form class="dialog-body" onsubmit={run}>
		<h2 id="bulk-users-title">{countWords(words.question, count)}</h2>
		<p>{words.effect}</p>

		{#if refusal}
			<Banner tone="bad" title="Nothing was changed">{refusal}</Banner>
		{/if}

		{#if stage === 'confirm'}
			{#if action === 'ban'}
				<label class="field">
					<span>Why? Shown on each account later.</span>
					<input bind:value={reason} placeholder="Reason" autocomplete="off" />
				</label>
			{/if}
			{#if action === 'delete'}
				<label class="field">
					<span>Type <strong>{count}</strong> to confirm.</span>
					<input bind:value={typed} inputmode="numeric" autocomplete="off" />
				</label>
			{/if}
		{:else}
			<p class="tally" role="status" aria-live="polite">
				{#if stage === 'running'}
					Working… {results.length} of {total} done.
				{:else}
					{worked.length}
					{words.done}.
					{#if failures.length > 0}
						{failures.length} {failures.length === 1 ? 'was' : 'were'} not:
					{/if}
				{/if}
			</p>
			{#if failures.length > 0}
				<ul class="results" aria-label="Not changed">
					{#each failures as result (result.id)}
						<li class="bad">
							<Icon name="circle-x" size={15} />
							<span class="who">{result.email}</span>
							<span class="why">{result.failure}</span>
						</li>
					{/each}
				</ul>
			{/if}
			{#if worked.length > 0}
				<details open={stage === 'done' && failures.length === 0 && worked.length <= 8}>
					<summary>{worked.length} {words.done}</summary>
					<ul class="results">
						{#each worked as result (result.id)}
							<li class="ok">
								<Icon name="circle-check" size={15} />
								<span class="who">{result.email}</span>
							</li>
						{/each}
					</ul>
				</details>
			{/if}
		{/if}

		<div class="actions">
			{#if stage === 'done'}
				<Button tier="primary" onclick={() => element?.close()}>Done</Button>
			{:else}
				<Button
					tier="quiet"
					disabled={stage === 'running'}
					reason={stage === 'running' ? 'Working.' : undefined}
					onclick={() => element?.close()}>Cancel</Button
				>
				<Button
					tier="primary"
					danger={words.danger}
					type="submit"
					icon={words.icon}
					disabled={!ready || stage === 'running'}
					reason={stage === 'running' ? 'Working.' : !ready ? `Type ${count} first.` : undefined}
				>
					{countWords(words.confirm, count)}
				</Button>
			{/if}
		</div>
	</form>
</dialog>

<style>
	.field {
		display: grid;
		gap: var(--s-1);
		font-size: 13px;
	}

	.field input {
		min-height: var(--control-h);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		background: var(--card);
		color: var(--text);
		padding-inline: 12px;
		font: inherit;
	}

	.tally {
		margin: 0 0 var(--s-2);
		font-size: 13px;
	}

	.results {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: var(--s-1);
		font-size: 13px;
		max-height: 40vh;
		overflow-y: auto;
	}

	.results li {
		display: grid;
		grid-template-columns: auto minmax(0, 1fr);
		gap: 0 var(--s-2);
		align-items: start;
	}

	.results li.bad {
		color: var(--bad-ink);
	}

	.results li.ok {
		color: var(--muted);
	}

	.who {
		overflow-wrap: anywhere;
		color: var(--ink);
	}

	.why {
		grid-column: 2;
		color: var(--muted);
	}

	details summary {
		cursor: pointer;
		font-size: 13px;
		color: var(--muted);
		margin-block: var(--s-2);
	}

	.actions {
		display: flex;
		gap: var(--s-2);
	}
</style>
