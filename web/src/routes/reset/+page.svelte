<script lang="ts">
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import Turnstile from '$lib/Turnstile.svelte';
	import { requestPasswordReset } from '$lib/auth-client';
	import { TURNSTILE_SITE_KEY, captchaOptions, captchaPending } from '$lib/captcha';
	import { toast } from '$lib/toast';
	import '$lib/pages/account/signed-out.css';

	let email = $state('');
	let busy = $state(false);
	let sent = $state(false);
	let captchaToken = $state<string | null>(null);
	let captcha = $state<ReturnType<typeof Turnstile> | null>(null);
	const challengePending = $derived(captchaPending(TURNSTILE_SITE_KEY, captchaToken));

	function messageOf(error: unknown, fallback: string): string {
		if (error !== null && typeof error === 'object' && 'message' in error) {
			const message = (error as { message?: unknown }).message;
			if (typeof message === 'string' && message.length > 0) {
				return message;
			}
		}
		return fallback;
	}

	// The outcome says nothing about whether the address is registered, because
	// the identity service says nothing either. A refusal is still reported: a
	// spent challenge or an unreachable service is a fact about this browser,
	// not about the address, and a page that swallowed it would leave the human
	// waiting for an email nobody sent.
	async function request(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		try {
			const { error } = await requestPasswordReset(email.trim(), captchaOptions(captchaToken));
			if (error) {
				captcha?.reset();
				toast('error', messageOf(error, 'We could not send a reset link. Try again.'));
				return;
			}
			sent = true;
		} finally {
			busy = false;
		}
	}
</script>

<div class="auth-card acct-signed-out">
	{#if sent}
		<h1>Check your email</h1>
		<p>If <b>{email.trim()}</b> has an account, a reset link is on its way.</p>
		<div class="actions"><Button tier="outline" href="/login">Go to sign in</Button></div>
	{:else}
		<h1>Reset your password</h1>
		<p>Enter the email you signed up with.</p>

		<form onsubmit={request} class="form">
			<Field label="Email" id="email" required>
				<input id="email" name="email" type="email" required autocomplete="username" bind:value={email} />
			</Field>
			<Turnstile bind:this={captcha} onToken={(token) => (captchaToken = token)} />
			<Button
				tier="primary"
				type="submit"
				disabled={busy || challengePending}
				reason={busy
					? 'Sending the link.'
					: challengePending
						? 'Complete the check above first.'
						: undefined}
			>
				{busy ? 'Sending…' : 'Send the link'}
			</Button>
		</form>

		<p class="auth-foot">
			Remembered it? <a class="link" href="/login">Sign in</a>.
		</p>
	{/if}
</div>
