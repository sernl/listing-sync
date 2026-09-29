<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { audienceSummary, countsLine } from '$lib/pages/admin/mail/mail';
	import '$lib/flow.css';
	import '$lib/pages/admin/admin.css';
	import '$lib/pages/admin/mail/mail.css';

	const campaigns = createQuery(() => ({
		queryKey: queryKeys.adminMailCampaigns,
		queryFn: () => api.mailCampaigns().then((view) => view.campaigns)
	}));

	const rows = $derived(campaigns.data ?? []);
	const now = Date.now();
</script>

<div class="page flow-page">
	<PageHead icon="mail" title="Mail" description="Emails to sellers about what is new.">
		{#snippet aside()}
			{#if rows.length > 0}
				<Button tier="primary" icon="plus" href="/admin/mail/new">New email</Button>
			{/if}
		{/snippet}
	</PageHead>

	<div class="flow">
		{#if campaigns.isPending}
			<p class="quiet">Loading emails…</p>
		{:else if campaigns.isError}
			<Placeholder
				icon="mail"
				headline="We could not load the emails"
				body="Try reloading the page."
			/>
		{:else if rows.length === 0}
			<Placeholder
				icon="mail"
				headline="No emails yet"
				body="Write one to tell sellers what is new."
			>
				{#snippet actions()}
					<Button tier="primary" icon="plus" href="/admin/mail/new">New email</Button>
				{/snippet}
			</Placeholder>
		{:else}
			<ul class="mail-list" aria-label="Emails sent">
				{#each rows as campaign (campaign.id)}
					{@const deleted = campaign.deleted_at !== null}
					<li class="mail-row" class:deleted>
						<div class="mail-row-top">
							{#if deleted}
								<StatusPill tone="soon" label="Deleted" />
							{:else if campaign.counts.queued > 0}
								<StatusPill tone="run" label="Sending" />
							{:else if campaign.counts.failed > 0}
								<StatusPill tone="bad" label="Some failed" />
							{:else}
								<StatusPill tone="ok" label="Sent" />
							{/if}
							<h2 class="mail-row-subject">
								<a href={`/admin/mail/${campaign.id}`}>{campaign.subject}</a>
							</h2>
						</div>
						<span class="s">{audienceSummary(campaign.audience)}</span>
						<span class="s">{countsLine(campaign.counts)}</span>
						<span class="s">
							<span title={utcInstant(campaign.created_at)}>
								{agoLabel(campaign.created_at, now)}
							</span>
							by {campaign.created_by_label}
						</span>
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</div>
