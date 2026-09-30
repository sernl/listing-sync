<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { ApiFailure, api, type MailCampaignDetail, type MailDraft } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import Explain from '$lib/Explain.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatCard from '$lib/StatCard.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import { audienceSummary, statusTone } from '$lib/pages/admin/mail/mail';
	import '$lib/flow.css';
	import '$lib/pages/admin/admin.css';
	import '$lib/pages/admin/mail/mail.css';

	const queryClient = useQueryClient();
	const id = $derived(page.params.id ?? '');
	const now = Date.now();

	// Polled while anyone is still queued, so the counts move as the outbox
	// drains, and left alone once it has.
	const campaign = createQuery(() => ({
		queryKey: queryKeys.adminMailCampaign(id),
		queryFn: () => api.mailCampaign(id),
		enabled: id.length > 0,
		refetchInterval: (query) => ((query.state.data?.counts.queued ?? 0) > 0 ? 5000 : false)
	}));

	const detail = $derived(campaign.data);
	const draft = $derived(detail?.draft ?? null);
	/** A deleted email has no body to draw; the preview read is off then. */
	const NO_DRAFT: MailDraft = { subject: '', body_html: '', link_url: null, link_label: null };

	const preview = createQuery(() => {
		const shown = draft ?? NO_DRAFT;
		return {
			queryKey: queryKeys.adminMailPreview(shown),
			queryFn: () => api.previewMail(shown),
			enabled: draft !== null,
			retry: false
		};
	});

	function settled(next: MailCampaignDetail, said: string) {
		queryClient.setQueryData(queryKeys.adminMailCampaign(next.id), next);
		void queryClient.invalidateQueries({ queryKey: queryKeys.adminMailCampaigns, exact: true });
		toast('success', said);
	}

	function failed(failure: Error, fallback: string) {
		toast('error', failure instanceof ApiFailure ? failure.message : fallback);
	}

	const retrying = createMutation(() => ({
		mutationFn: () => api.retryMailCampaign(id),
		onSuccess: (next: MailCampaignDetail) => settled(next, 'The failed ones are queued again.'),
		onError: (failure: Error) => failed(failure, 'Nothing was queued again.')
	}));

	const deleting = createMutation(() => ({
		mutationFn: () => api.deleteMailCampaign(id),
		onSuccess: (next: MailCampaignDetail) => settled(next, 'Email deleted.'),
		onError: (failure: Error) => failed(failure, 'The email was not deleted.')
	}));

	function remove() {
		if (
			!confirm(
				'Delete this email?\n\nIts text and the list of who got it are removed for good. The counts stay.'
			)
		) {
			return;
		}
		deleting.mutate();
	}

	/** The email's own height, so the preview scrolls with the page. */
	function fit(event: Event) {
		const frame = event.currentTarget;
		if (!(frame instanceof HTMLIFrameElement)) {
			return;
		}
		const height = frame.contentDocument?.documentElement.scrollHeight;
		if (height !== undefined && height > 0) {
			frame.style.height = `${height}px`;
		}
	}
</script>

<div class="page flow-page">
	<PageHead
		icon="mail"
		title={detail?.subject ?? 'Email'}
		back={{ href: '/admin/mail', label: 'Mail' }}
	>
		{#snippet aside()}
			{#if detail && detail.deleted_at === null}
				<Button
					danger
					icon="trash-2"
					disabled={deleting.isPending}
					reason={deleting.isPending ? 'Deleting.' : undefined}
					onclick={remove}
				>
					Delete
				</Button>
			{/if}
		{/snippet}
	</PageHead>

	<div class="flow">
		{#if campaign.isPending}
			<p class="quiet">Loading the email…</p>
		{:else if campaign.isError || !detail}
			<Placeholder
				icon="mail"
				headline="We could not load this email"
				body="It may not exist. Go back to Mail and pick it from the list."
			/>
		{:else}
			<section class="flow-section" aria-label="Summary">
				<dl class="mail-facts">
					<div>
						<dt>To</dt>
						<dd>{audienceSummary(detail.audience)}</dd>
					</div>
					<div>
						<dt>Sent</dt>
						<dd>
							<span title={utcInstant(detail.created_at)}>{agoLabel(detail.created_at, now)}</span>
							by {detail.created_by_label}
						</dd>
					</div>
					{#if detail.deleted_at !== null}
						<div>
							<dt>Deleted</dt>
							<dd title={utcInstant(detail.deleted_at)}>{agoLabel(detail.deleted_at, now)}</dd>
						</div>
					{/if}
				</dl>
			</section>

			<div class="op-stats">
				<StatCard icon="users" label="Recipients">{detail.counts.total}</StatCard>
				<StatCard icon="circle-check" tone="ok" label="Sent">{detail.counts.sent}</StatCard>
				<StatCard icon="circle-x" tone={detail.counts.failed > 0 ? 'bad' : ''} label="Failed">
					{detail.counts.failed}
				</StatCard>
				<StatCard icon="clock" tone={detail.counts.queued > 0 ? 'warn' : ''} label="Queued">
					{detail.counts.queued}
				</StatCard>
				<StatCard icon="minus" label="Skipped" sub="No verified address or no sign-in">
					{detail.counts.skipped}
				</StatCard>
			</div>

			{#if detail.counts.failed > 0 && detail.deleted_at === null}
				<Banner tone="bad">
					{detail.counts.failed === 1
						? '1 email failed to send.'
						: `${detail.counts.failed} emails failed to send.`}
					{#snippet action()}
						<Button
							icon="refresh-cw"
							disabled={retrying.isPending}
							reason={retrying.isPending ? 'Queuing them again.' : undefined}
							onclick={() => retrying.mutate()}
						>
							Try failed again
						</Button>
					{/snippet}
				</Banner>
			{/if}

			{#if detail.deleted_at !== null}
				<p class="mail-deleted-note">
					This email was deleted, so its text and the list of who got it are gone. The counts stay.
				</p>
			{:else}
				<section class="flow-section" aria-labelledby="mail-body-title">
					<div class="flow-section-head">
						<h2 id="mail-body-title">The email</h2>
					</div>
					{#if preview.data}
						<iframe
							class="mail-frame"
							title="Email preview"
							srcdoc={preview.data.html}
							sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox"
							onload={fit}
						></iframe>
					{:else if preview.isError}
						<p class="quiet">We could not draw the email.</p>
					{:else}
						<p class="quiet">Drawing the email…</p>
					{/if}
				</section>

				<section class="flow-section" aria-labelledby="mail-recipients-title">
					<div class="flow-section-head">
						<h2 id="mail-recipients-title">Who it went to</h2>
						<Explain title="Reading the list" label="">
							<p>
								Each row is one seller. Queued ones are still waiting to go; the page updates every
								few seconds until none are left.
							</p>
							<p>
								Skipped ones had no verified address or no sign-in. The Resend id finds the message
								in Resend’s logs.
							</p>
						</Explain>
					</div>
					<div class="flow-table-wrap op-table">
						<table class="flow-table">
							<thead>
								<tr>
									<th>Organisation</th>
									<th>Status</th>
									<th class="num">Tries</th>
									<th>Resend id</th>
									<th>Updated</th>
								</tr>
							</thead>
							<tbody>
								{#each detail.recipients as row (row.user)}
									<tr>
										<td class="op-cell" data-label="Organisation">
											<span class="t">{row.org_name}</span>
											<span class="s">{row.plan}</span>
										</td>
										<td class="op-cell" data-label="Status">
											<StatusPill tone={statusTone(row.status)} label={row.status} />
											{#if row.error !== null}
												<span class="s" title={row.error}>{row.error}</span>
											{/if}
										</td>
										<td class="num" data-label="Tries">{row.attempts}</td>
										<td class="op-cell" data-label="Resend id">
											<span class="t mono">{row.provider_id ?? '—'}</span>
										</td>
										<td class="op-cell" data-label="Updated">
											<span class="t" title={utcInstant(row.updated_at)}>
												{agoLabel(row.updated_at, now)}
											</span>
										</td>
									</tr>
								{:else}
									<tr><td colspan="5" class="quiet">Nobody yet.</td></tr>
								{/each}
							</tbody>
						</table>
					</div>
				</section>
			{/if}
		{/if}
	</div>
</div>
