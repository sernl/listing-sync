<script lang="ts">
	import { goto, invalidateAll } from '$app/navigation';
	import { api, ApiFailure } from '$lib/api';
	import { toast } from '$lib/toast';

	let token = $state('');
	let busy = $state(false);

	async function exchange(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		try {
			const who = await api.exchange(token.trim());
			toast('info', `Signed in for organisation ${who.org.slice(0, 8)}…`);
			await invalidateAll();
			goto('/');
		} catch (failure) {
			const message =
				failure instanceof ApiFailure && failure.status === 401
					? 'That token was not accepted. Mint a fresh one and paste the whole line.'
					: 'Something went wrong reaching the server.';
			toast('error', message);
		} finally {
			busy = false;
		}
	}
</script>

<div class="mx-auto max-w-md">
	<h1 class="mb-2 text-xl font-semibold">Sign in</h1>
	<p class="mb-4 text-sm text-slate-600">
		Paste the session line the operator minted for you — the whole
		<code>tam_session=…</code> line works as-is. The exchange sets a secure
		cookie; the token itself is never stored here.
	</p>
	<form onsubmit={exchange} class="flex flex-col gap-3">
		<input
			class="rounded border border-slate-300 px-3 py-2 font-mono text-sm"
			placeholder="tam_session=…"
			bind:value={token}
			autocomplete="off"
		/>
		<button
			class="rounded bg-slate-900 px-4 py-2 text-white disabled:opacity-50"
			disabled={busy || token.trim().length === 0}
		>
			{busy ? 'Signing in…' : 'Sign in'}
		</button>
	</form>
</div>
