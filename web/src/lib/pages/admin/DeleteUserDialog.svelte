<script lang="ts">
	// Deleting a seller, in two halves and in order: the platform half first
	// (their organisation and everything in it, which the API may refuse),
	// then their sign-in account. A refusal on the first leaves both standing.
	// If the second fails after the first went through, the dialog says so and
	// a retry deletes only what is left.

	import { ApiFailure, api } from '$lib/api';
	import type { AdminUserRow } from '$lib/admin';
	import { removeIdentityUser } from '$lib/auth-client';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { lightDismiss } from '$lib/dismiss';
	import Explain from '$lib/Explain.svelte';
	import Icon from '$lib/Icon.svelte';
	import { deleteConfirmed, displayName } from './users-view';

	let {
		row,
		onClose,
		onDeleted
	}: {
		row: AdminUserRow;
		onClose: () => void;
		/** Both halves are gone. The host re-reads its lists and closes. */
		onDeleted: (email: string) => void;
	} = $props();

	type Half = 'waiting' | 'working' | 'done' | 'failed';

	let element = $state<HTMLDialogElement | null>(null);
	let typed = $state('');
	let working = $state(false);
	let refusal = $state<string | null>(null);
	// Set once the platform half is gone (or was never there), so a retry
	// after a failed identity removal does not ask the API a second time.
	let platformGone = $state(false);
	let platformHalf = $state<Half>('waiting');
	let identityHalf = $state<Half>('waiting');

	const user = $derived(row.identity);
	const platform = $derived(row.platform);
	const ready = $derived(deleteConfirmed(typed, user.email));

	$effect(() => {
		if (element !== null && !element.open) {
			element.showModal();
		}
	});

	async function remove() {
		if (!ready || working) return;
		working = true;
		refusal = null;
		try {
			if (!platformGone && platform !== null) {
				platformHalf = 'working';
				try {
					await api.adminDeleteUser(user.id);
				} catch (failure) {
					// No platform user behind this subject after all: nothing on
					// that side to delete, which is not a reason to stop.
					if (!(failure instanceof ApiFailure && failure.status === 404)) {
						platformHalf = 'failed';
						refusal =
							failure instanceof Error ? failure.message : 'The organisation was not deleted.';
						return;
					}
				}
			}
			platformGone = true;
			platformHalf = 'done';
			identityHalf = 'working';
			try {
				await removeIdentityUser(user.id);
			} catch (failure) {
				identityHalf = 'failed';
				refusal =
					(platform === null
						? ''
						: 'Their organisation is deleted, but their sign-in account is not. ') +
					(failure instanceof Error ? failure.message : 'The sign-in account was not deleted.') +
					' Try again to delete just the sign-in account.';
				return;
			}
			identityHalf = 'done';
			onDeleted(user.email);
		} finally {
			working = false;
		}
	}

	const HALF_WORDS: Record<Half, string> = {
		waiting: '',
		working: 'Deleting…',
		done: 'Deleted',
		failed: 'Not deleted'
	};
</script>

<dialog use:lightDismiss
	bind:this={element}
	class="ux-delete"
	aria-labelledby="delete-user-title"
	onclose={onClose}
	oncancel={(event) => {
		if (working) event.preventDefault();
	}}
>
	<div class="dialog-body">
		<h2 id="delete-user-title">Delete {displayName(user)}?</h2>
		<p>This can't be undone. Here is everything that goes:</p>

		<ol class="halves">
			{#if platform !== null}
				<li class={platformHalf}>
					<span class="dot" aria-hidden="true">
						{#if platformHalf === 'done'}<Icon name="check" size={13} />{:else}1{/if}
					</span>
					<span class="what">
						<span class="t">{platform.organisation.name}, and everything in it</span>
						<span class="s">Resources, files, listing links, history and settings.</span>
					</span>
					<span class="state">{HALF_WORDS[platformHalf]}</span>
				</li>
			{/if}
			<li class={identityHalf}>
				<span class="dot" aria-hidden="true">
					{#if identityHalf === 'done'}<Icon name="check" size={13} />{:else}{platform === null
							? 1
							: 2}{/if}
				</span>
				<span class="what">
					<span class="t">Their sign-in account</span>
					<span class="s">{user.email}, its passwords, passkeys and sign-ins.</span>
				</span>
				<span class="state">{HALF_WORDS[identityHalf]}</span>
			</li>
		</ol>

		<Explain title="When a delete is refused" label="When is it refused?">
			<p>
				If someone else is in their organisation, nothing is deleted: their work lives there too.
			</p>
			<p>
				If they still have a live subscription at Stripe, cancel it there first. Deleting the
				account would not stop the charges.
			</p>
			<p>An operator cannot be deleted. Turn Operator off on their account first.</p>
			<p>
				The organisation goes first. If that is refused, the sign-in account is left alone. Both
				deletions are recorded: the sign-in one in the identity audit trail.
			</p>
		</Explain>

		{#if refusal}
			<Banner tone="bad" title="Not deleted">{refusal}</Banner>
		{/if}

		<label class="confirm">
			<span>Type <strong>{user.email}</strong> to confirm</span>
			<input
				type="email"
				autocomplete="off"
				spellcheck="false"
				bind:value={typed}
				disabled={working}
			/>
		</label>

		<div class="actions">
			<Button
				tier="outline"
				disabled={working}
				reason={working ? 'Deleting.' : undefined}
				onclick={() => element?.close()}
			>
				Cancel
			</Button>
			<Button
				tier="primary"
				danger
				icon="x"
				disabled={!ready || working}
				reason={working ? 'Deleting.' : !ready ? 'Type their address to confirm.' : undefined}
				onclick={remove}
			>
				{working ? 'Deleting…' : 'Delete for good'}
			</Button>
		</div>
	</div>
</dialog>

<style>
	.halves {
		list-style: none;
		margin: 0 0 var(--s-3);
		padding: 0;
		display: grid;
		gap: var(--s-2);
	}

	.halves li {
		display: grid;
		grid-template-columns: auto 1fr auto;
		align-items: start;
		gap: var(--s-3);
		padding: var(--s-3);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		background: var(--surface);
	}

	.halves .dot {
		display: inline-grid;
		place-items: center;
		width: 22px;
		height: 22px;
		border-radius: var(--r-pill);
		background: var(--bad-soft);
		color: var(--bad-ink);
		font-size: 12px;
		font-weight: 600;
	}

	.halves li.done .dot {
		background: var(--ok-soft);
		color: var(--ok-ink);
	}

	.halves .what {
		display: grid;
		gap: 2px;
		min-width: 0;
	}

	.halves .t {
		font-weight: 600;
		overflow-wrap: anywhere;
	}

	.halves .s {
		color: var(--muted);
		font-size: 12.5px;
		overflow-wrap: anywhere;
	}

	.halves .state {
		font-size: 12px;
		color: var(--muted);
	}

	.halves li.failed .state {
		color: var(--bad-ink);
	}

	.confirm {
		display: grid;
		gap: var(--s-1);
		margin-top: var(--s-3);
		font-size: 13px;
	}

	.confirm strong {
		overflow-wrap: anywhere;
	}

	.confirm input {
		min-height: var(--control-h);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		background: var(--card);
		color: var(--text);
		padding-inline: 12px;
		font: inherit;
	}

	.confirm input:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 1px;
	}
</style>
