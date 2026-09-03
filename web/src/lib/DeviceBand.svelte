<script lang="ts">
	import type { ConnectionView, DeviceView } from '$lib/api';
	import {
		SIGN_IN_LABEL,
		bandNotice,
		deviceFootnote,
		deviceRows,
		deviceSummary,
		needingDeviceSignIn,
		schedulesRunning,
		signInStates
	} from '$lib/devices-view';
	import { agoLabel } from '$lib/elapsed';
	import Panel from '$lib/Panel.svelte';
	import { MARKETPLACE_NAME } from '$lib/platforms';

	let {
		devices,
		connections,
		pending
	}: {
		devices: DeviceView[];
		connections: ConnectionView[];
		pending: boolean;
	} = $props();

	// Read once when the panel renders rather than per row, so every age on it
	// is measured from the same instant and the list does not appear to tick.
	const now = Date.now();

	const rows = $derived(deviceRows(devices, now));
	const summary = $derived(deviceSummary(rows));
	const states = $derived(signInStates(devices, connections, now));
	const waiting = $derived(needingDeviceSignIn(states));
	const notice = $derived(bandNotice(summary, schedulesRunning(rows)));

	function standingLabel(standing: (typeof rows)[number]['standing']): string {
		switch (standing) {
			case 'checking_in':
				return 'Checking in';
			case 'quiet':
				return 'Quiet';
			case 'signed_out':
				return 'Signed out';
		}
	}

	function standingTone(standing: (typeof rows)[number]['standing']): string {
		switch (standing) {
			case 'checking_in':
				return 'ok';
			case 'quiet':
				return 'run';
			case 'signed_out':
				return 'mut';
		}
	}
</script>

<Panel
	title="Your devices and logins"
	description="TPT and Tes have no official API, so their work runs on your own machine under your own session. Your login never leaves that device."
>
	{#snippet more()}
		<a class="link" href="/settings/devices">Manage devices</a>
	{/snippet}

	{#if pending}
		<p class="quiet">Reading your devices…</p>
	{:else if summary.total === 0}
		<div class="placeholder">
			<span class="big" aria-hidden="true">▣</span>
			<b>No machine of yours is registered</b>
			<p>
				Scheduled syncing for TPT and Tes runs on a machine you own, signed in as you. Until one
				is registered, work for those marketplaces waits and nothing is sent.
			</p>
		</div>
	{:else}
		{#if notice}
			<div class="attn {notice.tone === 'warn' ? 'warn' : ''}">
				<div class="t">{notice.headline}</div>
				<p>{notice.body}</p>
				<a class="act" href="/settings/devices">Open your devices</a>
			</div>
		{:else if waiting.length > 0}
			<div class="attn warn">
				<div class="t">
					{waiting.length}
					{waiting.length === 1 ? 'marketplace needs' : 'marketplaces need'} you to sign in
				</div>
				<p>{waiting.map((entry) => MARKETPLACE_NAME[entry.marketplace]).join(', ')}.</p>
			</div>
		{/if}

		{#each states as entry (entry.marketplace)}
			<div class="row">
				<span class="what">
					<span class="t">{MARKETPLACE_NAME[entry.marketplace]}</span>
					<span class="s">{entry.line}</span>
					{#if entry.accountLabel}
						<span class="s">Shown there as {entry.accountLabel}.</span>
					{/if}
				</span>
				<span class="grow"></span>
				<span class="pill {entry.tone}">{SIGN_IN_LABEL[entry.state]}</span>
			</div>
		{/each}

		<p class="foot-note">{deviceFootnote(summary)}</p>

		{#each rows as row (row.device.id)}
			<div class="row">
				<span class="what">
					<span class="t">{row.device.name}</span>
					<span class="s">
						{row.device.os} · {row.device.arch} · version {row.device.app_version} · last seen
						{agoLabel(row.device.last_seen_at, now)}
					</span>
				</span>
				<span class="grow"></span>
				<span class="pill {standingTone(row.standing)}">{standingLabel(row.standing)}</span>
				{#if row.wipeOutstanding}
					<span class="pill bad">Wipe outstanding</span>
				{/if}
			</div>
		{/each}
	{/if}
</Panel>
