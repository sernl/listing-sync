<script lang="ts">
	import { api, type ConnectionView, type SyncRequestHead } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { agoLabel } from '$lib/elapsed';
	import Field from '$lib/Field.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { SHORT_NAME, platformTitle } from '$lib/platforms';
	import StatusPill from '$lib/StatusPill.svelte';
	import {
		FILES_STAY_ON_YOUR_COMPUTER,
		siteChoiceQuestion,
		siteLabel
	} from '$lib/sync-request';
	import type { InventoryId } from '$lib/generated/vocab';
	import {
		CONNECTIONS_UNREAD,
		CONNECT_HREF,
		HANDOFF_LABEL,
		IMPORTS_UNREAD,
		IMPORT_IS_A_MIGRATION,
		NO_IMPORT_YET,
		WHAT_AN_IMPORT_IS,
		deviceLine,
		handoffBlocked,
		importCards,
		importRows,
		migrationHref,
		notConnected,
		standingBadge,
		type ImportCard
	} from './import-view';
	import './import.css';

	// Null until the list has actually been read. An empty array is a seller
	// with no marketplace, which is a claim; not having read the list is not.
	let connections = $state<ConnectionView[] | null>(null);
	let connectionsUnread = $state(false);

	let requests = $state<SyncRequestHead[]>([]);
	let requestsLoaded = $state(false);
	let requestsUnread = $state(false);

	let chosen = $state<Record<string, InventoryId>>({});

	const cards = $derived(importCards(connections));
	const rows = $derived(importRows(requests, (inventory) => SHORT_NAME[inventory]));

	$effect(() => {
		void api
			.connections()
			.then((view) => {
				connections = view.connections;
				connectionsUnread = false;
			})
			.catch(() => {
				connections = null;
				connectionsUnread = true;
			});
		void loadRequests();
	});

	async function loadRequests() {
		try {
			requests = (await api.syncRequests()).requests;
			requestsUnread = false;
		} catch {
			requests = [];
			requestsUnread = true;
		}
		requestsLoaded = true;
	}

	function siteOf(card: ImportCard): InventoryId | null {
		return chosen[card.marketplace] ?? card.preselected;
	}
</script>

<div class="page">
	<PageHead
		icon="download"
		title="Import"
		description="Bring a shop across: your own device reads it, signed in as you, and sends us what it finds."
	/>

	<p class="import-lead">{WHAT_AN_IMPORT_IS}</p>
	<p class="import-lead">{IMPORT_IS_A_MIGRATION}</p>

	{#if connectionsUnread}
		<Banner tone="bad" title="Your marketplaces could not be read">{CONNECTIONS_UNREAD}</Banner>
	{/if}

	<div class="import-cards">
		{#each cards as card (card.marketplace)}
			{@const blocked = handoffBlocked(card)}
			{@const site = siteOf(card)}
			<section class="import-card">
				<div class="head">
					<h2>{card.name}</h2>
					<span class="badges">
						{#if card.unreadable === null}
							{@const badge = standingBadge(card)}
							<StatusPill tone={badge.tone} label={badge.label} />
						{/if}
						<StatusPill tone="flat" label="On your device" />
					</span>
				</div>

				{#if card.unreadable !== null}
					<p class="why">{card.unreadable}</p>
				{:else}
					{#if card.sites.length > 1}
						<Field
							label={siteChoiceQuestion(card.sites)}
							id={`import-site-${card.marketplace}`}
						>
							<select
								id={`import-site-${card.marketplace}`}
								value={site}
								onchange={(event) =>
									(chosen[card.marketplace] = event.currentTarget.value as InventoryId)}
							>
								{#each card.sites as option (option)}
									<option value={option} title={platformTitle(option)}>
										{SHORT_NAME[option]} — {siteLabel(option)}
									</option>
								{/each}
							</select>
						</Field>
					{/if}

					<p class="quiet">{deviceLine(card)}</p>

					{#if card.standing === 'absent'}
						<Banner tone="bad">
							{notConnected(card)}
							{#snippet action()}
								<Button href={CONNECT_HREF}>Go to Marketplaces</Button>
							{/snippet}
						</Banner>
					{/if}

					<div class="actions">
						{#if blocked === null && site !== null}
							<Button tier="primary" icon="arrow-right-left" href={migrationHref(site)}>
								{HANDOFF_LABEL}
							</Button>
						{:else}
							<!-- Disabled rather than absent, so the card still shows what
							     the seller would do here and says what stands in the way. -->
							<Button
								tier="primary"
								icon="arrow-right-left"
								disabled
								reason={blocked ?? undefined}
							>
								{HANDOFF_LABEL}
							</Button>
						{/if}
					</div>
				{/if}
			</section>
		{/each}
	</div>

	<p class="foot-note">{FILES_STAY_ON_YOUR_COMPUTER}</p>

	<Panel
		title="Your imports"
		description="Every shop you have brought across, newest first."
	>
		{#if requestsUnread}
			<p class="quiet">{IMPORTS_UNREAD}</p>
		{:else if !requestsLoaded}
			<p class="quiet">Loading…</p>
		{:else if rows.length === 0}
			<Placeholder
				icon="download"
				headline={NO_IMPORT_YET}
				body="Choose a marketplace above to start one."
			/>
		{:else}
			{#each rows as row (row.request)}
				<!-- The badge and the line must stay inside one link. Two of the stage
				     labels are "Nothing to import" and "Nothing imported", which a
				     reader who gets no colour tells apart only by the sentence that
				     follows; announced as one link, the pairing resolves for them as
				     the tone resolves it for everyone else. Splitting the badge out of
				     this anchor, or showing the label without its line anywhere, makes
				     those two labels indistinguishable and needs different words
				     upstream rather than a change here. -->
				<a class="import-row" href={`/sync/requests/${row.request}`}>
					<span class="mark"><StatusPill tone={row.tone} label={row.label} /></span>
					<span class="who">
						<span class="t">{row.title}</span>
						<span class="w">{row.line}</span>
					</span>
					<span class="at">{agoLabel(row.created_at, Date.now())}</span>
				</a>
			{/each}
		{/if}
	</Panel>
</div>
