<script lang="ts">
	import {
		createMutation,
		createQuery,
		keepPreviousData,
		useQueryClient
	} from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import {
		ApiFailure,
		api,
		type MailAudience,
		type MailCampaignDetail,
		type MailDraft,
		type MailSegment
	} from '$lib/api';
	import { identity } from '$lib/auth-client';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import FlowStep from '$lib/FlowStep.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import RichTextEditor from '$lib/RichTextEditor.svelte';
	import Stepper from '$lib/Stepper.svelte';
	import Toggle from '$lib/Toggle.svelte';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import {
		MAIL_VARIABLES,
		SEGMENTS,
		SUBJECT_MAX,
		audienceSummary,
		countLine,
		mailDraft,
		sanitiseMailHtml,
		sellers,
		validDraft
	} from '$lib/pages/admin/mail/mail';
	import '$lib/flow.css';
	import '$lib/pages/admin/admin.css';
	import '$lib/pages/admin/mail/mail.css';

	const queryClient = useQueryClient();

	let subject = $state('');
	let body = $state('');
	let linkUrl = $state('');
	let linkLabel = $state('');
	let segment = $state<MailSegment>('all');
	let excludeOperators = $state(true);
	let verifiedOnly = $state(true);
	let editor = $state<ReturnType<typeof RichTextEditor> | null>(null);

	const draft = $derived(mailDraft({ subject, body, linkUrl, linkLabel }));
	const problem = $derived(validDraft(draft));
	const audience = $derived<MailAudience>({
		segment,
		exclude_operators: excludeOperators,
		verified_only: verifiedOnly
	});

	// The preview follows the draft once typing pauses, rather than a render
	// per keystroke.
	let previewed = $state.raw<MailDraft>(
		mailDraft({ subject: '', body: '', linkUrl: '', linkLabel: '' })
	);
	$effect(() => {
		const next = draft;
		const timer = setTimeout(() => (previewed = next), 400);
		return () => clearTimeout(timer);
	});

	const preview = createQuery(() => {
		const shown = previewed;
		return {
			queryKey: queryKeys.adminMailPreview(shown),
			queryFn: () => api.previewMail(shown),
			enabled: validDraft(shown) === null,
			placeholderData: keepPreviousData,
			retry: false
		};
	});

	const count = createQuery(() => {
		const asked = audience;
		return {
			queryKey: queryKeys.adminMailAudience(asked),
			queryFn: () => api.mailAudience(asked),
			placeholderData: keepPreviousData
		};
	});
	const recipients = $derived(count.data?.recipients ?? null);

	// The name the campaign's log line carries. The identity service holds the
	// operator's name and address; the domain database holds neither.
	const who = createQuery(() => ({
		queryKey: queryKeys.identity,
		queryFn: () => identity()
	}));
	const createdBy = $derived(who.data?.name.trim() || who.data?.email || 'an admin');

	function failed(failure: Error, fallback: string) {
		toast('error', failure instanceof ApiFailure ? failure.message : fallback);
	}

	const testing = createMutation(() => ({
		mutationFn: () => api.testMail({ draft, created_by_label: createdBy }),
		onSuccess: () => toast('info', `A test is on its way to ${who.data?.email ?? 'your address'}.`),
		onError: (failure: Error) => failed(failure, 'The test was not sent.')
	}));

	const sending = createMutation(() => ({
		mutationFn: () => api.createMailCampaign({ draft, audience, created_by_label: createdBy }),
		onSuccess: (detail: MailCampaignDetail) => {
			queryClient.setQueryData(queryKeys.adminMailCampaign(detail.id), detail);
			void queryClient.invalidateQueries({ queryKey: queryKeys.adminMailCampaigns, exact: true });
			void goto(`/admin/mail/${detail.id}`);
		},
		onError: (failure: Error) => failed(failure, 'The email was not sent.')
	}));

	const sendRefusal = $derived(
		problem ??
			(recipients === null
				? 'Counting the sellers.'
				: recipients === 0
					? 'Nobody matches these filters.'
					: sending.isPending
						? 'Sending.'
						: null)
	);

	function send() {
		if (sendRefusal !== null || recipients === null) {
			return;
		}
		if (
			!confirm(
				`Send “${draft.subject}” to ${sellers(recipients)} now?\n\nIt starts sending at once.`
			)
		) {
			return;
		}
		sending.mutate();
	}

	/** The email's own height, so the preview scrolls with the page rather
	 *  than inside a box. The frame is same-origin and runs no script. */
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

	const steps = $derived([
		{ id: 'write', label: 'Write', done: problem === null },
		{ id: 'audience', label: 'Audience', done: (recipients ?? 0) > 0 },
		{ id: 'send', label: 'Send', done: false }
	]);
</script>

<div class="page flow-page mail-compose">
	<PageHead
		icon="mail"
		title="New email"
		description="Write it, choose who gets it, then send."
		back={{ href: '/admin/mail', label: 'Mail' }}
	/>

	<Stepper {steps} label="Email steps" />

	<div class="flow">
		<FlowStep
			n={1}
			id="write"
			title="Write"
			hint="Write the email; the preview shows it as sellers will see it."
			summary={problem ?? draft.subject}
			done={problem === null}
		>
			{#snippet aside()}
				<Explain title="Variables" label="Variables">
					<p>Each variable is swapped for the seller’s own details when the email sends.</p>
					<ul>
						{#each MAIL_VARIABLES as variable (variable.token)}
							<li><code>{variable.token}</code>: {variable.hint}</li>
						{/each}
					</ul>
					<p>Every email also ends with an unsubscribe line.</p>
				</Explain>
			{/snippet}
			<div class="mail-write">
				<div class="mail-fields">
					<Field label="Subject" id="mail-subject" required>
						<input
							id="mail-subject"
							type="text"
							required
							maxlength={SUBJECT_MAX}
							placeholder="New this term: sync to Tes"
							bind:value={subject}
						/>
					</Field>
					<Field label="Email" id="mail-body" required>
						<RichTextEditor
							bind:this={editor}
							id="mail-body"
							label="Email"
							value={body}
							onchange={(html) => (body = html)}
							sanitise={sanitiseMailHtml}
							placeholder="Hello @first_name, …"
							required
							headings
							uploadImage={(file) => api.uploadMailImage(file).then((stored) => stored.url)}
						>
							{#snippet extra()}
								<span class="mail-vars" role="group" aria-label="Variables">
									{#each MAIL_VARIABLES as variable (variable.token)}
										<button
											type="button"
											class="res-md-b rte-b"
											title={variable.hint}
											onmousedown={(e) => e.preventDefault()}
											onclick={() => editor?.insertText(variable.token)}
										>
											{variable.token}
										</button>
									{/each}
								</span>
							{/snippet}
						</RichTextEditor>
					</Field>
					<div class="mail-link">
						<Field
							label="Button address"
							id="mail-link-url"
							hint="Where @link goes. Starts with https://."
						>
							<input
								id="mail-link-url"
								type="url"
								inputmode="url"
								placeholder="https://teachouse.com/…"
								bind:value={linkUrl}
							/>
						</Field>
						<Field label="Button words" id="mail-link-label" hint="Empty says “Open Teachouse”.">
							<input
								id="mail-link-label"
								type="text"
								placeholder="Open Teachouse"
								bind:value={linkLabel}
							/>
						</Field>
					</div>
					<div class="actions">
						<Button
							icon="send"
							disabled={problem !== null || testing.isPending}
							reason={problem ?? (testing.isPending ? 'Sending the test.' : undefined)}
							onclick={() => testing.mutate()}
						>
							{testing.isPending ? 'Sending…' : 'Send a test to yourself'}
						</Button>
					</div>
				</div>

				<section class="mail-preview" aria-label="Preview">
					{#if preview.isError}
						<Banner tone="bad">
							{preview.error instanceof ApiFailure
								? preview.error.message
								: 'We could not draw the preview.'}
						</Banner>
					{/if}
					{#if preview.data}
						<p class="mail-preview-subject">Subject: <b>{preview.data.subject}</b></p>
						<iframe
							class="mail-frame"
							title="Email preview"
							srcdoc={preview.data.html}
							sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox"
							onload={fit}
						></iframe>
					{:else}
						<p class="mail-preview-empty">
							The preview shows here once the email has a subject and some words.
						</p>
					{/if}
				</section>
			</div>
		</FlowStep>

		<FlowStep
			n={2}
			id="audience"
			title="Audience"
			hint="Choose which sellers get it."
			summary={audienceSummary(audience)}
			done={(recipients ?? 0) > 0}
		>
			<fieldset class="mail-segments">
				<legend>Who gets it</legend>
				{#each SEGMENTS as choice (choice.value)}
					<label>
						<input type="radio" name="mail-segment" value={choice.value} bind:group={segment} />
						{choice.label}
					</label>
				{/each}
			</fieldset>
			<Toggle label="Leave out admins" bind:checked={excludeOperators} />
			<Toggle label="Only verified addresses" bind:checked={verifiedOnly} />
			{#if count.isError}
				<Banner tone="bad">We could not count the sellers. Try reloading the page.</Banner>
			{:else if recipients === null}
				<p class="quiet">Counting the sellers…</p>
			{:else}
				<div class="mail-count" class:none={recipients === 0} aria-live="polite">
					{countLine(recipients)}
					<Explain title="Who is counted" label="Who?">
						{#if count.data}
							<p>
								{sellers(count.data.sellers)} are in this group. Of them,
								{count.data.opted_out.toLocaleString('en-GB')} turned news off,
								{count.data.no_sign_in.toLocaleString('en-GB')} have no sign-in to email, and
								{count.data.operators_excluded.toLocaleString('en-GB')} admins are left out.
							</p>
						{/if}
						<p>Addresses that aren’t verified are skipped when it sends.</p>
					</Explain>
				</div>
			{/if}
		</FlowStep>

		<FlowStep
			n={3}
			id="send"
			title="Send"
			hint="Check it once more, then send."
			summary={recipients === null ? undefined : countLine(recipients)}
		>
			<dl class="mail-facts">
				<div>
					<dt>Subject</dt>
					<dd>{draft.subject === '' ? 'No subject yet.' : draft.subject}</dd>
				</div>
				<div>
					<dt>To</dt>
					<dd>{audienceSummary(audience)}</dd>
				</div>
				<div>
					<dt>How many</dt>
					<dd>{recipients === null ? 'Counting…' : countLine(recipients)}</dd>
				</div>
			</dl>
			<div class="actions">
				<Button
					tier="primary"
					icon="send"
					disabled={sendRefusal !== null}
					reason={sendRefusal ?? undefined}
					onclick={send}
				>
					{sending.isPending
						? 'Sending…'
						: recipients === null || recipients === 0
							? 'Send'
							: `Send to ${sellers(recipients)}`}
				</Button>
			</div>
		</FlowStep>
	</div>
</div>
