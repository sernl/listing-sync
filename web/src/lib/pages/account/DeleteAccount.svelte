<script lang="ts">
	// The danger zone at the foot of Account settings: one sentence on what
	// deleting does, the detail behind Explain, and a sheet that asks for the
	// account's name (or DELETE) and the proof the identity service wants —
	// the password where the account has one, otherwise a sign-in from the
	// last few minutes. The rules live in `delete-account.ts`.
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import type { QueryClient } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import { type DeletionProof, deletionProof, signOutEverywhere } from '$lib/auth-client';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import Icon from '$lib/Icon.svelte';
	import { setLedgerScope } from '$lib/ledger';
	import Panel from '$lib/Panel.svelte';
	import Sheet, { type SheetHandle } from '$lib/Sheet.svelte';
	import {
		CONFIRM_WORD,
		GOODBYE_URL,
		REASON_MAX_CHARS,
		REAUTH_URL,
		confirmationAccepted,
		deletionBody,
		deletionFailure,
		needsFreshSignIn
	} from '$lib/pages/account/delete-account';

	let {
		slug,
		queryClient
	}: {
		/** The account's name in the console's address, or null before one is
		 *  chosen, when only DELETE confirms. */
		slug: string | null;
		queryClient: QueryClient;
	} = $props();

	let open = $state(false);
	let sheet = $state<SheetHandle>();
	let proof = $state<DeletionProof | null>(null);
	let checkFailed = $state<string | null>(null);
	let confirm = $state('');
	let password = $state('');
	let reason = $state('');
	let refusal = $state<string | null>(null);
	let refusalField = $state<'password' | 'confirm' | null>(null);
	let sending = $state(false);
	let leaving = $state(false);

	const confirmed = $derived(confirmationAccepted(confirm, slug));
	const freshNeeded = $derived(proof !== null && needsFreshSignIn(proof, Date.now()));
	const blocked = $derived(
		!confirmed
			? `Type ${slug ?? CONFIRM_WORD} to confirm.`
			: proof?.hasPassword === true && password.length === 0
				? 'Enter your password.'
				: sending
					? 'Deleting your account.'
					: null
	);

	async function openSheet() {
		open = true;
		proof = null;
		checkFailed = null;
		confirm = '';
		password = '';
		reason = '';
		refusal = null;
		refusalField = null;
		try {
			proof = await deletionProof();
		} catch {
			checkFailed = 'We could not check how you sign in. Close this and try again.';
		}
	}

	// Back from signing in again: `/settings?delete=1` opens the sheet, and the
	// marker leaves the address so a reload does not open it a second time.
	onMount(() => {
		if (page.url.searchParams.get('delete') === '1') {
			void goto('/settings', { replaceState: true, noScroll: true, keepFocus: true });
			void openSheet();
		}
	});

	/** Ends this sign-in and goes to the sign-in page, which brings the seller
	 *  back here with the sheet open. */
	async function signInAgain() {
		leaving = true;
		setLedgerScope(null);
		await signOutEverywhere();
		queryClient.clear();
		window.location.assign(REAUTH_URL);
	}

	async function remove(event: SubmitEvent) {
		event.preventDefault();
		if (blocked !== null) return;
		sending = true;
		refusal = null;
		refusalField = null;
		try {
			await api.deleteAccount(deletionBody(confirm, password, reason));
		} catch (failure) {
			sending = false;
			const outcome = deletionFailure(failure);
			if (outcome.kind === 'reauthenticate') {
				await signInAgain();
				return;
			}
			refusal = outcome.message;
			refusalField = outcome.field;
			return;
		}
		// The account is gone and the API's cookie with it; the identity
		// service's sign-out finds nothing to end, which is the point.
		leaving = true;
		setLedgerScope(null);
		await signOutEverywhere();
		queryClient.clear();
		window.location.assign(GOODBYE_URL);
	}

	function keepOpenWhileSending(event: Event) {
		if (sending || leaving) event.preventDefault();
	}
</script>

<Panel id="delete-account" title="Delete your account">
	<div class="acct-danger">
		<p>
			This deletes your Teachouse account, your catalogue and your device registrations. Your
			listings on TPT and Tes stay as they are. This cannot be undone.
		</p>
		<div class="acct-danger-actions">
			<Explain title="What deleting your account does" label="Explain" tone="warn">
				<p>
					If you have a plan, it is cancelled at Stripe straight away and you won't be charged again.
					There is no refund beyond what our Terms say.
				</p>
				<p>Any packs of moves you bought are forfeited.</p>
				<p>
					The files on your devices are untouched. Teachouse stops syncing them, and you can uninstall
					the app whenever you like.
				</p>
				<p>
					We send one last email to say it is done. If you didn't ask for it, reply to that email
					straight away.
				</p>
			</Explain>
			<Button tier="outline" danger icon="trash-2" onclick={() => void openSheet()}>
				Delete your account
			</Button>
		</div>
	</div>
</Panel>

{#if open}
	<Sheet
		labelledby="delete-account-title"
		onClose={() => (open = false)}
		oncancel={keepOpenWhileSending}
		bind:handle={sheet}
	>
		<header class="da-head">
			<h2 id="delete-account-title">Delete your account</h2>
			<button
				type="button"
				class="da-close"
				aria-label="Close"
				disabled={sending || leaving}
				onclick={() => sheet?.close()}
			>
				<Icon name="x" />
			</button>
		</header>

		<div class="da-body">
			{#if checkFailed !== null}
				<Banner tone="bad">{checkFailed}</Banner>
			{:else if proof === null}
				<p class="quiet">Checking how you sign in…</p>
			{:else if freshNeeded}
				<p>To keep your account safe, sign in again first. You'll come straight back here.</p>
				<div class="da-foot">
					<Button onclick={() => sheet?.close()}>Cancel</Button>
					<Button
						tier="primary"
						icon="shield-check"
						disabled={leaving}
						reason={leaving ? 'Signing you out.' : undefined}
						onclick={() => void signInAgain()}
					>
						{leaving ? 'Signing out…' : 'Sign in again'}
					</Button>
				</div>
			{:else}
				<form class="da-form" novalidate onsubmit={remove}>
					<Banner tone="warn">
						This cannot be undone. Your plan stops now and your catalogue is deleted.
					</Banner>

					<Field
						label={slug === null ? `Type ${CONFIRM_WORD} to confirm` : `Type ${slug} to confirm`}
						id="delete-confirm"
						required
						hint={slug === null ? undefined : `Or type ${CONFIRM_WORD}.`}
					>
						<input
							id="delete-confirm"
							type="text"
							autocomplete="off"
							autocapitalize="none"
							spellcheck="false"
							aria-invalid={refusalField === 'confirm'}
							bind:value={confirm}
						/>
					</Field>

					{#if proof.hasPassword}
						<Field label="Your password" id="delete-password" required>
							<input
								id="delete-password"
								type="password"
								autocomplete="current-password"
								aria-invalid={refusalField === 'password'}
								bind:value={password}
							/>
						</Field>
					{/if}

					<Field
						label="Why are you leaving?"
						id="delete-reason"
						hint="Optional. Only we read this."
					>
						<textarea id="delete-reason" rows="3" maxlength={REASON_MAX_CHARS} bind:value={reason}
						></textarea>
					</Field>

					{#if refusal !== null}
						<Banner tone="bad">{refusal}</Banner>
					{/if}

					<div class="da-foot">
						<Button disabled={sending || leaving} onclick={() => sheet?.close()}>Cancel</Button>
						<Button
							tier="primary"
							danger
							type="submit"
							icon="trash-2"
							disabled={blocked !== null || leaving}
							reason={blocked ?? undefined}
						>
							{sending || leaving ? 'Deleting…' : 'Delete my account'}
						</Button>
					</div>
				</form>
			{/if}
		</div>
	</Sheet>
{/if}

<style>
	.da-head {
		display: grid;
		grid-template-columns: 1fr auto;
		gap: var(--s-3);
		align-items: center;
		padding: var(--s-4) var(--s-4) var(--s-3);
		border-bottom: 1px solid var(--line);
	}

	.da-head h2 {
		margin: 0;
		font-size: 16px;
		font-weight: 600;
	}

	.da-close {
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

	.da-close:hover {
		background: var(--hover);
		color: var(--ink);
	}

	.da-body {
		flex: 1;
		overflow-y: auto;
		padding: var(--s-4) var(--s-4) calc(var(--s-4) + env(safe-area-inset-bottom));
		display: grid;
		align-content: start;
		gap: var(--s-4);
	}

	.da-body p {
		margin: 0;
		font-size: 14px;
		line-height: 1.5;
	}

	.da-form {
		display: grid;
		gap: var(--s-4);
	}

	.da-form input,
	.da-form textarea {
		width: 100%;
		box-sizing: border-box;
		min-height: var(--control-h);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		background: var(--card);
		padding: 8px 11px;
		font: inherit;
		font-size: 14px;
		color: var(--ink);
	}

	.da-form :global([aria-invalid='true']) {
		border-color: var(--bad);
	}

	.da-foot {
		display: flex;
		justify-content: flex-end;
		flex-wrap: wrap;
		gap: var(--s-2);
		padding-top: var(--s-3);
		border-top: 1px solid var(--line);
	}
</style>
