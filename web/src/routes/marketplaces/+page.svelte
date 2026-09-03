<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { api, type ConnectionView, type DeviceSessionView, type DeviceView } from '$lib/api';
	import {
		SIGN_IN_LABEL,
		deviceFootnote,
		deviceRows,
		deviceSummary,
		marketplaceRows,
		needingAttention,
		type MarketplaceRow
	} from '$lib/devices-view';
	import { agoLabel } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { MARKETPLACE_NAME } from '$lib/platforms';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import { type BrowserSession, matchNote, merge, sessionLabel } from './merge';
	import { currentSessionToken, listBrowserSessions, revokeBrowserSession } from './sessions';

	// The two reads that decide a marketplace row are keyed in `$lib/query`
	// because the dashboard's band and its attention panel read the same rows:
	// a revoke here has to move them too. The two identity-plane reads are
	// keyed locally, because nothing outside this page invalidates them.
	const KEYS = {
		browserSessions: ['browser-sessions'] as const,
		currentToken: ['current-session-token'] as const
	};

	const queryClient = useQueryClient();

	const registry = createQuery(() => ({
		queryKey: queryKeys.devices,
		queryFn: () => api.devices()
	}));

	const linked = createQuery(() => ({
		queryKey: queryKeys.connections,
		queryFn: () => api.connections()
	}));

	const signIns = createQuery(() => ({
		queryKey: KEYS.browserSessions,
		queryFn: () => listBrowserSessions()
	}));

	const current = createQuery(() => ({
		queryKey: KEYS.currentToken,
		queryFn: () => currentSessionToken()
	}));

	// Read once when the page renders rather than per row, so every age on it is
	// measured from the same instant and the list does not appear to tick.
	const now = Date.now();

	const devices = $derived(registry.data?.devices ?? []);
	const connections = $derived(linked.data?.connections ?? []);
	const rows = $derived(marketplaceRows(devices, connections, now));
	const waiting = $derived(needingAttention(rows));
	const summary = $derived(deviceSummary(deviceRows(devices, now)));
	const joined = $derived(merge(devices, signIns.data ?? [], current.data ?? null));

	const BRANCH_LABEL: Record<MarketplaceRow['transport'], string> = {
		OfficialApi: 'Runs on our infrastructure',
		SellerDevice: 'Runs on your device'
	};

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

	const revoking = createMutation(() => ({
		mutationFn: (connection: string) => api.revoke(connection),
		onSuccess: async (done: { elapsed_ms: number }) => {
			toast('info', `Revoked in ${done.elapsed_ms}ms.`);
			await queryClient.invalidateQueries({ queryKey: queryKeys.connections });
		},
		onError: () => {
			toast('error', 'The revoke did not complete.');
		}
	}));

	function revoke(connection: ConnectionView) {
		const sure = confirm(
			'Revoke this connection? The stored credential is destroyed and every ' +
				'queued item for it pauses until you re-link.'
		);
		if (sure) {
			revoking.mutate(connection.id);
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
				queryClient.invalidateQueries({ queryKey: queryKeys.devices }),
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
		icon="⚲"
		title="Marketplaces"
		description="Every marketplace once, where its login lives, and whether it can be written to right now."
	/>

	{#if waiting.length > 0}
		<div class="attn warn">
			<div class="t">
				{waiting.length}
				{waiting.length === 1 ? 'marketplace needs' : 'marketplaces need'} you
			</div>
			<p>{waiting.map((row) => MARKETPLACE_NAME[row.marketplace]).join(', ')}.</p>
		</div>
	{/if}

	<Panel
		title="Marketplaces"
		description="Which branch a marketplace runs on is a recorded fact, not an implementation detail, so it is stated here. A marketplace with a sanctioned API is worked from our own infrastructure under a token it issues us. One without is worked from your own machine under your own session, and that login never reaches our servers."
	>
		{#if registry.isPending || linked.isPending}
			<p class="quiet">Loading…</p>
		{:else if registry.isError || linked.isError}
			<p class="quiet">Your marketplaces could not be read.</p>
		{:else}
			{#each rows as row (row.marketplace)}
				<div class="row">
					<span class="what">
						<span class="t">
							{MARKETPLACE_NAME[row.marketplace]}
							<span class="pill mut">{BRANCH_LABEL[row.transport]}</span>
						</span>
						<span class="s">{row.signIn.line}</span>
						{#if row.signIn.accountLabel}
							<span class="s">Shown there as {row.signIn.accountLabel}.</span>
						{/if}
						{#if row.quiet && row.signIn.device}
							<span class="s">
								{row.signIn.device.name} last checked in {agoLabel(
									row.signIn.device.last_seen_at,
									now
								)}, and a machine checks in every hour.
							</span>
						{/if}
						{#each row.wipeOutstandingOn as device (device.id)}
							<span class="s">
								{device.name} was signed out and has not been heard from since, so this
								login may still be on it.
							</span>
						{/each}
						{#if row.connection}
							<span class="s">Linked {agoLabel(row.connection.created_at, now)}.</span>
						{/if}
					</span>
					<span class="grow"></span>
					<span class="pill {row.signIn.tone}">{SIGN_IN_LABEL[row.signIn.state]}</span>
					{#if row.transport === 'OfficialApi'}
						<button
							class="btn small"
							type="button"
							disabled
							title="No endpoint links a marketplace connection yet."
						>
							{row.connection === null ? 'Link' : 'Re-link'}
						</button>
						{#if row.connection && row.connection.state !== 'revoked'}
							<button
								class="btn small danger"
								type="button"
								disabled={revoking.isPending && revoking.variables === row.connection.id}
								onclick={() => row.connection && revoke(row.connection)}
							>
								{revoking.isPending && revoking.variables === row.connection.id
									? 'Revoking…'
									: 'Revoke'}
							</button>
						{/if}
					{:else}
						<a class="btn small" href="#machines">Your machines</a>
					{/if}
				</div>
			{/each}
			<p class="foot-note">
				Linking is disabled because nothing on the server does it yet: the API serves the
				connection list and a revoke, and no endpoint establishes one. Until there is, a
				marketplace on our own infrastructure is linked by us rather than from here. Signing
				in to a marketplace that runs on your device is done on that machine, in its own app,
				which is the only place its session can exist.
			</p>
		{/if}
	</Panel>

	<div id="machines">
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
							disabled={signingOut.isPending && signingOut.variables?.device.id === row.device.id}
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
								<span class="t">{MARKETPLACE_NAME[session.marketplace]}</span>
								<span class="pill {statusTone(session.status)}">{session.status}</span>
								<span class="s">{statusWords(session)}</span>
								<span class="grow"></span>
								<span class="when">linked {agoLabel(session.linked_at, now)}</span>
							</div>
						{/each}
					</div>
				{/if}
			{/each}
			<p class="foot-note">{deviceFootnote(summary)}</p>
		{/if}
		</Panel>
	</div>

	<Panel
		title="Browser sign-ins"
		description="Where this account is signed in to the console. These are sign-ins to us, not to any marketplace; the identity service records only the address and the browser each was made from, which is why some cannot be pinned to a machine above."
	>
		{#if signIns.isPending}
			<p class="quiet">Loading…</p>
		{:else if signIns.isError}
			<p class="quiet">Your browser sign-ins could not be listed.</p>
		{:else if joined.orphans.length === 0}
			<p class="quiet">Every browser sign-in on this account is shown against a machine above.</p>
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
						disabled={endingSignIn.isPending && endingSignIn.variables === orphan.session.token}
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
			Ending a sign-in takes effect on the next request that browser makes; the session cache
			that would otherwise delay it is switched off for this account.
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
