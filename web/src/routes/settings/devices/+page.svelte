<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { api, type DeviceSessionView, type DeviceView } from '$lib/api';
	import { agoLabel } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import { type BrowserSession, matchNote, merge, sessionLabel } from './merge';
	import { currentSessionToken, listBrowserSessions, revokeBrowserSession } from './sessions';

	// The registry read is keyed in `$lib/query` because the dashboard's device
	// band reads the same rows; the two identity-plane reads are keyed here,
	// because they belong to this page and nothing else invalidates them.
	const KEYS = {
		devices: queryKeys.devices,
		browserSessions: ['browser-sessions'] as const,
		currentToken: ['current-session-token'] as const
	};

	const queryClient = useQueryClient();

	const registry = createQuery(() => ({
		queryKey: KEYS.devices,
		queryFn: () => api.devices()
	}));

	const signIns = createQuery(() => ({
		queryKey: KEYS.browserSessions,
		queryFn: () => listBrowserSessions()
	}));

	const current = createQuery(() => ({
		queryKey: KEYS.currentToken,
		queryFn: () => currentSessionToken()
	}));

	const joined = $derived(
		merge(registry.data?.devices ?? [], signIns.data ?? [], current.data ?? null)
	);

	// Read once when the page loads rather than per row, so every age on it is
	// measured from the same instant and the list does not appear to tick.
	const now = Date.now();

	function osLabel(device: DeviceView): string {
		const words: Record<string, string> = {
			windows: 'Windows',
			macos: 'macOS',
			linux: 'Linux',
			android: 'Android',
			ios: 'iOS'
		};
		return words[device.os.toLowerCase()] ?? device.os;
	}

	function statusTone(status: DeviceSessionView['status']): string {
		switch (status) {
			case 'connected':
				return 'ok';
			case 'signed_out':
				return 'mut';
			case 'wiped':
				return 'bad';
		}
	}

	function statusWords(session: DeviceSessionView): string {
		switch (session.status) {
			case 'connected':
				return `signed in${session.account_label ? ` as ${session.account_label}` : ''}`;
			case 'signed_out':
				return 'disconnected on the device';
			case 'wiped':
				return 'forgotten when this device was signed out';
		}
	}

	/** Signing a machine out is two acts, and the page performs both: our
	 *  registry marks the device revoked, and the identity service ends the
	 *  browser sign-in we matched to it. Neither implies the other — the two
	 *  planes are separate by charter — so one failing must not skip the other,
	 *  and the result says which parts actually happened. */
	const signingOut = createMutation(() => ({
		mutationFn: async (input: { device: DeviceView; session: BrowserSession | null }) => {
			const endingDevice = api.revokeDevice(input.device.id).then(
				() => true,
				() => false
			);
			const endingSignIn =
				input.session === null
					? Promise.resolve(null)
					: revokeBrowserSession(input.session.token).then(
							() => true,
							() => false
						);
			const [deviceEnded, signInEnded] = await Promise.all([endingDevice, endingSignIn]);
			return { deviceEnded, signInEnded };
		},
		onSuccess: async (done: { deviceEnded: boolean; signInEnded: boolean | null }) => {
			if (!done.deviceEnded) {
				toast('error', 'The machine was not signed out. Nothing changed on our side.');
			} else if (done.signInEnded === false) {
				toast(
					'error',
					'The machine was signed out, but its browser sign-in could not be ended. Try that one again below.'
				);
			} else {
				toast('info', 'Signed out. This machine wipes its marketplace logins when it next checks in.');
			}
			await Promise.all([
				queryClient.invalidateQueries({ queryKey: KEYS.devices }),
				queryClient.invalidateQueries({ queryKey: KEYS.browserSessions })
			]);
		},
		onError: () => {
			toast('error', 'The machine was not signed out.');
		}
	}));

	const endingSignIn = createMutation(() => ({
		mutationFn: (token: string) => revokeBrowserSession(token),
		onSuccess: async () => {
			toast('info', 'That browser sign-in was ended.');
			await queryClient.invalidateQueries({ queryKey: KEYS.browserSessions });
		},
		onError: () => {
			toast('error', 'That sign-in was not ended.');
		}
	}));

	function signOut(device: DeviceView, session: BrowserSession | null) {
		const sure = confirm(
			`Sign "${device.name}" out?\n\n` +
				'It stops syncing, and it forgets its marketplace logins the next time it ' +
				'reaches us. Until then — and forever, if it never reconnects — it still ' +
				'holds those logins, because they are on that machine and never on our ' +
				'servers. Signing in again on that machine restores it.'
		);
		if (sure) {
			signingOut.mutate({ device, session });
		}
	}

	function endSignIn(session: BrowserSession, isCurrent: boolean) {
		const sure = confirm(
			isCurrent
				? 'End this sign-in? You are using it right now, so you will be signed out of this browser.'
				: 'End this browser sign-in? That browser will have to sign in again.'
		);
		if (sure) {
			endingSignIn.mutate(session.token);
		}
	}
</script>

<div class="page">
	<PageHead
		icon="▢"
		title="Your devices"
		description="The machines signed in to this account, and the marketplace logins each one holds."
	/>

	<Panel
		title="Machines"
		description="Each machine running the desktop app registers itself here. Your marketplace logins live on the machine that captured them and never on our servers, so this list is what each one reports holding — never the logins themselves."
	>
		{#if registry.isPending}
			<p class="quiet">Loading…</p>
		{:else if registry.isError}
			<p class="quiet">Your machines could not be listed.</p>
		{:else if joined.rows.length === 0}
			<div class="placeholder">
				<span class="big" aria-hidden="true">▢</span>
				<b>No machines registered yet</b>
				<p>
					Install the desktop app and sign in on it. It registers itself here on first run,
					and every marketplace you connect on it appears beside it.
				</p>
			</div>
		{:else}
			{#each joined.rows as row (row.device.id)}
				<div class="row">
					<span class="what">
						<span class="t">
							{row.device.name}
							{#if row.isCurrent}<span class="pill ok">this browser</span>{/if}
							{#if row.device.revoked_at !== null}<span class="pill bad">signed out</span>{/if}
						</span>
						<span class="s">
							{osLabel(row.device)} · {row.device.arch} · app {row.device.app_version} ·
							last seen {agoLabel(row.device.last_seen_at, now)}
						</span>
						<span class="s">{matchNote(row.confidence)}</span>
					</span>
					<span class="grow"></span>
					{#if row.device.revoked_at === null}
						<button
							type="button"
							class="btn small danger"
							disabled={signingOut.isPending &&
								signingOut.variables?.device.id === row.device.id}
							onclick={() => signOut(row.device, row.session)}
						>
							{signingOut.isPending && signingOut.variables?.device.id === row.device.id
								? 'Signing out…'
								: 'Sign out'}
						</button>
					{/if}
				</div>

				{#if row.device.wipe_outstanding}
					<p class="refusal">
						Signed out {agoLabel(row.device.revoked_at ?? now, now)}, and this machine has
						not checked in since. It still holds the marketplace logins below until it
						does. If it never reconnects, they stay on that machine until each
						marketplace expires them — there is nothing we can do from here, because we
						have never held them.
					</p>
				{/if}

				{#if row.device.sessions.length === 0}
					<p class="foot-note indented">No marketplace logins on this machine.</p>
				{:else}
					<div class="held">
						{#each row.device.sessions as session (session.marketplace)}
							<div class="row">
								<span class="t">{session.marketplace}</span>
								<span class="pill {statusTone(session.status)}">{session.status}</span>
								<span class="s">{statusWords(session)}</span>
								<span class="grow"></span>
								<span class="when">
									linked {agoLabel(session.linked_at, now)}
								</span>
							</div>
						{/each}
					</div>
				{/if}
			{/each}
		{/if}
	</Panel>

	<Panel
		title="Browser sign-ins"
		description="Where this account is signed in to the console. These are sign-ins to us, not to any marketplace; the identity service records only the address and the browser each was made from, which is why some cannot be pinned to a machine above."
	>
		{#if signIns.isPending}
			<p class="quiet">Loading…</p>
		{:else if signIns.isError}
			<p class="quiet">Your browser sign-ins could not be listed.</p>
		{:else if joined.orphans.length === 0}
			<p class="quiet">
				Every browser sign-in on this account is shown against a machine above.
			</p>
		{:else}
			{#each joined.orphans as orphan (orphan.session.token)}
				<div class="row">
					<span class="what">
						<span class="t">
							{sessionLabel(orphan.session)}
							{#if orphan.isCurrent}<span class="pill ok">this browser</span>{/if}
						</span>
						{#if orphan.session.createdAt}
							<span class="s">
								Signed in {agoLabel(new Date(orphan.session.createdAt).getTime(), now)}
							</span>
						{/if}
					</span>
					<span class="grow"></span>
					<button
						type="button"
						class="btn small danger"
						disabled={endingSignIn.isPending &&
							endingSignIn.variables === orphan.session.token}
						onclick={() => endSignIn(orphan.session, orphan.isCurrent)}
					>
						{endingSignIn.isPending && endingSignIn.variables === orphan.session.token
							? 'Ending…'
							: 'End sign-in'}
					</button>
				</div>
			{/each}
		{/if}
		<p class="foot-note">
			Ending a sign-in takes effect on the next request that browser makes; the session
			cache that would otherwise delay it is switched off for this account.
		</p>
	</Panel>
</div>

<style>
	.held {
		margin-left: 18px;
		padding-left: 12px;
		border-left: 1px solid var(--line);
	}

	.indented {
		margin-left: 18px;
	}
</style>
