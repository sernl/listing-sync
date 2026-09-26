<script lang="ts">
	import { untrack } from 'svelte';
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { AuthFailure, listUserSessions, revokeUserSessions } from '$lib/auth-client';
	import Button from '$lib/Button.svelte';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';

	// One account's live sign-ins, and the one control that ends them all.
	//
	// The rows come from the identity service rather than from the API, because
	// no API role can read `auth."session"` — `tam_app` holds the audit trail
	// and nothing else in that schema. So this is the identity plane speaking
	// for itself, beside a row the platform answered.
	let {
		userId,
		email,
		onClose,
		onCount
	}: {
		userId: string;
		email: string;
		onClose: () => void;
		/** The count, back to the list, so the row an operator opened stops
		 *  saying "not read". The identity service lists sessions one account
		 *  at a time, so the column cannot be filled for fifty rows at once. */
		onCount: (userId: string, count: number) => void;
	} = $props();

	const queryClient = useQueryClient();
	const now = Date.now();
	let ending = $state(false);

	const sessions = createQuery(() => ({
		queryKey: queryKeys.identityUserSessions(userId),
		queryFn: () => listUserSessions(userId),
		retry: false
	}));

	let element = $state<HTMLDialogElement | null>(null);

	// Guarded on the element's own state: `showModal` on a dialog that is
	// already modal throws, and this effect re-runs as the element binds.
	$effect(() => {
		if (element !== null && !element.open) {
			element.showModal();
		}
	});

	// Reported out of an effect, and the call is untracked: the list the
	// operator opened this drawer from writes the count into its own state,
	// and an effect that both read that state and wrote it would re-enter
	// until Svelte stopped it.
	$effect(() => {
		const listed = sessions.data;
		if (listed !== undefined) {
			untrack(() => onCount(userId, listed.length));
		}
	});

	const rows = $derived(sessions.data ?? []);

	function when(at: string | Date | null | undefined): number | null {
		if (at === null || at === undefined) {
			return null;
		}
		const parsed = new Date(at);
		return Number.isNaN(parsed.getTime()) ? null : parsed.getTime();
	}

	async function endAll() {
		const sure = confirm(
			`Sign ${email} out everywhere?\n\n` +
				'Every browser and device they use is signed out right away, with no delay. ' +
				'They can sign in again at once.'
		);
		if (!sure) {
			return;
		}
		ending = true;
		try {
			await revokeUserSessions(userId);
			await queryClient.invalidateQueries({
				queryKey: queryKeys.identityUserSessions(userId)
			});
			onCount(userId, 0);
			toast('info', 'Signed out everywhere.');
		} catch (failure) {
			toast(
				'error',
				failure instanceof AuthFailure ? failure.message : 'Those sign-ins were not ended. Try again.'
			);
		} finally {
			ending = false;
		}
	}
</script>

<dialog bind:this={element} aria-labelledby="sessions-title" onclose={onClose}>
	<div class="dialog-body">
		<h2 id="sessions-title">Sign-ins for {email}</h2>

		{#if sessions.isPending}
			<p class="quiet">Loading sign-ins…</p>
		{:else if sessions.isError}
			<p class="refusal">
				{sessions.error instanceof AuthFailure
					? sessions.error.message
					: "We could not load this account's sign-ins."}
			</p>
		{:else if rows.length === 0}
			<p class="quiet">
				This account is not signed in anywhere. That is not a ban: a ban also blocks the next
				sign-in.
			</p>
		{:else}
			{#each rows as session (session.id)}
				{@const started = when(session.createdAt)}
				<div class="acct-state-row">
					<span class="who">
						<span class="t">{session.ipAddress || 'no address recorded'}</span>
						<span class="why" title={session.userAgent ?? undefined}>
							{session.userAgent || 'no user agent recorded'}
						</span>
					</span>
					<span class="why" title={started === null ? undefined : utcInstant(started)}>
						{started === null ? 'started at an unrecorded time' : `started ${agoLabel(started, now)}`}
					</span>
				</div>
			{/each}
		{/if}

		<p class="foot-note">
			The identity service keeps these, not the app. It records only an IP address and user
			agent, with no device name.
		</p>

		<div class="actions">
			<Button tier="outline" onclick={onClose} disabled={ending}>Close</Button>
			<Button
				danger
				disabled={ending || rows.length === 0}
				reason={rows.length === 0 ? 'This account is not signed in anywhere.' : undefined}
				onclick={endAll}
			>
				{ending ? 'Signing out…' : 'Sign out everywhere'}
			</Button>
		</div>
	</div>
</dialog>
