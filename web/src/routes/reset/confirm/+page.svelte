<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { resetPassword } from '$lib/auth-client';
	import { toast } from '$lib/toast';

	// better-auth's `/reset-password/:token` callback hands the browser back
	// here carrying either `?token=` or `?error=INVALID_TOKEN`, so both are read
	// (`better-auth/api/routes/password`). A token that expired between the
	// callback and this submit comes back as the same code on the response.
	const linked = $derived(page.url.searchParams.get('token'));
	const linkRefused = $derived(page.url.searchParams.get('error') !== null);

	let password = $state('');
	let confirmation = $state('');
	let busy = $state(false);
	let spent = $state(false);

	const unusable = $derived(linkRefused || linked === null || spent);

	function messageOf(error: unknown, fallback: string): string {
		if (error !== null && typeof error === 'object' && 'message' in error) {
			const message = (error as { message?: unknown }).message;
			if (typeof message === 'string' && message.length > 0) {
				return message;
			}
		}
		return fallback;
	}

	async function choose(event: SubmitEvent) {
		event.preventDefault();
		const token = linked;
		if (token === null) {
			return;
		}
		if (password !== confirmation) {
			toast('error', 'Those two passwords are not the same.');
			return;
		}
		busy = true;
		try {
			const { error } = await resetPassword(token, password);
			if (error) {
				if (error.code === 'INVALID_TOKEN') {
					spent = true;
					return;
				}
				toast('error', messageOf(error, 'The password could not be changed.'));
				return;
			}
			password = '';
			confirmation = '';
			toast('info', 'Password changed. Sign in with the new one.');
			await goto('/login');
		} finally {
			busy = false;
		}
	}
</script>

<div class="auth-card">
	{#if unusable}
		<h1>That link no longer works</h1>
		<p>A reset link expires, and each one can be spent once. Ask for a fresh one.</p>
		<div class="actions"><a class="cta" href="/reset">Send a new link</a></div>
	{:else}
		<h1>Choose a new password</h1>
		<p>This link signs off the change. Type the new password twice.</p>

		<form onsubmit={choose} class="form">
			<label class="field" for="password">
				New password
				<input
					id="password"
					name="password"
					type="password"
					required
					autocomplete="new-password"
					bind:value={password}
				/>
			</label>
			<label class="field" for="confirmation">
				Repeat it
				<input
					id="confirmation"
					name="confirmation"
					type="password"
					required
					autocomplete="new-password"
					bind:value={confirmation}
				/>
			</label>
			<button class="cta" disabled={busy}>
				{busy ? 'Saving…' : 'Change password'}
			</button>
		</form>

		<p class="auth-foot">
			Remembered it after all? <a class="link" href="/login">Sign in</a>.
		</p>
	{/if}
</div>
